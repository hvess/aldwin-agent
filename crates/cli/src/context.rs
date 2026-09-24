use std::path::{Path, PathBuf};

/// Builds the opaque additional-context string handed to aldwin-core: the
/// absolute cwd path, then the full text of each context file in the order
/// given. The caller (`bootstrap::context_files`) decides which files those
/// are — the two conventional names at the project root, whichever exist;
/// this function just formats whatever list it's handed.
///
/// Also carries the platform facts and the workspace roots.
///
/// The platform half is there because the agent otherwise learns it by
/// failing: an observed session wrote a bash 4 associative array on a macOS
/// bash 3.2, watched it fail, rewrote it, and only checked `bash --version`
/// four turns later — then shipped a script using GNU `sed -i` syntax that
/// would not run on the machine it was written on. None of that is a
/// judgement call; it is three facts that are free to state up front.
///
/// The roots half is there because "outside the project root" was a refusal
/// the model could not act on without knowing what the root was.
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

    sections.push(platform_facts());

    for path in approved {
        // A file that went away between the caller finding it and this read
        // is skipped rather than treated as an error.
        if let Ok(contents) = std::fs::read_to_string(path) {
            sections.push(format!("--- {} ---\n{}", path.display(), contents));
        }
    }

    sections.join("\n\n")
}

/// Facts about this machine that a shell script has to be right about.
/// Detected, never assumed — a wrong fact here is worse than none.
fn platform_facts() -> String {
    let mut facts = vec![format!("Platform: {}", std::env::consts::OS)];

    if let Some(version) = program_version("bash", &["--version"]) {
        facts.push(format!("bash: {version}"));
    }
    // The distinction that actually bites: GNU `sed -i` takes no argument,
    // BSD `sed -i` requires one. Asked of the `sed` on PATH, not inferred
    // from the OS — a Mac with gnu-sed installed is GNU. GNU answers
    // `--version`; BSD sed has no such flag and fails.
    match program_version("sed", &["--version"]) {
        Some(version) if version.contains("GNU") => {
            facts.push("sed: GNU (in-place edit is `sed -i`)".to_string())
        }
        Some(_) => {}
        None if which("sed") => {
            facts.push("sed: BSD (in-place edit is `sed -i ''`, not `sed -i`)".to_string())
        }
        None => {}
    }
    facts.join("\n")
}

/// Whether `program` is on PATH at all.
fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// First line of `<program> <args>`, or `None` if it cannot be run or fails. Best
/// effort: a missing program is a fact we simply do not state.
fn program_version(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
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

    /// The facts are stated so the agent does not have to learn them by
    /// failing — a bash 3.2 associative array, a GNU `sed -i` on BSD.
    #[test]
    fn states_the_platform_facts_a_shell_script_has_to_be_right_about() {
        let out = build(Path::new("/some/project"), &[], &[]);
        assert!(out.contains(std::env::consts::OS));
        assert!(out.contains("sed:"));
        // Which flavour depends on the `sed` on PATH, not on the OS — that is
        // the point — so assert only that a detected one is described usably.
        assert!(
            out.contains("sed -i"),
            "the fact must say how to edit in place: {out}"
        );
    }

    /// A second root is named, because "outside the project root" was a
    /// refusal the model could not act on without knowing the root.
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
        assert!(!out.contains("Also reachable"));
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
