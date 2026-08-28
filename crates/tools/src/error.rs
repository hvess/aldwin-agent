use std::path::PathBuf;

use thiserror::Error;

/// Structured tool failure. Per amundsen-tools.md: "errors are structured
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

    #[error("{path}: expected exactly one occurrence of the given text, found {count}")]
    AmbiguousMatch { path: PathBuf, count: usize },

    #[error("denied by permission policy")]
    Denied,

    #[error("command timed out after {secs}s")]
    Timeout { secs: u64 },

    #[error("permission engine error: {0}")]
    Permission(#[from] amundsen_permissions::PermissionError),

    #[error("malformed prompt response from developer")]
    MalformedPromptResponse,

    #[error("language server error: {0}")]
    Lsp(#[from] crate::lsp::LspError),
}
