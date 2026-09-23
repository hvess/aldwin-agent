use thiserror::Error;

/// Every one of these happens before the TUI has launched (or, for `Io`,
/// after it has already cleanly torn itself down) — safe to print straight
/// to stderr. Per aldwin-cli.md's Pitfall, none of these paraphrase the
/// failing field; they all pass through a lower crate's own `Display`
/// (already written to quote the exact path/var/domain verbatim) or quote
/// it directly themselves.
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
}
