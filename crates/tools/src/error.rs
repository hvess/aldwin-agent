use std::path::PathBuf;

use thiserror::Error;

/// A tool failure. Each variant's `Display` becomes `ToolResult.content` with
/// `is_error: true`, written for the model to read; nothing retries here
/// (aldwin-tools.md).
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

    /// A path outside the workspace (ADR 0007). The message must name the
    /// roots and the fix, or the model routes around the refusal.
    #[error("path {path:?} is outside this workspace. Reachable roots: {roots}. To reach it, its directory goes under `roots:` in .aldwin/permissions.yaml — ask the developer rather than routing around it, and add it with `edit` only if they say so")]
    PathEscapesWorkspace {
        /// The path as the model gave it.
        path: String,
        /// Every workspace root, comma-separated, as the message shows them.
        roots: String,
    },

    /// An edit's text to replace did not occur exactly once in the file. The
    /// message must say how to make the next call succeed.
    #[error("{path}: {}", ambiguous_match_fix(.lines))]
    AmbiguousMatch {
        /// The file being edited.
        path: PathBuf,
        /// The 1-based line each occurrence starts on: none, or more than one.
        lines: Vec<usize>,
    },

    /// The command ran and exited non-zero; not a tool failure. It is a
    /// `ToolError` because that is the only route to `is_error: true`.
    /// `Display` is the whole rendered output, stdout included.
    ///
    /// A non-zero exit is not always a fault (`grep`, `diff` exit 1); this
    /// reports the code and never guesses, and `run`'s description tells the
    /// model to read it.
    #[error("{output}")]
    CommandFailed {
        /// The command's rendered output and exit code.
        output: String,
    },

    /// This system can confine a process, building the confinement failed,
    /// and nothing ran. Never fall back to unconfined here (ADR 0011).
    #[error("the sandbox could not be built, so nothing ran: {source}")]
    Sandbox {
        /// Why the confinement could not be built.
        #[source]
        source: std::io::Error,
    },

    /// The command ran out of time; carries what it wrote before, for
    /// diagnosis.
    #[error("command timed out after {seconds}s\n{partial}")]
    Timeout {
        /// The time budget that ran out, in seconds.
        seconds: u64,
        /// What the command wrote before it was stopped.
        partial: String,
    },

    /// A settings file could not be read again by `reload`.
    #[error("these settings files could not be read, so they keep their previous values and the workspace roots were left as they were: {detail}")]
    Settings {
        /// Which file could not be read and why, each named.
        detail: String,
    },

    /// The question was open when the turn was cancelled or the session ended.
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

/// What an edit's `before` did wrong, and the fix for the next call.
fn ambiguous_match_fix(lines: &[usize]) -> String {
    match lines {
        [] => "the text to replace does not occur in the file. Read the file again and copy \
               `before` from what it holds now, whitespace and indentation included; a file \
               with staged edits reads back with them applied"
            .to_string(),
        _ => {
            // The first few say where to look; every one would crowd the
            // conversation when `before` is a line like `}`.
            const NAMED: usize = 10;
            let at: Vec<String> = lines.iter().take(NAMED).map(usize::to_string).collect();
            let more = match lines.len().saturating_sub(NAMED) {
                0 => String::new(),
                rest => format!(" and {rest} more"),
            };
            format!(
                "the text to replace occurs {} times, starting on lines {}{more}. Add the \
                 lines around the one you mean to `before` so it occurs once",
                lines.len(),
                at.join(", ")
            )
        }
    }
}
