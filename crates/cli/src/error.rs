use thiserror::Error;

/// Every one of these happens before the TUI has launched (or, for `Tui`,
/// after it has already cleanly torn itself down) — safe to print straight
/// to stderr. Per mjolnir-cli.md's Pitfall, none of these paraphrase the
/// failing field; they all pass through a lower crate's own `Display`
/// (already written to quote the exact path/var/domain verbatim) or quote
/// it directly themselves.
#[derive(Debug, Error)]
pub enum StartupError {
    #[error(transparent)]
    Config(#[from] mjolnir_config::ConfigError),

    #[error(
        "~/.mjolnir is missing required file(s): {missing:?} — refusing to start. \
         Restore the missing file(s), or remove ~/.mjolnir entirely to reinitialize it."
    )]
    PartiallyPresentGlobalConfig { missing: Vec<&'static str> },

    #[error(transparent)]
    Llm(#[from] mjolnir_llm::LlmClientInitError),

    #[error("no provider is configured: {0}")]
    NoProvider(#[source] mjolnir_config::ConfigError),

    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("could not read the current working directory: {0}")]
    Cwd(#[source] std::io::Error),

    #[error("the first-run screen could not be drawn: {0}")]
    FirstRun(#[source] std::io::Error),

    /// Failing to persist a first-run answer is fatal rather than a warning:
    /// the session would otherwise start with an access posture the
    /// developer chose but the harness never recorded, and would ask again
    /// on the next start as though nothing had been decided.
    #[error("could not save the first-run answers: {0}")]
    FirstRunWrite(#[source] mjolnir_config::ConfigError),
}
