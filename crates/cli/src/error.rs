use thiserror::Error;

/// Why the process exits non-zero. Each is printed after the TUI has let go
/// of the terminal — before it launched, or after it has cleanly torn itself
/// down — so each is safe to write straight to stderr.
/// Per aldwin-cli.md's Pitfall, none of these paraphrase the failing field;
/// they all pass through a lower crate's own `Display` (already written to
/// quote the exact path/var/domain verbatim) or quote it directly
/// themselves.
#[derive(Debug, Error)]
pub enum StartupError {
    /// A config layer failed to load or the global directory could not be
    /// initialised.
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

    /// The current working directory, which roots the workspace, could not
    /// be read.
    #[error("could not read the current working directory: {0}")]
    Cwd(#[source] std::io::Error),

    /// A task of the running session — the agent, or the slash-command
    /// interceptor — panicked. Found when the session is wound down.
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

    /// The git on `PATH` has no `commit --trailer`, so the shim would fail
    /// every commit.
    #[error("{version} is older than 2.32, which `git commit --trailer` needs")]
    GitTooOld {
        /// What `git --version` answered.
        version: String,
    },
}
