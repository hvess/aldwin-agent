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

#[tokio::main]
async fn main() -> std::process::ExitCode {
    Cli::parse();

    match aldwin_cli::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aldwin: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
