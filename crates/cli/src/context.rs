use std::path::{Path, PathBuf};

/// Builds the opaque additional-context string handed to aldwin-core: the
/// absolute cwd path, then the full text of each *approved* context file
/// (in the order given). `approved` is expected to already be the fully
/// resolved list — see `context_approval::resolve`, which is what actually
/// enforces "only approved files, never a raw filesystem walk" by going
/// through the permission engine; this function just formats whatever
/// list it's handed.
pub fn build(cwd: &Path, approved: &[PathBuf]) -> String {
    let mut sections = vec![format!("Working directory: {}", cwd.display())];

    for path in approved {
        // A path that's approved but no longer exists (renamed, deleted
        // since approval) is skipped rather than treated as an error —
        // GC of stale approved-path entries is explicitly deferred past V0
        // per aldwin-permissions.md's Pitfalls; this is just the read
        // side tolerating that gap gracefully.
        if let Ok(contents) = std::fs::read_to_string(path) {
            sections.push(format!("--- {} ---\n{}", path.display(), contents));
        }
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_cwd_and_nothing_else_when_no_files_are_approved() {
        let out = build(Path::new("/some/project"), &[]);
        assert_eq!(out, "Working directory: /some/project");
    }

    #[test]
    fn includes_the_full_text_of_each_approved_file_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let claude_md = dir.path().join("CLAUDE.md");
        std::fs::write(&claude_md, "# Project notes\nBe careful.").unwrap();

        let out = build(dir.path(), std::slice::from_ref(&claude_md));
        assert!(out.contains("Working directory:"));
        assert!(out.contains(&claude_md.display().to_string()));
        assert!(out.contains("Be careful."));
    }

    #[test]
    fn a_path_that_no_longer_exists_is_skipped_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.md");

        let out = build(dir.path(), &[missing]); // must not panic
        assert_eq!(out, format!("Working directory: {}", dir.path().display()));
    }
}
