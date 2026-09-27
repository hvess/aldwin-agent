use std::path::PathBuf;

use thiserror::Error;

/// Structured tool failure. Per aldwin-tools.md: "errors are structured
/// and fed back to the model; transport errors do not retry here" — every
/// variant's `Display` becomes `ToolResult.content` with `is_error: true`,
/// text the model is meant to read and adapt to.
#[derive(Debug, Error)]
pub enum ToolError {
    /// The model called a tool that is not registered.
    #[error("no such tool: {name}")]
    UnknownTool {
        /// The name the model used.
        name: String,
    },

    /// A second tool was registered under a name already taken.
    #[error("a tool named {name:?} is already registered")]
    DuplicateTool {
        /// The name both tools claim.
        name: String,
    },

    /// The call's arguments did not fit the tool's input schema.
    #[error("invalid input for {tool}: {message}")]
    InvalidInput {
        /// The tool that was called.
        tool: String,
        /// What was wrong with the arguments, for the model to correct.
        message: String,
    },

    /// Reading or writing a file failed.
    #[error("{path}: {source}")]
    Io {
        /// The file the operation was on.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },

    /// The message names the roots rather than only the refusal: the
    /// observed failure was a model told "outside the project root" with no
    /// way to learn what the root *was*, which silently pushed the work onto
    /// `run` — the one tool that was not checking (ADR 0007).
    #[error("path {path:?} is outside this workspace. Reachable roots: {roots}. To reach it, the developer adds its directory under `roots:` in .aldwin/permissions.yaml and runs /reload-config — say so rather than routing around it")]
    PathEscapesWorkspace {
        /// The path as the model gave it.
        path: String,
        /// Every workspace root, comma-separated, as the message shows them.
        roots: String,
    },

    /// An edit's text to replace did not occur exactly once in the file, so
    /// which occurrence was meant is unknown.
    #[error("{path}: expected exactly one occurrence of the given text, found {count}")]
    AmbiguousMatch {
        /// The file being edited.
        path: PathBuf,
        /// How many times the text occurred: zero or more than one.
        count: usize,
    },

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
    CommandFailed {
        /// The command's rendered output and exit code.
        output: String,
    },

    /// This system can confine a process and building the confinement
    /// failed, so nothing ran. Running it unconfined instead would be the
    /// silent weakening ADR 0011 rules out.
    #[error("the sandbox could not be built, so nothing ran: {source}")]
    Sandbox {
        /// Why the confinement could not be built.
        #[source]
        source: std::io::Error,
    },

    /// Carries whatever the command wrote before the budget ran out —
    /// without it, a long command that failed reported only that it was
    /// long, and the 30-minute clone that prompted this left nothing at all
    /// to diagnose it with.
    #[error("command timed out after {seconds}s\n{partial}")]
    Timeout {
        /// The time budget that ran out, in seconds.
        seconds: u64,
        /// What the command wrote before it was stopped.
        partial: String,
    },

    /// The question was never answered — the turn was cancelled, or the
    /// session ended, while it was open.
    #[error("the question was not answered")]
    Unanswered,

    /// The language server behind `explain` failed.
    #[error("language server error: {0}")]
    Lsp(#[from] crate::lsp::LspError),

    /// An MCP server could not be reached or spoke out of protocol.
    #[error("MCP bridge error: {0}")]
    Mcp(#[from] crate::mcp::McpError),

    /// An MCP server was reached and its tool reported that the call failed.
    #[error("MCP tool {server}:{tool} returned an error: {message}")]
    McpToolError {
        /// The server's name in `mcp.yaml`.
        server: String,
        /// The tool's name on that server.
        tool: String,
        /// The text the tool returned with its error.
        message: String,
    },
}
