use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::BufReader;
use tokio::sync::oneshot;

use super::protocol::{read_message, write_message};
use crate::paths::Workspace;
use crate::sandbox;

#[derive(Debug, thiserror::Error)]
pub enum LspError {
    #[error("failed to spawn language server {command:?}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
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
    matches!(
        err,
        LspError::Rpc {
            code: CONTENT_MODIFIED,
            ..
        }
    )
}

/// Requests awaiting an answer. `None` once the reader has seen the server's
/// stdout end: a request registered after that would never be answered, so
/// closing the map is what turns it into `Closed` rather than a hang.
type PendingMap = Arc<Mutex<Option<HashMap<i64, oneshot::Sender<Result<Value, LspError>>>>>>;

/// What the server was last sent for one document.
///
/// The text itself, not a hash of it: a collision would mean an edit is never
/// sent and every later position resolves against stale text — silently, and
/// that is the exact failure `sync_document` exists to prevent. A source file
/// already in the model's context is not worth saving.
struct Synced {
    version: i64,
    text: String,
}

struct Inner {
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    next_id: AtomicI64,
    pending: PendingMap,
    // A `tokio::sync::Mutex`, not `std::sync::Mutex`: `sync_document` must
    // hold this lock across its notify `.await` (a check-then-insert split
    // across the await let two concurrent calls on the same URI both see
    // "not yet opened" and both send `didOpen` — a protocol violation).
    synced: tokio::sync::Mutex<HashMap<String, Synced>>,
    // Kept alive so `kill_on_drop` ends the server when the last
    // `LspClient` clone is dropped — with the session, or on a respawn.
    _child: tokio::process::Child,
}

/// A single language server's JSON-RPC connection over stdio. Cheap to
/// clone (`Arc`-backed) — spawned once per language, persisted for the
/// session (see `explain.rs`'s manager).
#[derive(Clone)]
pub struct LspClient {
    inner: Arc<Inner>,
}

