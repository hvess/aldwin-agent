//! Seatbelt through `sandbox-exec`: the Linux backend's rule, applied by
//! rewriting the command rather than in the child. Do not call
//! `sandbox_init_with_parameters` in the child: it allocates, and allocating
//! between `fork` and `execve` in a threaded process can deadlock.
//!
//! `sandbox-exec` is deprecated but shipped, and the only route without
//! entitlements. If it goes, `unavailable` says so and the developer is told
//! once.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

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
/// beneath the roots and the incidental paths allowed back. Seatbelt is
/// last-match-wins: the exemptions must follow the denial.
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
            // Seatbelt matches resolved paths and `/tmp`, `/var` are symlinks
            // into `/private`, so both forms are emitted. An unquotable path
            // is skipped: it could end the literal and rewrite the profile.
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

    /// The program under `sandbox-exec` with this profile. `--` keeps a
    /// program starting with `-` from being read as a `sandbox-exec` flag.
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

    /// No-op: `sandbox-exec` confines itself, then `exec`s the program.
    pub fn install(self, _cmd: &mut Command) {}
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
