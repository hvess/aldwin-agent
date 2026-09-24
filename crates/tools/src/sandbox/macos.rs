//! Seatbelt, spoken through `sandbox-exec`.
//!
//! The same rule as the Linux backend — *write only beneath the roots and
//! the incidental paths* — reached by a different route, and the difference
//! is deliberate.
//!
//! Landlock is engaged **inside the forked child**, which is safe because
//! engaging it is two syscalls with no allocation. macOS's equivalent
//! primitive, `sandbox_init_with_parameters`, is not: it compiles an SBPL
//! profile, which allocates, and calling it between `fork` and `execve` in a
//! threaded process is how you get a child that deadlocks in the allocator.
//! So this backend runs nothing in the child. It rewrites the command to run
//! under `/usr/bin/sandbox-exec`, which applies the profile to itself and
//! then `exec`s the real program, which inherits the confinement.
//!
//! **`sandbox-exec` is deprecated and has been since 10.8.** It is also still
//! shipped, still the only route to this primitive without entitlements,
//! and if it goes away `unavailable` says so and processes run unconfined
//! with the developer told once — never silently.

use std::io;
use std::path::{Path, PathBuf};

use super::incidental_writes;

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

pub fn unavailable() -> Option<&'static str> {
    if Path::new(SANDBOX_EXEC).exists() {
        None
    } else {
        Some("/usr/bin/sandbox-exec is not present on this system")
    }
}

/// An SBPL profile: everything allowed, then every write denied, then writes
/// beneath the roots and the incidental paths allowed back.
///
/// Rule order is the opposite of what a firewall reader expects: **Seatbelt
/// is last-match-wins**, so the exemptions come after the blanket denial.
pub struct Sandbox {
    profile: String,
}

impl Sandbox {
    pub fn build(roots: &[PathBuf]) -> io::Result<Self> {
        let mut profile = String::from(
            "(version 1)\n\
             (allow default)\n\
             (deny file-write*)\n",
        );
        for path in roots.iter().cloned().chain(incidental_writes()) {
            // Seatbelt matches the *resolved* path of the file being written,
            // and on macOS the incidental paths are mostly symlinks: `/tmp` is
            // `/private/tmp`, and `$TMPDIR` lives under `/var`, which is
            // `/private/var`. A rule for the path as written never matches,
            // so both forms are emitted; the redundant one is harmless.
            //
            // An unquotable path is skipped rather than allowed to end the
            // literal early and change what the rest of the profile means.
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

    /// The real program, run under `sandbox-exec` with this profile. `--`
    /// keeps a program whose own first argument starts with `-` from being
    /// read as a flag to `sandbox-exec`.
    pub fn command_line(&self, program: &str, args: &[String]) -> (String, Vec<String>) {
        let mut argv = vec![
            "-p".to_string(),
            self.profile.clone(),
            "--".to_string(),
            program.to_string(),
        ];
        argv.extend(args.iter().cloned());
        (SANDBOX_EXEC.to_string(), argv)
    }

    /// Nothing to install in the child: `sandbox-exec` confines itself and
    /// then `exec`s the real program.
    pub fn install(self, _cmd: &mut tokio::process::Command) {}
}

/// Quotes a path as an SBPL string literal, or `None` if it cannot be
/// represented.
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
    fn the_profile_denies_writes_then_allows_the_workspace_back() {
        let sandbox = Sandbox::build(&[PathBuf::from("/Users/you/project")]).unwrap();
        let deny = sandbox
            .profile
            .find("(deny file-write*)")
            .expect("a blanket write denial");
        let root = sandbox
            .profile
            .find("(allow file-write* (subpath \"/Users/you/project\"))")
            .expect("the root allowed back");
        assert!(
            deny < root,
            "Seatbelt is last-match-wins: exemptions must follow the denial"
        );
        assert!(
            !sandbox.profile.contains("network"),
            "the network is not restricted (ADR 0011)"
        );
    }

    #[test]
    fn the_wrapped_command_line_puts_the_real_program_after_a_double_dash() {
        let sandbox = Sandbox::build(&[PathBuf::from("/tmp/project")]).unwrap();
        let (program, argv) =
            sandbox.command_line("/bin/sh", &["-c".to_string(), "ls | head".to_string()]);
        assert_eq!(program, SANDBOX_EXEC);
        let dash = argv.iter().position(|a| a == "--").expect("a -- separator");
        assert_eq!(argv[dash + 1], "/bin/sh");
        assert_eq!(
            &argv[dash + 2..],
            &["-c".to_string(), "ls | head".to_string()]
        );
    }

    #[test]
    fn a_path_that_cannot_be_quoted_is_skipped_not_embedded() {
        assert_eq!(
            sbpl_string(Path::new("/tmp/ok")),
            Some("\"/tmp/ok\"".to_string())
        );
        assert_eq!(sbpl_string(Path::new("/tmp/we\"ird")), None);
        assert_eq!(sbpl_string(Path::new("/tmp/new\nline")), None);
    }
}
