use thiserror::Error;

/// Why the process exits non-zero. Printed to stderr only while the TUI does
/// not hold the terminal.
///
/// Never paraphrase the failing field (aldwin-cli.md's Pitfall): pass a lower
/// crate's `Display` through, or quote the path/var/domain verbatim.
#[derive(Debug, Error)]
pub enum StartupError {
    /// A config layer failed to load, or the global directory to initialise.
    #[error(transparent)]
    Config(#[from] aldwin_config::ConfigError),

    /// `~/.aldwin` exists but lacks some of its required files, so it is
    /// neither left alone nor reinitialised.
    #[error(
        "~/.aldwin is missing required file(s): {missing:?} — refusing to start. \
         Restore the missing file(s), or remove ~/.aldwin entirely to reinitialize it."
    )]
    PartiallyPresentGlobalConfig {
        /// The required file names that are absent.
        missing: Vec<&'static str>,
    },

    /// The configured provider's client could not be built.
    #[error(transparent)]
    Llm(#[from] aldwin_llm::LlmClientInitError),

    /// The terminal could not be taken over, drawn to or restored.
    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The current working directory (the workspace root) could not be read.
    #[error("could not read the current working directory: {0}")]
    Cwd(#[source] std::io::Error),

    /// The agent or slash-command interceptor task panicked; found at
    /// session wind-down.
    #[error("the {task} stopped unexpectedly: {source}")]
    TaskFailed {
        /// Which task stopped: `"agent"` or `"interceptor"`.
        task: &'static str,
        /// The join error carrying the panic.
        #[source]
        source: tokio::task::JoinError,
    },
}

/// Why the git shim (ADR 0013) is not installed. Not fatal: the session
/// starts without it, and says so once.
#[derive(Debug, Error)]
pub enum ShimError {
    /// The shim's directory, symlink or `PATH` entry could not be made.
    #[error(transparent)]
    Install(#[from] std::io::Error),

    /// The git on `PATH` lacks `commit --trailer`; the shim would fail every
    /// commit.
    #[error("{version} is older than 2.32, which `git commit --trailer` needs")]
    GitTooOld {
        /// What `git --version` answered.
        version: String,
    },
}
