use std::path::{Component, Path, PathBuf};

use crate::error::ToolError;

/// Resolves `path_str` against `project_root`, refusing to leave it.
///
/// `PathBuf::join` silently *discards* the base when the joined path is
/// absolute (standard library semantics) — `project_root.join("/etc/passwd")`
/// is just `/etc/passwd`, not an error, which is exactly how a model-supplied
/// absolute path used to slip straight past a broad `read:**`-style grant
/// (the grant matches the model's literal string argument, which has no way
/// to know the join silently escaped the project). This rejects absolute
/// paths outright and any relative path whose `..` components climb above
/// `project_root` once resolved.
///
/// The lexical check alone doesn't see through a symlink planted *inside*
/// the project tree pointing outside `project_root` (legal in a git repo —
/// symlinks are ordinary blobs and git enforces no containment on their
/// target) — a broad `read:**`/edit grant would otherwise let a call through
/// a path like `out/x` actually touch whatever `out` points at, arbitrarily
/// far outside the tree the developer thinks they've confined the model to.
/// So containment is re-checked against the canonicalized (symlink-resolved)
/// form too — see `canonicalize_existing_prefix` for how that copes with a
/// target that may not exist yet (Edit writing a new file). `joined` itself,
/// not the canonicalized form, is what's returned and used for actual I/O,
/// so behavior for a path with no symlinks involved is unchanged.
pub fn resolve_in_project(project_root: &Path, path_str: &str) -> Result<PathBuf, ToolError> {
    let candidate = Path::new(path_str);
    if candidate.is_absolute() {
        return Err(ToolError::PathEscapesProject { path: path_str.to_string() });
    }

    let joined = project_root.join(candidate);
    let normalized_root = normalize_lexically(project_root);
    let normalized_joined = normalize_lexically(&joined);
    if !normalized_joined.starts_with(&normalized_root) {
        return Err(ToolError::PathEscapesProject { path: path_str.to_string() });
    }

    let canonical_root = project_root.canonicalize().map_err(|source| ToolError::Io { path: project_root.to_path_buf(), source })?;
    // Re-anchor the lexically-resolved tail onto `canonical_root` (always
    // absolute) rather than canonicalizing `normalized_joined` as-is — a
    // relative `project_root` (e.g. `.`) lexically normalizes away its own
    // anchor (`normalize_lexically` drops `Component::CurDir` outright),
    // which would otherwise leave nothing for `canonicalize_existing_prefix`
    // to walk up to.
    let relative_tail = normalized_joined.strip_prefix(&normalized_root).expect("starts_with just checked above");
    let canonical_joined =
        canonicalize_existing_prefix(&canonical_root.join(relative_tail)).map_err(|source| ToolError::Io { path: joined.clone(), source })?;
    if !canonical_joined.starts_with(&canonical_root) {
        return Err(ToolError::PathEscapesProject { path: path_str.to_string() });
    }

    Ok(joined)
}

/// Canonicalizes the longest *existing* ancestor of `path` (resolving any
/// symlink along it) and re-appends whatever tail doesn't exist on disk yet,
/// lexically — the target itself may not exist (Edit writing a brand-new
/// file), and `Path::canonicalize` errors on any component that doesn't.
/// `path` must already be lexically normalized (no `.`/`..`): this walks
/// ancestors via plain component-stripping, which doesn't understand `..`
/// semantically, so an un-normalized `a/../b` would double-count `a` instead
/// of cancelling it.
fn canonicalize_existing_prefix(path: &Path) -> std::io::Result<PathBuf> {
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let mut ancestor = path;
    loop {
        match ancestor.canonicalize() {
            Ok(mut canonical) => {
                for component in tail.into_iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(err) => {
                let Some(parent) = ancestor.parent() else { return Err(err) };
                if let Some(name) = ancestor.file_name() {
                    tail.push(name);
                }
                ancestor = parent;
            }
        }
    }
}

/// Collapses `.`/`..` components without touching the filesystem (the
/// target may not exist yet, e.g. Edit writing a new file) — purely for the
/// containment check above; the returned `joined` path from
/// `resolve_in_project` is left as-is for actual I/O, which resolves `..`
/// correctly on its own.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Real temp directories, not fabricated paths like `/home/user/project`
    // — `resolve_in_project` now canonicalizes `project_root` itself as part
    // of the symlink-containment check, which requires the root to actually
    // exist on disk.

    #[test]
    fn ordinary_relative_paths_resolve_under_the_root() {
        let root = tempdir().unwrap();
        let resolved = resolve_in_project(root.path(), "src/main.rs").unwrap();
        assert_eq!(resolved, root.path().join("src/main.rs"));
    }

    #[test]
    fn absolute_paths_are_rejected_outright() {
        let root = tempdir().unwrap();
        let err = resolve_in_project(root.path(), "/etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesProject { .. }));
    }

    #[test]
    fn dot_dot_climbing_above_the_root_is_rejected() {
        let root = tempdir().unwrap();
        let err = resolve_in_project(root.path(), "../../etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesProject { .. }));
    }

    #[test]
    fn dot_dot_that_stays_inside_the_root_is_allowed() {
        let root = tempdir().unwrap();
        let resolved = resolve_in_project(root.path(), "src/../src/main.rs").unwrap();
        assert_eq!(resolved, root.path().join("src/../src/main.rs"));
    }

    #[test]
    fn a_nonexistent_target_under_a_real_directory_is_allowed() {
        // Edit writing a brand-new file: `src/` exists, `new.rs` doesn't yet.
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let resolved = resolve_in_project(root.path(), "src/new.rs").unwrap();
        assert_eq!(resolved, root.path().join("src/new.rs"));
    }

    /// Regression test for the symlink-escape gap found in the tools audit:
    /// a symlink lexically inside `project_root` but pointing outside it
    /// used to pass the lexical containment check even though the real I/O
    /// it enables reaches outside the project.
    #[test]
    #[cfg(unix)]
    fn a_symlink_inside_the_root_pointing_outside_it_is_rejected() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("out")).unwrap();

        let err = resolve_in_project(root.path(), "out/x").unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesProject { .. }));
    }

    /// A symlink that stays inside the project must not be rejected as a
    /// false positive — only escaping symlinks are a problem.
    #[test]
    #[cfg(unix)]
    fn a_symlink_inside_the_root_pointing_inside_it_is_allowed() {
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("real")).unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("link")).unwrap();

        let resolved = resolve_in_project(root.path(), "link/x.rs").unwrap();
        assert_eq!(resolved, root.path().join("link/x.rs"));
    }

    /// Same escape, but through a symlink to a not-yet-existing file (the
    /// Edit-writing-a-new-file case) — `canonicalize_existing_prefix` must
    /// still resolve the symlinked ancestor even though the final component
    /// itself doesn't exist.
    #[test]
    #[cfg(unix)]
    fn a_symlink_inside_the_root_pointing_outside_it_is_rejected_even_for_a_new_file() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("out")).unwrap();

        let err = resolve_in_project(root.path(), "out/new_file.rs").unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesProject { .. }));
    }
}
