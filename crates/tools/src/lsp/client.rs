use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::BufReader;
use tokio::sync::oneshot;

use super::protocol::{read_message, write_message};

#[derive(Debug, thiserror::Error)]
pub enum LspError {
    #[error("failed to spawn language server {command:?}: {source}")]
    Spawn { command: String, #[source] source: std::io::Error },
    #[error("language server exited or its stdio closed")]
    Closed,
    #[error("language server returned error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("i/o error talking to language server: {0}")]
    Io(#[from] std::io::Error),
}

/// LSP's `ContentModified`. The server discarded a computation because its
/// view of the content changed underneath it: the answer is gone, but the
/// request was well-formed and the state that invalidated it is transient.
/// The protocol's intent is that a client quietly asks again rather than
/// reporting a failure — rust-analyzer returns this throughout startup and
/// reindexing, which is exactly when a developer asks the first question of
/// a session.
const CONTENT_MODIFIED: i64 = -32801;

/// How hard [`LspClient::request`] tries again after a `ContentModified`.
/// Doubling from 100ms gives five attempts inside ~1.5s — long enough to
/// ride out the churn of a reindex, short enough that a developer waiting
/// on an answer does not conclude the tool has hung. A server that is
/// *persistently* reindexing will still surface the error, which is
/// correct: at that point it is information, not noise.
const RETRY_ATTEMPTS: usize = 5;
const RETRY_BACKOFF: Duration = Duration::from_millis(100);
// `request`'s loop reads `attempt < RETRY_ATTEMPTS` to decide whether a
// retry is left, so a budget of one would make the whole policy dead code
// while still looking like one on the page.
const _: () = assert!(RETRY_ATTEMPTS > 1, "a single attempt is not a retry policy");

/// Whether a failed request is worth re-sending unchanged.
///
/// Deliberately the narrowest possible rule — `ContentModified` and nothing
/// else. Every other `Rpc` code reports something about the request itself
/// (a bad position, an unsupported method, a malformed param), and re-sending
/// an identical request can only produce an identical error while hiding it
/// behind a delay. Kept as its own function so the policy is testable, and
/// so widening it is a deliberate edit to a documented rule rather than a
/// tweak to a match arm.
fn is_retriable(err: &LspError) -> bool {
    matches!(err, LspError::Rpc { code: CONTENT_MODIFIED, .. })
}

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, LspError>>>>>;

struct Inner {
    stdin:   tokio::sync::Mutex<tokio::process::ChildStdin>,
    next_id: AtomicI64,
    pending: PendingMap,
    // Servers (rust-analyzer included) answer position-based requests only
    // for documents the client has explicitly opened — see `ensure_open`.
    // A `tokio::sync::Mutex`, not `std::sync::Mutex`: `ensure_open` must hold
    // this lock across the `didOpen` notify `.await` (a check-then-insert
    // split across the await let two concurrent calls on the same URI both
    // see "not yet opened" and both send `didOpen` — a protocol violation;
    // see mjolnir-tools.md's Progress note).
    opened:  tokio::sync::Mutex<HashSet<String>>,
    // Kept alive so `kill_on_drop` fires when the last `LspClient` clone is
    // dropped — the safety net under the graceful `shutdown()` handshake.
    _child:  std::sync::Mutex<tokio::process::Child>,
}

/// A single language server's JSON-RPC connection over stdio. Cheap to
/// clone (`Arc`-backed) — spawned once per language, persisted for the
/// session (see `explain.rs`'s manager).
#[derive(Clone)]
pub struct LspClient {
    inner: Arc<Inner>,
}

impl LspClient {
    /// Spawns `command`, performs the `initialize`/`initialized` handshake
    /// against `root`, and returns once the server has acknowledged it.
    pub async fn spawn(command: &str, args: &[&str], root: &Path) -> Result<Self, LspError> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|source| LspError::Spawn { command: command.to_string(), source })?;
        let stdin = child.stdin.take().expect("stdin is piped above");
        let stdout = child.stdout.take().expect("stdout is piped above");

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        spawn_reader(stdout, pending.clone());

        let inner = Arc::new(Inner {
            stdin: tokio::sync::Mutex::new(stdin),
            next_id: AtomicI64::new(1),
            pending,
            opened: tokio::sync::Mutex::new(HashSet::new()),
            _child: std::sync::Mutex::new(child),
        });
        let client = Self { inner };

        let root_uri = format!("file://{}", root.display());
        let workspace_name = root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace");
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {},
            "workspaceFolders": [{"uri": root_uri, "name": workspace_name}],
        });
        client.request("initialize", init_params).await?;
        client.notify("initialized", json!({})).await?;
        Ok(client)
    }

    /// Sends `textDocument/didOpen` for `uri` the first time it's seen —
    /// servers answer position-based requests (definition, references,
    /// hover, implementation) only for documents the client has opened,
    /// even when reading straight off disk otherwise. Idempotent per URI
    /// for this client's lifetime; V0 never edits through this path, so
    /// there's no matching `didClose`/`didChange` to send. Holds `opened`'s
    /// lock across the `notify` `.await` so two concurrent calls for the
    /// same URI (e.g. `definition` and `hover` dispatched in the same step)
    /// can't both observe "not yet opened" and both send `didOpen`.
    pub async fn ensure_open(&self, uri: &str, language_id: &str, text: &str) -> Result<(), LspError> {
        let mut opened = self.inner.opened.lock().await;
        if opened.contains(uri) {
            return Ok(());
        }
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": language_id, "version": 1, "text": text } }),
        )
        .await?;
        opened.insert(uri.to_string());
        Ok(())
    }

    /// Sends one request and waits for its response, re-sending it while the
    /// server answers `ContentModified` (see [`is_retriable`]).
    ///
    /// The retry lives here rather than in `explain.rs` because it is a
    /// property of the protocol, not of any one caller: `ContentModified` is
    /// the server telling the *client* to ask again. Before this, the error
    /// travelled all the way out to the developer as a tool failure, so
    /// asking a question while rust-analyzer was still indexing produced an
    /// error that went away on its own if you asked twice.
    ///
    /// Each attempt takes a fresh request id — a retry is a new request, not
    /// a re-await of the abandoned one, whose id the server has already
    /// answered and will never answer again.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, LspError> {
        let mut backoff = RETRY_BACKOFF;
        for attempt in 1..=RETRY_ATTEMPTS {
            let result = self.request_once(method, &params).await;
            match &result {
                Err(e) if is_retriable(e) && attempt < RETRY_ATTEMPTS => {}
                _ => return result,
            }
            tokio::time::sleep(backoff).await;
            backoff *= 2;
        }
        // `RETRY_ATTEMPTS` is a non-zero constant, so the loop either
        // returned or slept its way to the final attempt, which returns
        // unconditionally through the `_` arm above.
        unreachable!("the final attempt returns rather than retrying")
    }

    async fn request_once(&self, method: &str, params: &Value) -> Result<Value, LspError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().expect("pending lock poisoned").insert(id, tx);
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(e) = self.write(&message).await {
            self.inner.pending.lock().expect("pending lock poisoned").remove(&id);
            return Err(e);
        }
        rx.await.unwrap_or(Err(LspError::Closed))
    }

    /// Must actually write inline rather than fire-and-forget: LSP requires
    /// message order to be preserved (in particular, `initialized` must
    /// reach the server before any subsequent request) — a spawned,
    /// unawaited write races the stdin lock against whatever the caller
    /// sends next and can arrive out of order.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.write(&message).await
    }

    async fn write(&self, message: &Value) -> Result<(), LspError> {
        let mut stdin = self.inner.stdin.lock().await;
        write_message(&mut *stdin, message).await.map_err(LspError::Io)
    }

    /// Graceful LSP shutdown: `shutdown` request, then `exit` notification.
    /// `kill_on_drop` on the child process is the fallback if this is never
    /// called (or the server doesn't respond) — see mjolnir-tools.md's "LSP
    /// servers ... shut down at process exit."
    pub async fn shutdown(&self) {
        let _ = self.request("shutdown", Value::Null).await;
        let _ = self.notify("exit", Value::Null).await;
    }
}

