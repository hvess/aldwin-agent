use std::path::Path;

use amundsen_config::Config;

/// Builds the opaque additional-context string handed to amundsen-core:
/// the absolute cwd path, then the full text of each *approved* context
/// file, in path order. Per amundsen-cli.md: a file present on disk but
/// absent from `project_context_files()`'s approved list is excluded
/// regardless of existence — the permission check is not optional, so this
/// never walks the filesystem itself, only ever reads paths already in the
/// approved snapshot.
pub fn build(cwd: &Path, config: &Config) -> String {
    let mut sections = vec![format!("Working directory: {}", cwd.display())];

    for path in &config.project_context_files().approved {
        // A path that's approved but no longer exists (renamed, deleted
        // since approval) is skipped rather than treated as an error —
        // GC of stale approved-path entries is explicitly deferred past V0
        // per amundsen-permissions.md's Pitfalls; this is just the read
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
    use amundsen_config::Config;
    use std::path::PathBuf;

    fn config_with_approved(project: &Path, global: &Path, approved: Vec<PathBuf>) -> Config {
        let config = Config::open_at(project, global).unwrap();
        for path in approved {
            config.add_context_file(path).unwrap();
        }
        config
    }

    #[test]
    fn includes_cwd_and_no_context_files_when_none_approved() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let config = Config::open_at(project.path(), global.path()).unwrap();

        let out = build(Path::new("/some/project"), &config);
        assert_eq!(out, "Working directory: /some/project");
    }

    #[test]
    fn includes_the_full_text_of_each_approved_file() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let claude_md = project.path().join("CLAUDE.md");
        std::fs::write(&claude_md, "# Project notes\nBe careful.").unwrap();
        let config = config_with_approved(project.path(), global.path(), vec![claude_md.clone()]);

        let out = build(project.path(), &config);
        assert!(out.contains("Working directory:"));
        assert!(out.contains(&claude_md.display().to_string()));
        assert!(out.contains("Be careful."));
    }

    #[test]
    fn a_file_present_on_disk_but_not_approved_is_excluded() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        std::fs::write(project.path().join("AGENTS.md"), "secret instructions").unwrap();
        // Note: never calling add_context_file — this file is on disk but
        // not in the approved list.
        let config = Config::open_at(project.path(), global.path()).unwrap();

        let out = build(project.path(), &config);
        assert!(!out.contains("secret instructions"));
    }

    #[test]
    fn an_approved_path_that_no_longer_exists_is_skipped_not_an_error() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let missing = project.path().join("gone.md");
        let config = config_with_approved(project.path(), global.path(), vec![missing]);

        let out = build(project.path(), &config); // must not panic
        assert_eq!(out, format!("Working directory: {}", project.path().display()));
    }
}
