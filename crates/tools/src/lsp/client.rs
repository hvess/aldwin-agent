use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

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

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, LspError>>>>>;

struct Inner {
    stdin:   tokio::sync::Mutex<tokio::process::ChildStdin>,
    next_id: AtomicI64,
    pending: PendingMap,
    // Servers (rust-analyzer included) answer position-based requests only
    // for documents the client has explicitly opened — see `ensure_open`.
    opened:  Mutex<HashSet<String>>,
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
            opened: Mutex::new(HashSet::new()),
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
    /// there's no matching `didClose`/`didChange` to send.
    pub async fn ensure_open(&self, uri: &str, language_id: &str, text: &str) -> Result<(), LspError> {
        if self.inner.opened.lock().expect("opened lock poisoned").contains(uri) {
            return Ok(());
        }
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": language_id, "version": 1, "text": text } }),
        )
        .await?;
        self.inner.opened.lock().expect("opened lock poisoned").insert(uri.to_string());
        Ok(())
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, LspError> {
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
    #[tokio::test]
    async fn spawns_and_initializes_a_real_language_server() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn hi() {}\n").unwrap();

        let client = LspClient::spawn("rust-analyzer", &[], dir.path()).await.expect("rust-analyzer must be on PATH for this test");
        client.shutdown().await;
    }
}
