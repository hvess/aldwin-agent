//! The `aldwin` binary: `--help` and `--version` only, then
//! [`aldwin_cli::run`]. Started as `git`, it is the git shim (ADR 0013).

use std::process::ExitCode;

use clap::Parser;

/// A tool for thought.
///
/// Zero-arg binary — every runtime setting lives in `.aldwin/` (project)
/// and `~/.aldwin/` (global) config files, not flags.
// Exists for --help/--version and to reject arguments (aldwin-cli.md's
// Decision against a CLI-flag surface).
#[derive(Parser)]
// Not clap's `CARGO_PKG_VERSION` default: `--version` must carry the
// commit, since most builds sit after the last release tag. See
// `aldwin_tui::version`.
#[command(name = "aldwin", version = aldwin_tui::VERSION_FULL, about, long_about = None)]
struct Cli;

// Not `#[tokio::main]`: installing the shim sets `PATH`, which is only
// sound while this is the one thread, so it precedes the runtime.
fn main() -> ExitCode {
    // Before clap: git's arguments are not Aldwin's.
    #[cfg(unix)]
    if let Some(status) = aldwin_cli::git_shim::intercept() {
        return status;
    }

    Cli::parse();

    #[cfg(unix)]
    let git_shim = aldwin_cli::git_shim::install();
    #[cfg(not(unix))]
    let git_shim: Result<(), aldwin_cli::ShimError> = Ok(());

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("aldwin: the async runtime could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    // `git_shim` must outlive the runtime: its directory is removed only
    // once nothing Aldwin started can still reach for it.
    match runtime.block_on(aldwin_cli::run(git_shim.as_ref().err())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aldwin: {e}");
            ExitCode::FAILURE
        }
    }
}
