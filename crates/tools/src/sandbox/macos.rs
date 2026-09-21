//! Seatbelt, spoken through `sandbox-exec`.
//!
//! The same guarantee as the Linux backend — *read anything, write nothing
//! but the incidental paths, reach no network* — reached by a different
//! route, and the difference is deliberate.
//!
//! Landlock is engaged **inside the forked child**, which is safe because
//! engaging it is two syscalls with no allocation. macOS's equivalent
//! primitive, `sandbox_init_with_parameters`, is not: it compiles an SBPL
//! profile, which allocates. Calling it between `fork` and `execve` in a
//! threaded process is how you get a child that deadlocks in the allocator —
//! the exact hazard `linux.rs` restructured itself to avoid. So this backend
//! does not run anything in the child at all. It rewrites the command to run
//! under `/usr/bin/sandbox-exec`, which applies the profile to itself and
//! then `exec`s the real program, inheriting the confinement.
//!
//! **`sandbox-exec` is deprecated and has been since 10.8.** It is also still
//! shipped on every macOS, still the only route to this primitive that does
//! not require entitlements, and the failure mode if it ever goes away is
//! benign: `availability` stops reporting `Enforcing`, and a read-declared
//! call becomes a question for the developer instead (ADR 0004 §4) rather
//! than running unconfined.
//!
//! **One gap this backend does not share with Linux.** SBPL's `network*`
//! covers more than Landlock's TCP-only control — UDP and unix sockets are
//! inside it here — so on this platform the network half of the read
//! guarantee is the stronger of the two.

use std::io;
use std::path::{Path, PathBuf};

use super::{incidental_writes, Availability};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

pub fn availability() -> Availability {
    if Path::new(SANDBOX_EXEC).exists() {
        // Seatbelt's network control is not ABI-versioned the way Landlock's
        // is; there is one profile language and it either applies or does
        // not. `abi: 0` reads as "not applicable" rather than a version.
        Availability::Enforcing { abi: 0, network: true }
    } else {
        Availability::Unavailable { reason: "reads need /usr/bin/sandbox-exec, which is not present on this system" }
    }
}

/// A compiled-in-advance SBPL profile: everything allowed, then writes and
/// network denied, then the incidental writes allowed back.
///
/// Rule order matters and is the opposite of what a firewall reader expects:
/// **Seatbelt is last-match-wins**, so the exemptions have to come after the
/// blanket denial, not before it.
pub struct ReadOnly {
    profile: String,
}

impl ReadOnly {
    pub fn build(roots: &[PathBuf]) -> io::Result<Self> {
        if !Path::new(SANDBOX_EXEC).exists() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "reads need /usr/bin/sandbox-exec, which is not present on this system",
            ));
        }

        let mut profile = String::from(
            "(version 1)\n\
             (allow default)\n\
             (deny file-write*)\n\
             (deny network*)\n",
        );

        // Read is allowed broadly on purpose — a program has to read its own
        // interpreter, its libraries and /etc to run at all. See the shared
        // module docs; the confinement is argument containment, not this.
        for path in incidental_writes(roots) {
            // Seatbelt matches the *resolved* path of the file being written,
            // and on macOS the incidental paths are mostly symlinks: `/tmp`
            // is `/private/tmp`, and `$TMPDIR` lives under `/var`, which is
            // `/private/var`. A rule for the path as written never matches,
            // so every temp-file write under a read declaration would be
            // denied. Both forms are emitted; the redundant one is harmless.
            //
            // A path that does not exist is not an error: a project with no
            // `target/` simply has nothing to exempt. An unquotable path is
            // skipped rather than allowed to truncate the profile at it.
            let mut forms = vec![path.clone()];
            if let Ok(canonical) = path.canonicalize() {
                if canonical != path {
                    forms.push(canonical);
                }
            }
            for form in forms {
                if let Some(literal) = sbpl_string(&form) {
                    profile.push_str(&format!("(allow file-write* (subpath {literal}))\n"));
                }
            }
        }

        Ok(Self { profile })
    }

    pub fn abi(&self) -> i32 {
        0
    }

    /// The command line to actually spawn: the real program, run under
    /// `sandbox-exec` with this profile.
    ///
    /// Returning a command line rather than mutating a built `Command` is
    /// deliberate. `std::process::Command` exposes no getter for its stdio
    /// or its `pre_exec` hooks, so a backend that rebuilt the command would
    /// silently drop the pipes and the `setsid` the caller had already set —
    /// and `run` would then panic taking a stdout that was no longer piped.
    /// Wrapping the argv happens before any of that is configured, so there
    /// is nothing to preserve.
    ///
    /// `--` is what keeps a program whose own first argument starts with `-`
    /// from being read as a flag to `sandbox-exec`.
    pub fn command_line(&self, program: &str, args: &[String]) -> (String, Vec<String>) {
        let mut argv = vec!["-p".to_string(), self.profile.clone(), "--".to_string(), program.to_string()];
        argv.extend(args.iter().cloned());
        (SANDBOX_EXEC.to_string(), argv)
    }

    /// Nothing to install in the child: `sandbox-exec` confines itself and
    /// then `exec`s the real program, which inherits it.
    pub fn install(self, _cmd: &mut tokio::process::Command) {}
}

/// Quotes a path as an SBPL string literal, or `None` if it cannot be
/// represented — a path with a newline or a quote in it is skipped rather
/// than allowed to terminate the literal early and change what the rest of
/// the profile means.
fn sbpl_string(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    if text.contains('"') || text.contains('\\') || text.contains('\n') {
        return None;
    }
    Some(format!("\"{text}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_denies_writes_before_it_allows_the_exemptions_back() {
        let Ok(plan) = ReadOnly::build(&[PathBuf::from("/tmp/project")]) else {
            return; // no sandbox-exec on this machine; nothing to assert
        };
        let deny = plan.profile.find("(deny file-write*)").expect("a blanket write denial");
        let allow = plan.profile.find("(allow file-write* (subpath").expect("an exemption");
        assert!(deny < allow, "Seatbelt is last-match-wins: exemptions must follow the denial");
    }

    #[test]
    fn the_wrapped_command_line_puts_the_real_program_after_a_double_dash() {
        let Ok(plan) = ReadOnly::build(&[PathBuf::from("/tmp/project")]) else {
            return; // no sandbox-exec on this machine
        };
        let (program, argv) = plan.command_line("/bin/grep", &["-r".to_string(), "needle".to_string()]);
        assert_eq!(program, SANDBOX_EXEC);
        let dash = argv.iter().position(|a| a == "--").expect("a -- separator");
        assert_eq!(argv[dash + 1], "/bin/grep");
        assert_eq!(&argv[dash + 2..], &["-r".to_string(), "needle".to_string()]);
    }

    #[test]
    fn a_path_that_cannot_be_quoted_is_skipped_not_embedded() {
        assert_eq!(sbpl_string(Path::new("/tmp/ok")), Some("\"/tmp/ok\"".to_string()));
        assert_eq!(sbpl_string(Path::new("/tmp/we\"ird")), None);
        assert_eq!(sbpl_string(Path::new("/tmp/new\nline")), None);
    }
}
