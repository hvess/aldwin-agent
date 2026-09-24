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

/// Paths a process may write outside the workspace, because ordinary
/// programs cannot run without them: the null and random devices, the
/// terminal, shared memory, the temp directories, and the per-user cache.
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
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".cache"));
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
pub(crate) fn command(
    program: &str,
    args: &[String],
    roots: &[PathBuf],
) -> io::Result<tokio::process::Command> {
    if unavailable().is_some() {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args);
        return Ok(cmd);
    }
    let sandbox = backend::Sandbox::build(roots)?;
    // Asked for before anything else is configured: a backend that wraps the
    // program (macOS) changes *what* is spawned, and `Command` has no getters
    // for stdio or `pre_exec` hooks that a rebuild would have to preserve.
    let (program, argv) = sandbox.command_line(program, args);
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(argv);
    sandbox.install(&mut cmd);
    Ok(cmd)
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;

// Compiled on every platform, used only on macOS. The backend is ordinary
// Rust — a profile string and a command rewrite, no FFI — so there is no
// reason to let it rot behind a `cfg` this project's own machines never
// build. Its unit tests run everywhere.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;
#[cfg(target_os = "macos")]
use macos as backend;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod backend {
    use std::io;
    use std::path::PathBuf;

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

        pub fn install(self, _cmd: &mut tokio::process::Command) {
            match self {}
        }
    }
}
