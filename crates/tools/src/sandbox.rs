//! The one sandbox every process Aldwin starts over the repository runs in
//! (ADR 0011): `run`'s shell, the language server behind `explain`, and each
//! MCP stdio server.
//!
//! The rule is one sentence: **write only inside the workspace roots and the
//! incidental paths; read anything; reach any network.** Reads are open
//! because a program has to read its interpreter, its libraries and `/etc`
//! to run at all, and because reading is not what the boundary protects.
//! The network is open because confining it is a different product — ADR
//! 0011 states that as a non-goal rather than leaving it implied.
//!
//! On Linux the primitive is Landlock (`linux.rs`): an unprivileged LSM that
//! restricts a process, and everything it goes on to spawn, to rules it
//! cannot widen. On macOS it is Seatbelt through `sandbox-exec` (`macos.rs`),
//! which confines by rewriting the command rather than acting in the forked
//! child. Anywhere else, or where the kernel offers neither, a process runs
//! unconfined — and the developer is told once, at startup, by the caller of
//! [`unavailable`]. Never silently.

use std::io;
use std::path::PathBuf;
// `std`'s, named apart from tokio's `Command`, which `command` returns.
use std::process::Command as StdCommand;

/// Paths a process may write outside the workspace, because ordinary
/// programs cannot run without them: the null and random devices, the
/// terminal, shared memory, the temp directories, the per-user cache, and
/// the package managers' shared stores — without those a build that fetches
/// a new dependency fails (the developer's call, ADR 0011 §1).
///
/// Kept short and readable on purpose — this list is the one place a
/// judgement about what programs "need" enters a rule otherwise stated as
/// "the workspace". A path that does not exist is skipped by each backend.
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
        // Each store where its tool puts it: the tool's own variable when
        // set, its documented default under `$HOME` otherwise.
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

/// Why processes cannot be confined on this system, or `None` when they
/// can. Asked once, at startup, so the developer hears it once.
pub fn unavailable() -> Option<&'static str> {
    backend::unavailable()
}

/// A command for `program args` that can write only beneath `roots` and the
/// incidental paths — or, where [`unavailable`] says this system cannot
/// confine anything, the same command unconfined.
///
/// An error means the system *can* confine and building the confinement
/// failed; the caller must not run the program then, because running it
/// unconfined would be exactly the silent weakening ADR 0011 rules out.
///
/// Building a ruleset is a handful of `open`/`stat` calls, synchronous on
/// purpose: microseconds, and a `spawn_blocking` hop would cost more than it
/// saves.
///
/// Every command carries [`AGENT`], confined or not: a commit made from
/// anything Aldwin starts is an agent's commit, and the repository's
/// pre-commit gate tells one from the developer's by that variable.
pub(crate) fn command(
    program: &str,
    args: &[String],
    roots: &[PathBuf],
) -> io::Result<tokio::process::Command> {
    std_command(program, args, roots).map(tokio::process::Command::from)
}

/// [`command`], as a `std` command: for a caller with no async runtime, such
/// as a probe Aldwin runs at startup before its runtime exists. Same
/// confinement, same `AGENT`.
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
    // Before anything else is configured: a backend that wraps the program
    // (macOS) changes what is spawned, and `Command` has no getters for stdio
    // or `pre_exec` hooks that a rebuild would have to preserve.
    let (program, argv) = sandbox.command_line(program, &args);
    let mut cmd = StdCommand::new(program);
    cmd.args(argv).env(AGENT.0, AGENT.1);
    sandbox.install(&mut cmd);
    Ok(cmd)
}

/// The variable that names the agent a process runs for, and Aldwin's value
/// for it.
pub(crate) const AGENT: (&str, &str) = ("AGENT", "aldwin");

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;

// Compiled on every platform, used only on macOS. The backend is ordinary
// Rust — a profile string and a command rewrite, no FFI — so there is no
// reason to let it rot behind a `cfg` a Linux build would never
// build. Its unit tests run everywhere.
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

    /// The pre-commit gate tells an agent's commit from the developer's by
    /// this variable, so a shell Aldwin starts must carry it.
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
