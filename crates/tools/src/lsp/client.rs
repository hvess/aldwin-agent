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

/// LSP's `ContentModified`: a transient error the protocol says to retry
/// quietly. rust-analyzer returns it throughout startup and reindexing.
const CONTENT_MODIFIED: i64 = -32801;

/// Attempts [`LspClient::request`] makes on `ContentModified`: doubling from
/// 100ms, ~1.5s in all, enough for a reindex without looking hung.
const RETRY_ATTEMPTS: usize = 5;
const RETRY_BACKOFF: Duration = Duration::from_millis(100);
// With one attempt the retry loop in `request` would be dead code.
const _: () = assert!(RETRY_ATTEMPTS > 1, "a single attempt is not a retry policy");

/// Whether a failed request is worth re-sending unchanged: `ContentModified`
/// only. Do not widen: every other code is about the request itself, and a
/// retry only delays the same error.
fn is_retriable(err: &LspError) -> bool {
    matches!(
        err,
        LspError::Rpc {
            code: CONTENT_MODIFIED,
            ..
        }
    )
}

/// Requests awaiting an answer. `None` once the server's stdout ends, so a
/// later request fails `Closed` instead of hanging.
type PendingMap = Arc<Mutex<Option<HashMap<i64, oneshot::Sender<Result<Value, LspError>>>>>>;

/// What the server was last sent for one document. The text, not a hash: a
/// collision would silently leave the server on stale text.
struct Synced {
    version: i64,
    text: String,
}

struct Inner {
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    next_id: AtomicI64,
    pending: PendingMap,
    // tokio's Mutex: `sync_document` holds it across `.await`, or two
    // concurrent calls could both send `didOpen` for one URI.
    synced: tokio::sync::Mutex<HashMap<String, Synced>>,
    // Held so `kill_on_drop` ends the server with the last `LspClient` clone.
    _child: tokio::process::Child,
}

/// One language server's JSON-RPC connection over stdio; clones share it.
/// `ExplainTool` keeps one per language.
#[derive(Clone)]
pub struct LspClient {
    inner: Arc<Inner>,
}

impl LspClient {
    /// Spawns `command` in the sandbox (it runs build scripts and proc
    /// macros) and completes the `initialize` handshake at the project root.
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

    /// Makes the server's copy of `uri` match `text`: `didOpen` first, a
    /// full-text `didChange` when it differs, else nothing. An open document
    /// is answered from this copy, never from disk.
    ///
    /// Holds the lock across the `.await` so concurrent calls cannot both
    /// send `didOpen`.
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

    /// Closes `uri` if it was shown, so a gone file stops answering.
    pub async fn close_document(&self, uri: &str) -> Result<(), LspError> {
        if self.inner.synced.lock().await.remove(uri).is_none() {
            return Ok(());
        }
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await
    }

    /// Whether the server's stdout has ended; recovery is a fresh `spawn`.
    pub fn is_closed(&self) -> bool {
        self.inner
            .pending
            .lock()
            .expect("pending lock poisoned")
            .is_none()
    }

    /// Sends one request and awaits its response, re-sending while the server
    /// answers `ContentModified` (see [`is_retriable`]). The retry is here,
    /// not in callers: it is protocol behaviour. Each attempt takes a fresh
    /// id; the old one is already answered.
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
        // The final attempt always returns through the `_` arm.
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

    /// Sends a notification. Writes inline, never spawned: LSP needs message
    /// order kept (`initialized` before any request).
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), LspError> {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.write(&message).await
    }

    async fn write(&self, message: &Value) -> Result<(), LspError> {
        let mut stdin = self.inner.stdin.lock().await;
        write_message(&mut *stdin, message).await?;
        Ok(())
    }
}

fn spawn_reader(stdout: tokio::process::ChildStdout, pending: PendingMap) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(message)) = read_message(&mut reader).await {
            dispatch_incoming(message, &pending);
        }
        // Closed under the lock requests register under, so none can slip in
        // after the drain and hang.
        let abandoned = pending.lock().expect("pending lock poisoned").take();
        for (_, tx) in abandoned.into_iter().flatten() {
            let _ = tx.send(Err(LspError::Closed));
        }
    });
}

/// Resolves the pending request a response answers. Server requests and
/// notifications are ignored; rust-analyzer's queries used here work without
/// answering them.
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

/// `path` as a percent-encoded `file://` URI; a space or non-ASCII byte must
/// be escaped or the server sees a different document.
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

    /// Pinned apart from the retry loop, which needs a live server.
    #[test]
    fn only_content_modified_is_retried() {
        assert!(is_retriable(&LspError::Rpc {
            code: CONTENT_MODIFIED,
            message: "content modified".into()
        }));

        for code in [-32700, -32600, -32601, -32602, -32603, -32802, -32803, 0, 1] {
            let err = LspError::Rpc {
                code,
                message: "other".into(),
            };
            assert!(!is_retriable(&err), "code {code} must not be retried");
        }

        // A dead connection is recovered by `spawn`, not by retrying.
        assert!(!is_retriable(&LspError::Closed));
        assert!(!is_retriable(&LspError::Io(std::io::Error::other(
            "broken pipe"
        ))));
    }

    /// Too short misses a reindex; too long looks hung.
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

    /// Regression: a document opened once was never updated after an edit.
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

    /// Regression: a request registered after the reader's drain hung.
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

    /// Ignored: needs `rust-analyzer` on `PATH`, which a fresh machine lacks,
    /// and would fail the review loop's stage 3.
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
