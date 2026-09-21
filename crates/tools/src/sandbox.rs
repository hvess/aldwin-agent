//! Holding a read declaration to its word (ADR 0004 §4).
//!
//! The agent declares a class for every call. Nothing verifies the
//! declaration, because verifying it would mean predicting what a command
//! does — the guess the whole model was reopened to avoid. Instead a call
//! declared `read` is executed **for real, where writing is impossible**. If
//! it finishes, it was a read; that is demonstrated rather than predicted. If
//! the kernel refuses it, nothing landed, and the developer is asked whether
//! to allow it as a write and run it again.
//!
//! On Linux the primitive is Landlock: an unprivileged LSM that restricts a
//! process, and every process it goes on to spawn, to a set of filesystem
//! rules it cannot escape or widen. The rules here are one sentence long —
//! *read anything, write nothing but the incidental paths* — plus, from ABI 4,
//! no TCP.
//!
//! **Read is allowed broadly on purpose.** A program has to read its own
//! interpreter, its shared libraries and `/etc` to run at all, so confining
//! reads to the project would confine the sandbox to programs that need
//! nothing. Keeping the model honest about this is why ADR 0004 §5 states the
//! claim as *no tool is pointed outside your workspace by us* — argument
//! containment is a separate check in `paths.rs`, and it is what bounds
//! reach.
//!
//! That sentence was false when it was written, and is worth recording as
//! such: `run` did not call `paths.rs` at all, so the containment this module
//! deferred to did not exist for the one tool that executes programs. ADR
//! 0007 made every tool go through `Workspace`, which is what makes the
//! deferral honest.
//!
//! On macOS the primitive is Seatbelt, reached through `sandbox-exec` — see
//! `macos.rs` for why that backend confines by rewriting the command rather
//! than by acting inside the forked child.
//!
//! **Two gaps, stated rather than papered over.** Landlock's network control
//! covers TCP bind and connect; UDP and unix sockets are outside it (SBPL's
//! `network*` does cover them, so this gap is Linux's alone). And a
//! denial reaches us as an ordinary failure from the child, so this module
//! reports *that a call could not complete with the tree read-only*, not
//! which path it reached for. Naming the path needs syscall interception —
//! worth building, and not needed for the guarantee, which comes from the
//! write being impossible rather than from our seeing it.

use std::path::PathBuf;

/// Whether this build, on this kernel, can hold a read declaration to its
/// word — and if not, why, in words a developer can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Reads are enforced. `abi` is the Landlock ABI the kernel offers;
    /// network control needs 4 or better.
    Enforcing { abi: i32, network: bool },
    /// Reads cannot be enforced here. A `read` grant must not be honoured
    /// silently — every call asks instead (ADR 0004 §4).
    Unavailable { reason: &'static str },
}

impl Availability {
    pub fn enforcing(&self) -> bool {
        matches!(self, Availability::Enforcing { .. })
    }
}

/// Paths a read-declared call may still write to, because a genuine read
/// cannot run without them.
///
/// This list is the one place our judgement re-enters a model built to avoid
/// it, so it is kept short, readable, and deliberately excludes the tempting
/// entry: **`.git/` is not here.** Letting a "read" write anywhere under
/// `.git/` would let `git commit` — which touches nothing else — succeed as a
/// read. Git's own `GIT_OPTIONAL_LOCKS=0` (set in [`read_only_env`]) is what
/// makes `git status` work instead, which is the case the exemption would
/// have been for.
/// Whether `resolved` — an already symlink-resolved path — is one of the
/// incidental paths, or under one.
///
/// `run`'s argument containment asks this, because the two lists have to
/// agree: a sandbox that lets a read write to `/dev/null` next to containment
/// that refuses `/dev/null` as an *argument* made `grep x file /dev/null`
/// fail under either class. Compared on resolved forms, since `/tmp` is a
/// symlink on macOS.
pub fn is_incidental(resolved: &std::path::Path, roots: &[PathBuf]) -> bool {
    incidental_writes(roots).iter().any(|p| {
        let canonical = p.canonicalize().unwrap_or_else(|_| p.clone());
        resolved.starts_with(&canonical)
    })
}

fn incidental_writes(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/dev/null"),
        PathBuf::from("/dev/zero"),
        PathBuf::from("/dev/full"),
        PathBuf::from("/dev/random"),
        PathBuf::from("/dev/urandom"),
        PathBuf::from("/dev/tty"),
        PathBuf::from("/dev/shm"),
        PathBuf::from("/tmp"),
    ];
    // Build caches. Artifacts, not source — a read that refreshes one has
    // changed nothing a developer wrote. Every root gets the same exemption:
    // a workspace whose second root had to be treated more strictly than its
    // first would just be a read that mysteriously fails over there.
    for root in roots {
        paths.push(root.join("target"));
        paths.push(root.join("node_modules/.cache"));
    }
    if let Some(tmp) = std::env::var_os("TMPDIR") {
        paths.push(PathBuf::from(tmp));
    }
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".cache"));
    }
    paths
}

/// Environment a read-declared call runs with, on top of the inherited one.
///
/// `GIT_OPTIONAL_LOCKS=0` is git's own switch for exactly this situation: it
/// stops read commands such as `status` from taking the index lock, so they
/// succeed against a read-only tree instead of needing a `.git/` write
/// exemption that would also let `git commit` through.
pub fn read_only_env() -> Vec<(&'static str, &'static str)> {
    vec![("GIT_OPTIONAL_LOCKS", "0")]
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{availability, ReadOnly};

// Compiled on every platform, used only on macOS. The backend is ordinary
// Rust — a profile string and a command rewrite, no FFI — so there is no
// reason to let it rot behind a `cfg` that this project's own machines never
// build. Its unit tests run everywhere too, and skip themselves where
// `sandbox-exec` is absent.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{availability, ReadOnly};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod elsewhere {
    use super::*;
    use std::io;

    pub fn availability() -> Availability {
        Availability::Unavailable {
            reason: "reads can only be enforced on Linux (Landlock) and macOS (Seatbelt)",
        }
    }

    /// A stand-in that refuses to be built, so a platform without enforcement
    /// cannot silently run a read-declared call unconfined. The caller's only
    /// path is to ask.
    pub struct ReadOnly(std::convert::Infallible);

    impl ReadOnly {
        pub fn build(_roots: &[PathBuf]) -> io::Result<Self> {
            Err(io::Error::new(io::ErrorKind::Unsupported, availability_reason()))
        }

        pub fn abi(&self) -> i32 {
            match self.0 {}
        }

        /// Unreachable: [`ReadOnly::build`] never succeeds on this platform.
        pub fn command_line(&self, _program: &str, _args: &[String]) -> (String, Vec<String>) {
            match self.0 {}
        }

        /// Unreachable, as above.
        pub fn install(self, _cmd: &mut tokio::process::Command) {
            match self.0 {}
        }
    }

    fn availability_reason() -> &'static str {
        match availability() {
            Availability::Unavailable { reason } => reason,
            Availability::Enforcing { .. } => unreachable!("this build has no enforcement"),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use elsewhere::{availability, ReadOnly};
