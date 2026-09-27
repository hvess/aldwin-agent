//! The sandbox every process Aldwin starts runs in (ADR 0011): `run`'s
//! shell, the language server, each MCP stdio server. Write only inside the
//! roots given — the workspace's, or none for an MCP server (ADR 0014) —
//! and the incidental paths; reads and the network are open.
//!
//! Linux: Landlock (`linux.rs`), inherited by every descendant. macOS:
//! Seatbelt via `sandbox-exec` (`macos.rs`), by rewriting the command.
//! Elsewhere a process runs unconfined, and the caller of [`unavailable`]
//! must tell the developer once at startup, never silently.

use std::io;
use std::path::PathBuf;
// `std`'s, named apart from tokio's `Command`, which `command` returns.
use std::process::Command as StdCommand;

/// Paths a process may write outside the workspace because ordinary programs
/// need them; package stores included so a build can fetch dependencies
/// (ADR 0011 §1). Keep it short: it is the one exception to the workspace
/// rule. Each backend skips a path that does not exist.
fn incidental_writes() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = [
        "/dev/null",
        "/dev/zero",
        "/dev/full",
        "/dev/random",
        "/dev/urandom",
        "/dev/tty",
        "/dev/shm",
        "/tmp",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    if let Some(tmp) = std::env::var_os("TMPDIR") {
        paths.push(PathBuf::from(tmp));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        // The tool's own variable when set, else its default under `$HOME`.
        let store = |var: &str, default: &str| {
            std::env::var_os(var).map_or_else(|| home.join(default), PathBuf::from)
        };
        paths.push(home.join(".cache"));
        paths.push(store("CARGO_HOME", ".cargo"));
        paths.push(store("RUSTUP_HOME", ".rustup"));
        paths.push(store("npm_config_cache", ".npm"));
        paths.push(store("GOMODCACHE", "go/pkg/mod"));
    }
    paths
}

/// Why processes cannot be confined here, or `None` when they can. Asked
/// once at startup, so the developer hears it once.
pub fn unavailable() -> Option<&'static str> {
    backend::unavailable()
}

/// A command for `program args` that can write only beneath `roots` and the
/// incidental paths; unconfined where [`unavailable`] says so.
///
/// On error the caller must not run the program: running it unconfined is
/// the silent weakening ADR 0011 rules out. Synchronous on purpose: a few
/// `open`/`stat` calls cost less than a `spawn_blocking` hop.
///
/// Every command carries [`AGENT`], confined or not: the repository's
/// pre-commit gate tells an agent's commit by it.
pub(crate) fn command(
    program: &str,
    args: &[String],
    roots: &[PathBuf],
) -> io::Result<tokio::process::Command> {
    std_command(program, args, roots).map(tokio::process::Command::from)
}

/// A sandboxed `std` command, for a caller with no async runtime; same
/// confinement and `AGENT` variable as the async form.
///
/// # Errors
///
/// When the system can confine processes and building the confinement fails.
///
/// # Examples
///
/// ```no_run
/// let cwd = std::env::current_dir()?;
/// let output = aldwin_tools::sandbox::std_command("git", &["--version"], &[cwd])?
///     .output()?;
/// # Ok::<(), std::io::Error>(())
/// ```
// `program` is text, not a path: the macOS backend writes it into the
// `sandbox-exec` argument line (`command_line`).
pub fn std_command(
    program: &str,
    args: &[impl AsRef<str>],
    roots: &[PathBuf],
) -> io::Result<StdCommand> {
    let args: Vec<String> = args.iter().map(|a| a.as_ref().to_string()).collect();
    if unavailable().is_some() {
        let mut cmd = StdCommand::new(program);
        cmd.args(args).env(AGENT.0, AGENT.1);
        return Ok(cmd);
    }
    let sandbox = backend::Sandbox::build(roots)?;
    // Before any other configuration: macOS rewrites the program, and
    // `Command` cannot be rebuilt keeping stdio or `pre_exec` hooks.
    let (program, argv) = sandbox.command_line(program, &args);
    let mut cmd = StdCommand::new(program);
    cmd.args(argv).env(AGENT.0, AGENT.1);
    sandbox.install(&mut cmd);
    Ok(cmd)
}

/// The variable naming the agent a process runs for, and Aldwin's value.
pub(crate) const AGENT: (&str, &str) = ("AGENT", "aldwin");

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;

// Compiled everywhere, used only on macOS: plain Rust, no FFI, so its unit
// tests run on every platform. Do not gate it behind `cfg`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;
#[cfg(target_os = "macos")]
use macos as backend;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod backend {
    use std::io;
    use std::path::PathBuf;
    use std::process::Command;

    pub fn unavailable() -> Option<&'static str> {
        Some("processes can only be confined on Linux (Landlock) and macOS (Seatbelt)")
    }

    /// Never built: [`unavailable`] always says why, so `command` never tries.
    pub enum Sandbox {}

    impl Sandbox {
        pub fn build(_roots: &[PathBuf]) -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no sandbox on this platform",
            ))
        }

        pub fn command_line(&self, _program: &str, _args: &[String]) -> (String, Vec<String>) {
            match *self {}
        }

        pub fn install(self, _cmd: &mut Command) {
            match self {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-commit gate identifies an agent's commit by `AGENT`.
    #[tokio::test]
    async fn every_process_aldwin_starts_names_its_agent() {
        let dir = tempfile::tempdir().unwrap();
        let args = ["-c".to_string(), "printf %s \"$AGENT\"".to_string()];
        let out = command("/bin/sh", &args, &[dir.path().to_path_buf()])
            .unwrap()
            .output()
            .await
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), AGENT.1);
    }
}
