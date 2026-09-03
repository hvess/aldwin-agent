use clap::Parser;

/// A tool for thought.
///
/// Zero-arg binary — every runtime setting lives in `.mjolnir/` (project)
/// and `~/.mjolnir/` (global) config files, not flags.
// Exists only so clap gives us --help/--version and rejects unexpected
// arguments — see mjolnir-cli.md's Decision against a CLI-flag surface.
#[derive(Parser)]
// `version` is spelled out rather than left to clap's `CARGO_PKG_VERSION`
// default so `--version` carries the commit too: on this harness most
// builds sit somewhere after the last release tag, and the version alone
// cannot tell two of them apart. See `mjolnir_tui::version`.
#[command(name = "mjolnir", version = mjolnir_tui::VERSION_FULL, about, long_about = None)]
struct Cli;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    Cli::parse();

    match mjolnir_cli::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mjolnir: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