impl LspClient {
    /// Spawns `command` in the sandbox over `workspace` (a language server
    /// runs build scripts and proc macros — repository code), performs the
    /// `initialize`/`initialized` handshake against the project root, and
    /// returns once the server has acknowledged it.
    pub async fn spawn(
        command: &str,
        args: &[&str],
        workspace: &Workspace,
    ) -> Result<Self, LspError> {
        let spawn_error = |source| LspError::Spawn {
            command: command.to_string(),
            source,
        };
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let mut cmd = sandbox::command(command, &args, &workspace.roots()).map_err(spawn_error)?;
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(spawn_error)?;
        let stdin = child.stdin.take().expect("stdin is piped above");
        let stdout = child.stdout.take().expect("stdout is piped above");

        let pending: PendingMap = Arc::new(Mutex::new(Some(HashMap::new())));
        spawn_reader(stdout, pending.clone());

        let inner = Arc::new(Inner {
            stdin: tokio::sync::Mutex::new(stdin),
            next_id: AtomicI64::new(1),
            pending,
            synced: tokio::sync::Mutex::new(HashMap::new()),
            _child: child,
        });
        let client = Self { inner };

        let root = workspace.project_root();
        let root_uri = file_uri(&root);
        let workspace_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace");
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

    /// Makes the server's copy of `uri` match `text`: `didOpen` the first
    /// time it is seen, a full-text `didChange` whenever the text differs
    /// from what was last sent, nothing otherwise.
    ///
    /// Servers answer position-based requests only for documents the client
    /// has opened, and once one is open they answer from *that copy*, not
    /// from disk. Opening once and never again meant every position asked
    /// about after an `edit` to the same file was resolved against the text
    /// from before it.
    ///
    /// Holds the lock across the `notify` `.await` so two concurrent calls
    /// for the same URI (e.g. `definition` and `hover` dispatched in the same
    /// step) can't both observe "not yet opened" and both send `didOpen`.
    pub async fn sync_document(
        &self,
        uri: &str,
        language_id: &str,
        text: &str,
    ) -> Result<(), LspError> {
        let mut synced = self.inner.synced.lock().await;
        let version = match synced.get(uri) {
            Some(sent) if sent.text == text => return Ok(()),
            Some(sent) => {
                let version = sent.version + 1;
                self.notify(
                    "textDocument/didChange",
                    json!({ "textDocument": { "uri": uri, "version": version }, "contentChanges": [{ "text": text }] }),
                )
                .await?;
                version
            }
            None => {
                self.notify(
                    "textDocument/didOpen",
                    json!({ "textDocument": { "uri": uri, "languageId": language_id, "version": 1, "text": text } }),
                )
                .await?;
                1
            }
        };
        synced.insert(
            uri.to_string(),
            Synced {
                version,
                text: text.to_string(),
            },
        );
        Ok(())
    }

    /// Whether the server's stdout has ended. Nothing heals on the same
    /// connection after that; the caller's recovery is a fresh `spawn`.
    pub fn is_closed(&self) -> bool {
        self.inner
            .pending
            .lock()
            .expect("pending lock poisoned")
            .is_none()
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
        match self
            .inner
            .pending
            .lock()
            .expect("pending lock poisoned")
            .as_mut()
        {
            Some(pending) => pending.insert(id, tx),
            None => return Err(LspError::Closed),
        };
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(e) = self.write(&message).await {
            if let Some(pending) = self
                .inner
                .pending
                .lock()
                .expect("pending lock poisoned")
                .as_mut()
            {
                pending.remove(&id);
            }
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
        write_message(&mut *stdin, message)
            .await
            .map_err(LspError::Io)
    }
}

fn spawn_reader(stdout: tokio::process::ChildStdout, pending: PendingMap) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(message)) = read_message(&mut reader).await {
            dispatch_incoming(message, &pending);
        }
        // Closed for good, under the same lock a new request registers
        // itself under — so none can slip in behind the drain and wait on
        // an answer that will never come.
        let abandoned = pending.lock().expect("pending lock poisoned").take();
        for (_, tx) in abandoned.into_iter().flatten() {
            let _ = tx.send(Err(LspError::Closed));
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
    let Value::Object(mut obj) = message else {
        return;
    };
    let Some(id) = obj.get("id").and_then(Value::as_i64) else {
        return;
    };
    if !obj.contains_key("result") && !obj.contains_key("error") {
        return; // a server->client request, not a response to one of ours
    }

    let Some(tx) = pending
        .lock()
        .expect("pending lock poisoned")
        .as_mut()
        .and_then(|p| p.remove(&id))
    else {
        return;
    };
    let result = if let Some(error) = obj.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Err(LspError::Rpc { code, message })
    } else {
        Ok(obj.remove("result").unwrap_or(Value::Null))
    };
    let _ = tx.send(result);
}

