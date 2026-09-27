//! The `aldwin` binary: parses no flags beyond `--help` and `--version`,
//! then hands the process to [`aldwin_cli::run`] and turns its outcome into
//! an exit code. Started as `git`, it is the git shim instead (ADR 0013).

use std::process::ExitCode;

use clap::Parser;

/// A tool for thought.
///
/// Zero-arg binary — every runtime setting lives in `.aldwin/` (project)
/// and `~/.aldwin/` (global) config files, not flags.
// Exists only so clap gives us --help/--version and rejects unexpected
// arguments — see aldwin-cli.md's Decision against a CLI-flag surface.
#[derive(Parser)]
// `version` is spelled out rather than left to clap's `CARGO_PKG_VERSION`
// default so `--version` carries the commit too: on this harness most
// builds sit somewhere after the last release tag, and the version alone
// cannot tell two of them apart. See `aldwin_tui::version`.
#[command(name = "aldwin", version = aldwin_tui::VERSION_FULL, about, long_about = None)]
struct Cli;

// Not `#[tokio::main]`: the shim is installed before the runtime exists,
// because installing it sets `PATH`, and that is only sound while this is
// the one thread.
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
    // `git_shim` outlives the runtime, so the directory goes only once
    // nothing Aldwin started can still be reaching for it.
    match runtime.block_on(aldwin_cli::run(git_shim.as_ref().err())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aldwin: {e}");
            ExitCode::FAILURE
        }
    }
}
