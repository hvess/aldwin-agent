use std::path::PathBuf;

use thiserror::Error;

/// Structured tool failure. Per aldwin-tools.md: "errors are structured
/// and fed back to the model; transport errors do not retry here" — every
/// variant's `Display` becomes `ToolResult.content` with `is_error: true`,
/// text the model is meant to read and adapt to.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("no such tool: {name}")]
    UnknownTool { name: String },

    #[error("a tool named {name:?} is already registered")]
    DuplicateTool { name: String },

    #[error("invalid input for {tool}: {message}")]
    InvalidInput { tool: String, message: String },

    #[error("{path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },

    /// The message names the roots rather than only the refusal: the
    /// observed failure was a model told "outside the project root" with no
    /// way to learn what the root *was*, which silently pushed the work onto
    /// `run` — the one tool that was not checking (ADR 0007).
    #[error("path {path:?} is outside this workspace. Reachable roots: {roots}. To reach it, the developer adds its directory under `roots:` in .aldwin/permissions.yaml and runs /reload-config — say so rather than routing around this with a shell")]
    PathEscapesWorkspace { path: String, roots: String },

    #[error("{path}: expected exactly one occurrence of the given text, found {count}")]
    AmbiguousMatch { path: PathBuf, count: usize },

    #[error("{path}: file changed on disk while the edit was awaiting approval; re-read it and retry")]
    ConcurrentModification { path: PathBuf },

    #[error("denied")]
    Denied,

    /// A deny is a lock (ADR 0004 §7), so this is not "you were not allowed"
    /// but "nothing you can answer here will allow it". The message names the
    /// file to go and change, because that is the only way out.
    #[error("{program} is denied by a rule in {where_it_lives} — no answer here can override it")]
    Locked { program: String, where_it_lives: &'static str },

    /// Not really an error: a call declared a read did not complete with the
    /// project read-only. Nothing landed. The dispatcher turns this into the
    /// second prompt of ADR 0004 §4 rather than reporting it to the model.
    #[error("{program} was declared a read and could not complete with the project read-only")]
    ReadRefused { program: String, args: Vec<String> },

    /// **Not a tool failure.** The tool worked; the program it ran exited
    /// non-zero. It travels as a `ToolError` because that is this crate's
    /// only route to `is_error: true`, and the model has to be told that the
    /// command did not succeed — `Ok` said the opposite, so a failed call
    /// arrived flagged as a good one and the model's next move was a guess.
    ///
    /// `Display` is the whole rendered output, stdout included, so nothing
    /// is lost by routing it through the error arm.
    ///
    /// Note a non-zero exit is not always a fault: `grep` exits 1 when it
    /// matched nothing, and `diff` exits 1 when files differ. This reports
    /// what the exit code *was* rather than guessing which programs mean
    /// failure by it; `run`'s own description tells the model to read the
    /// code rather than assume something broke.
    #[error("{output}")]
    CommandFailed { output: String },

    /// Reads cannot be enforced here, so a read declaration cannot be
    /// honoured — the call is not run unconfined.
    ///
    /// **This is a question, not a failure**, and the dispatcher turns it
    /// into one (ADR 0004 §4: "where a `read` grant cannot be honoured,
    /// every call asks"). It reached the model as a flat error until ADR
    /// 0007, which on a platform with no enforcement primitive meant every
    /// read-declared call failed — so the model stopped declaring reads
    /// after two attempts and spent the next 69 calls declaring `ls` and
    /// `grep` as writes. Carries `program`/`args` so the prompt can name
    /// what it is asking about.
    #[error("this call was declared a read, but reads cannot be enforced on this system: {source}")]
    SandboxUnavailable { program: String, args: Vec<String>, #[source] source: std::io::Error },

    #[error("no such program: {program}")]
    ProgramNotFound { program: String },

    /// Carries whatever the program wrote before the budget ran out —
    /// without it, a long command that failed reported only that it was
    /// long, and the 30-minute clone that prompted this left nothing at all
    /// to diagnose it with.
    #[error("command timed out after {seconds}s\n{partial}")]
    Timeout { seconds: u64, partial: String },

    #[error("permission engine error: {0}")]
    Permission(#[from] aldwin_permissions::PermissionError),

    #[error("malformed prompt response from developer")]
    MalformedPromptResponse,

    #[error("language server error: {0}")]
    Lsp(#[from] crate::lsp::LspError),

    #[error("MCP bridge error: {0}")]
    Mcp(#[from] crate::mcp::McpError),

    #[error("MCP tool {server}:{tool} returned an error: {message}")]
    McpToolError { server: String, tool: String, message: String },
}