/// `path` as a `file://` URI. Percent-encoded: a bare `format!` produced an
/// invalid URI for any path with a space or a non-ASCII character in it, and
/// the server then answered about a document it had never been given.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_encoded_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// The path a `file://` URI names, percent-decoded. Anything else comes back
/// as it arrived.
pub fn path_from_uri(uri: &str) -> String {
    let Some(encoded) = uri.strip_prefix("file://") else {
        return uri.to_string();
    };
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The retry rule, pinned at its own level because the loop that uses
    /// it can only be exercised against a live server. Widening this is the
    /// hazard: a client that silently re-sends every failed request turns a
    /// deterministic error into a slow one and hides it from the developer
    /// for as long as the backoff runs.
    #[test]
    fn only_content_modified_is_retried() {
        assert!(is_retriable(&LspError::Rpc {
            code: CONTENT_MODIFIED,
            message: "content modified".into()
        }));

        // Every other code says something about the request itself, so
        // re-sending it unchanged can only reproduce the same answer.
        for code in [-32700, -32600, -32601, -32602, -32603, -32802, -32803, 0, 1] {
            let err = LspError::Rpc {
                code,
                message: "other".into(),
            };
            assert!(!is_retriable(&err), "code {code} must not be retried");
        }

        // Nor is anything that is not an `Rpc` answer at all: a dead server
        // or a broken pipe will not heal by asking again on the same
        // connection, and `spawn` is where that is recovered.
        assert!(!is_retriable(&LspError::Closed));
        assert!(!is_retriable(&LspError::Io(std::io::Error::other(
            "broken pipe"
        ))));
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

    const FAKE_SERVER: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fake_lsp_server.py"
    );

    async fn hover(client: &LspClient, uri: &str) -> Value {
        client
            .request(
                "textDocument/hover",
                json!({ "textDocument": { "uri": uri } }),
            )
            .await
            .unwrap()
    }

    /// The server answers from the copy it was sent, not from disk. The
    /// document was opened once and never updated, so after an `edit` every
    /// position was resolved against the text from before it.
    #[tokio::test]
    async fn a_document_that_changed_is_sent_again_and_one_that_did_not_is_not() {
        let client = LspClient::spawn("python3", &[FAKE_SERVER], &Workspace::new("/"))
            .await
            .unwrap();
        let uri = "file:///a.rs";

        client.sync_document(uri, "rust", "before").await.unwrap();
        client.sync_document(uri, "rust", "before").await.unwrap();
        assert_eq!(
            hover(&client, uri).await,
            json!({"contents": "before", "opens": 1, "changes": 0})
        );

        client.sync_document(uri, "rust", "after").await.unwrap();
        assert_eq!(
            hover(&client, uri).await,
            json!({"contents": "after", "opens": 1, "changes": 1})
        );
    }

    /// A request made after the server has gone must fail, not wait: the
    /// reader drained the pending map once and exited, so anything
    /// registered after that had nobody left to answer it.
    #[tokio::test]
    async fn a_server_that_has_exited_reads_as_closed_and_later_requests_do_not_hang() {
        let client = LspClient::spawn("python3", &[FAKE_SERVER], &Workspace::new("/"))
            .await
            .unwrap();
        assert!(!client.is_closed());

        assert!(client.request("test/die", Value::Null).await.is_err());
        assert!(
            client.is_closed(),
            "the caller needs to see this to respawn"
        );

        let later = tokio::time::timeout(
            Duration::from_secs(5),
            client.request("textDocument/hover", json!({})),
        )
        .await;
        assert!(later.expect("must not wait on a dead server").is_err());
    }

    #[test]
    fn a_path_survives_the_trip_through_a_uri() {
        let path = Path::new("/home/dev/my project/src/naïve.rs");
        let uri = file_uri(path);
        assert_eq!(uri, "file:///home/dev/my%20project/src/na%C3%AFve.rs");
        assert_eq!(path_from_uri(&uri), path.to_str().unwrap());
        assert_eq!(path_from_uri("untitled:Untitled-1"), "untitled:Untitled-1");
        assert_eq!(
            path_from_uri("file:///a%2"),
            "/a%2",
            "a truncated escape is kept, not dropped"
        );
    }

    /// Gated the way `llm/tests/live_lumo.rs` gates its live-API tests:
    /// this one needs `rust-analyzer` on `PATH`, which the build does not
    /// provide and a clean checkout on a fresh machine does not have. It
    /// failed the whole suite there, which makes the review loop's stage 2
    /// report a missing dependency as a broken workspace.
    ///
    ///     cargo test -p aldwin-tools -- --ignored
    #[tokio::test]
    #[ignore = "spawns real rust-analyzer; needs it on PATH"]
    async fn spawns_and_initializes_a_real_language_server() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn hi() {}\n").unwrap();

        LspClient::spawn("rust-analyzer", &[], &Workspace::new(dir.path()))
            .await
            .expect("rust-analyzer must be on PATH for this test");
    }
}
