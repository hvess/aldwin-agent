use clap::Parser;

/// A tool for thought.
///
/// Zero-arg binary — every runtime setting lives in `.amundsen/` (project)
/// and `~/.amundsen/` (global) config files, not flags.
// Exists only so clap gives us --help/--version and rejects unexpected
// arguments — see amundsen-cli.md's Decision against a CLI-flag surface.
#[derive(Parser)]
#[command(name = "amundsen", version, about, long_about = None)]
struct Cli;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    Cli::parse();

    match amundsen_cli::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("amundsen: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
