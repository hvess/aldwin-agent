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
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// The message names the roots rather than only the refusal: the
    /// observed failure was a model told "outside the project root" with no
    /// way to learn what the root *was*, which silently pushed the work onto
    /// `run` — the one tool that was not checking (ADR 0007).
    #[error("path {path:?} is outside this workspace. Reachable roots: {roots}. To reach it, the developer adds its directory under `roots:` in .aldwin/permissions.yaml and runs /reload-config — say so rather than routing around this with a program")]
    PathEscapesWorkspace { path: String, roots: String },

    #[error("{path}: expected exactly one occurrence of the given text, found {count}")]
    AmbiguousMatch { path: PathBuf, count: usize },

    /// A deny is a lock (ADR 0004 §7, kept by ADR 0009), so this is not
    /// "you were not allowed" but "nothing you can do here will allow it".
    /// The message names the file to go and change, because that is the
    /// only way out.
    #[error("{program} is denied by a rule in {where_it_lives} — nothing here can override it")]
    Locked {
        program: String,
        where_it_lives: &'static str,
    },

    /// A call declared a read did not complete with the project read-only.
    /// Nothing landed — that is the property the sandbox exists to provide —
    /// and the model is told so directly: the declaration was wrong, and the
    /// call is its to make again as what it is. There is no prompt in
    /// between (ADR 0009 §3).
    #[error("{program} was declared a read but tried to write or connect; nothing changed. If it should, declare it \"write\" and run it again")]
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
    /// honoured. Not a failure the model sees: the dispatcher runs the call
    /// unconfined and tells the developer once (ADR 0009 §3). Carries
    /// `program`/`args` so a future prompt could name what it is about.
    #[error(
        "this call was declared a read, but reads cannot be enforced on this system: {source}"
    )]
    SandboxUnavailable {
        program: String,
        args: Vec<String>,
        #[source]
        source: std::io::Error,
    },

    #[error("no such program: {program}")]
    ProgramNotFound { program: String },

    /// Carries whatever the program wrote before the budget ran out —
    /// without it, a long command that failed reported only that it was
    /// long, and the 30-minute clone that prompted this left nothing at all
    /// to diagnose it with.
    #[error("command timed out after {seconds}s\n{partial}")]
    Timeout { seconds: u64, partial: String },

    /// The question was never answered — the turn was cancelled, or the
    /// session ended, while it was open.
    #[error("the question was not answered")]
    Unanswered,

    #[error("language server error: {0}")]
    Lsp(#[from] crate::lsp::LspError),

    #[error("MCP bridge error: {0}")]
    Mcp(#[from] crate::mcp::McpError),

    #[error("MCP tool {server}:{tool} returned an error: {message}")]
    McpToolError {
        server: String,
        tool: String,
        message: String,
    },
}