fn spawn_reader(stdout: tokio::process::ChildStdout, pending: PendingMap) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        loop {
            match read_message(&mut reader).await {
                Ok(Some(message)) => dispatch_incoming(message, &pending),
                _ => {
                    for (_, tx) in pending.lock().expect("pending lock poisoned").drain() {
                        let _ = tx.send(Err(LspError::Closed));
                    }
                    return;
                }
            }
        }
    });
}

/// Routes one incoming JSON-RPC message: a response resolves the matching
/// pending request; anything else (server->client requests, notifications
/// like `window/logMessage` or progress) is silently ignored — V0 doesn't
/// answer server-initiated requests, matching how rust-analyzer's basic
/// definition/references/hover/implementation/workspace-symbol flow works
/// without a client that does.
fn dispatch_incoming(message: Value, pending: &PendingMap) {
    let Some(obj) = message.as_object() else { return };
    let Some(id) = obj.get("id").and_then(Value::as_i64) else { return };
    if !obj.contains_key("result") && !obj.contains_key("error") {
        return; // a server->client request, not a response to one of ours
    }

    let Some(tx) = pending.lock().expect("pending lock poisoned").remove(&id) else { return };
    let result = if let Some(error) = obj.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = error.get("message").and_then(Value::as_str).unwrap_or("").to_string();
        Err(LspError::Rpc { code, message })
    } else {
        Ok(obj.get("result").cloned().unwrap_or(Value::Null))
    };
    let _ = tx.send(result);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawns real rust-analyzer (confirmed present in this environment) and
    /// checks the initialize/shutdown handshake completes — this exercises
    /// the actual subprocess + stdio wiring, not just the framing logic
    /// already covered in `protocol.rs`. Doesn't wait on indexing (that's
    /// covered separately, and ignored by default — see `explain.rs`).
    /// The retry rule, pinned at its own level because the loop that uses
    /// it can only be exercised against a live server. Widening this is the
    /// hazard: a client that silently re-sends every failed request turns a
    /// deterministic error into a slow one and hides it from the developer
    /// for as long as the backoff runs.
    #[test]
    fn only_content_modified_is_retried() {
        assert!(is_retriable(&LspError::Rpc { code: CONTENT_MODIFIED, message: "content modified".into() }));

        // Every other code says something about the request itself, so
        // re-sending it unchanged can only reproduce the same answer.
        for code in [-32700, -32600, -32601, -32602, -32603, -32802, -32803, 0, 1] {
            let err = LspError::Rpc { code, message: "other".into() };
            assert!(!is_retriable(&err), "code {code} must not be retried");
        }

        // Nor is anything that is not an `Rpc` answer at all: a dead server
        // or a broken pipe will not heal by asking again on the same
        // connection, and `spawn` is where that is recovered.
        assert!(!is_retriable(&LspError::Closed));
        assert!(!is_retriable(&LspError::Io(std::io::Error::other("broken pipe"))));
    }

    /// Five attempts at 100ms doubling is ~1.5s of waiting in the worst
    /// case. Pinned because both halves matter and pull opposite ways: too
    /// short and the retry does not outlast a reindex, too long and a
    /// developer waiting on an answer concludes the tool has hung.
    #[test]
    fn the_retry_budget_stays_inside_a_second_and_a_half() {
        let mut total = Duration::ZERO;
        let mut backoff = RETRY_BACKOFF;
        for _ in 1..RETRY_ATTEMPTS {
            total += backoff;
            backoff *= 2;
        }
        assert_eq!(total, Duration::from_millis(1500));
    }

    /// Gated the way `llm/tests/live_lumo.rs` gates its live-API tests:
    /// this one needs `rust-analyzer` on `PATH`, which the build does not
    /// provide and a clean checkout on a fresh machine does not have. It
    /// failed the whole suite there, which makes the review loop's stage 2
    /// report a missing dependency as a broken workspace.
    ///
    ///     cargo test -p mjolnir-tools -- --ignored
    #[tokio::test]
    #[ignore = "spawns real rust-analyzer; needs it on PATH"]
    async fn spawns_and_initializes_a_real_language_server() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn hi() {}\n").unwrap();

        let client = LspClient::spawn("rust-analyzer", &[], dir.path()).await.expect("rust-analyzer must be on PATH for this test");
        client.shutdown().await;
    }
}
