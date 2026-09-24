use thiserror::Error;

/// Why the process exits non-zero. Every one of these is printed after the
/// TUI has let go of the terminal — before it launched, or after it has
/// cleanly torn itself down — so each is safe to write straight to stderr.
/// Per aldwin-cli.md's Pitfall, none of these paraphrase the failing field;
/// they all pass through a lower crate's own `Display` (already written to
/// quote the exact path/var/domain verbatim) or quote it directly
/// themselves.
#[derive(Debug, Error)]
pub enum StartupError {
    #[error(transparent)]
    Config(#[from] aldwin_config::ConfigError),

    #[error(
        "~/.aldwin is missing required file(s): {missing:?} — refusing to start. \
         Restore the missing file(s), or remove ~/.aldwin entirely to reinitialize it."
    )]
    PartiallyPresentGlobalConfig { missing: Vec<&'static str> },

    #[error(transparent)]
    Llm(#[from] aldwin_llm::LlmClientInitError),

    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("could not read the current working directory: {0}")]
    Cwd(#[source] std::io::Error),

    /// A task of the running session — the agent, or the slash-command
    /// interceptor — panicked. Found when the session is wound down.
    #[error("the {task} stopped unexpectedly: {source}")]
    TaskFailed {
        task: &'static str,
        #[source]
        source: tokio::task::JoinError,
    },
}
