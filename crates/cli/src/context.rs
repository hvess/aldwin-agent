use std::path::{Path, PathBuf};
use std::process::Output;

/// The additional-context string handed to aldwin-core: the cwd, any extra
/// workspace roots, the platform facts, then the full text of each file in
/// `approved`, in order. `bootstrap::context_files` chooses the files.
///
/// The roots are named so the model can act on an "outside the workspace"
/// refusal; the platform facts (bash version, sed flavour) spare it learning
/// them by failing.
pub fn build(cwd: &Path, roots: &[PathBuf], approved: &[PathBuf]) -> String {
    let mut sections = vec![format!("Working directory: {}", cwd.display())];

    if roots.len() > 1 {
        let extra: Vec<String> = roots
            .iter()
            .skip(1)
            .map(|r| r.display().to_string())
            .collect();
        sections.push(format!(
            "Also in the workspace (declared in .aldwin/permissions.yaml): {}\nEvery tool refuses a path outside the workspace, and a command you run cannot write outside it.",
            extra.join(", ")
        ));
    }

    sections.push(platform_facts(roots));

    for path in approved {
        // A file removed since the caller found it is skipped, not an error.
        if let Ok(contents) = std::fs::read_to_string(path) {
            sections.push(format!("--- {} ---\n{}", path.display(), contents));
        }
    }

    sections.join("\n\n")
}

/// Facts a shell script must get right on this machine. Detected, never
/// assumed: a wrong fact is worse than none.
fn platform_facts(roots: &[PathBuf]) -> String {
    let mut facts = vec![format!("Platform: {}", std::env::consts::OS)];

    if let Some(version) = program_version("bash", &["--version"], roots) {
        facts.push(format!("bash: {version}"));
    }
    facts.extend(
        probe("sed", &["--version"], roots)
            .and_then(|out| {
                sed_fact(
                    &String::from_utf8_lossy(&out.stdout),
                    &String::from_utf8_lossy(&out.stderr),
                )
            })
            .map(str::to_string),
    );
    facts.join("\n")
}

/// The `sed` fact from what `sed --version` printed. GNU `sed -i` takes no
/// argument, BSD's requires one. Ask the `sed` on PATH, not the OS: a Mac
/// can have GNU sed. Only a flavour's own words are evidence: BSD sed
/// rejects `--version` with its usage line, and a launcher that could not
/// start `sed` (macOS's `sandbox-exec`) also exits non-zero.
fn sed_fact(stdout: &str, stderr: &str) -> Option<&'static str> {
    if stdout.contains("(GNU sed)") {
        Some("sed: GNU (in-place edit is `sed -i`)")
    } else if stderr.contains("usage: sed") {
        Some("sed: BSD (in-place edit is `sed -i ''`, not `sed -i`)")
    } else {
        None
    }
}

/// First line of `<program> <args>`, or `None` if it cannot be run or fails.
fn program_version(program: &str, args: &[&str], roots: &[PathBuf]) -> Option<String> {
    let out = probe(program, args, roots).filter(|out| out.status.success())?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

/// What `<program> <args>` printed; `None` if it could not be started. Must
/// run in the sandbox, as every process Aldwin starts does (ADR 0011).
fn probe(program: &str, args: &[&str], roots: &[PathBuf]) -> Option<Output> {
    aldwin_tools::sandbox::std_command(program, args, roots)
        .ok()?
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leads_with_the_cwd_and_carries_no_file_text_when_none_are_approved() {
        let out = build(Path::new("/some/project"), &[], &[]);
        assert!(out.starts_with("Working directory: /some/project"));
        assert!(out.contains("Platform:"));
        assert!(!out.contains("---"), "no approved file sections");
    }

    /// Regression: the probes ran outside the sandbox (ADR 0011). Only
    /// `sandbox::std_command` sets `AGENT`, so seeing it proves the route.
    #[test]
    fn a_probe_runs_through_the_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let seen = program_version(
            "sh",
            &["-c", "printf %s \"$AGENT\""],
            &[dir.path().to_path_buf()],
        );
        assert_eq!(seen.as_deref(), Some("aldwin"));
    }

    /// Regression: a `sed` that could not be started was reported as BSD.
    #[test]
    fn a_sed_that_cannot_be_run_is_no_evidence_of_bsd() {
        assert_eq!(
            sed_fact("", "sandbox-exec: execvp() of 'sed' failed: No such file"),
            None
        );
        let bsd = "sed: illegal option -- -\nusage: sed script [-Ealnru] [-i extension]";
        assert!(sed_fact("", bsd).is_some_and(|f| f.contains("BSD")));
        assert!(sed_fact("sed (GNU sed) 4.9\n", "").is_some_and(|f| f.contains("GNU")));
        assert_eq!(
            sed_fact("This is not GNU sed version 4.0\n", ""),
            None,
            "busybox"
        );
    }

    #[test]
    fn states_the_platform_facts_a_shell_script_has_to_be_right_about() {
        let out = build(Path::new("/some/project"), &[], &[]);
        assert!(out.contains(std::env::consts::OS));
        assert!(out.contains("sed:"));
        // The flavour depends on the `sed` on PATH, so only its wording is
        // asserted.
        assert!(
            out.contains("sed -i"),
            "the fact must say how to edit in place: {out}"
        );
    }

    #[test]
    fn names_every_reachable_root_when_more_than_one_is_declared() {
        let out = build(
            Path::new("/some/project"),
            &[
                PathBuf::from("/some/project"),
                PathBuf::from("/other/checkout"),
            ],
            &[],
        );
        assert!(out.contains("/other/checkout"));
        assert!(out.contains("cannot write outside it"));
    }

    #[test]
    fn a_single_root_adds_no_reachability_section() {
        let out = build(
            Path::new("/some/project"),
            &[PathBuf::from("/some/project")],
            &[],
        );
        assert!(!out.contains("Also in the workspace"));
    }

    #[test]
    fn includes_the_full_text_of_each_approved_file_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let claude_md = dir.path().join("CLAUDE.md");
        std::fs::write(&claude_md, "# Project notes\nBe careful.").unwrap();

        let out = build(dir.path(), &[], std::slice::from_ref(&claude_md));
        assert!(out.contains("Working directory:"));
        assert!(out.contains(&claude_md.display().to_string()));
        assert!(out.contains("Be careful."));
    }

    #[test]
    fn a_path_that_no_longer_exists_is_skipped_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.md");

        let out = build(dir.path(), &[], &[missing]); // must not panic
        assert!(out.starts_with(&format!("Working directory: {}", dir.path().display())));
        assert!(!out.contains("gone.md"));
    }
}
