use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::error::ToolError;

/// The directories tools may address and the only ones a process Aldwin
/// starts may write to: the whole boundary (ADR 0007, ADR 0011).
///
/// Roots are stated, never inferred: the project root, then `roots:` from
/// `.aldwin/permissions.yaml`. Every tool's path argument (`run`'s `cwd`
/// included) goes through [`Workspace::resolve`], symlinks included; every
/// spawned process runs in `crate::sandbox`. Reads are not bounded (ADR 0011).
///
/// Clones share the roots, so `/reload-config` through one reaches all.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Canonical, absolute. `roots[0]` is the project root (base of relative
    /// paths, default working directory) and is never replaced.
    roots: Arc<RwLock<Vec<PathBuf>>>,
}

impl Workspace {
    /// A workspace of the project root alone.
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self::with_roots(project_root, Vec::new())
    }

    /// A workspace of the project root plus `extra`; a missing root is
    /// dropped, as in [`Self::set_extra_roots`].
    pub fn with_roots(project_root: impl Into<PathBuf>, extra: Vec<PathBuf>) -> Self {
        let project_root = project_root.into();
        let canonical_project = project_root.canonicalize().unwrap_or(project_root);
        let workspace = Self {
            roots: Arc::new(RwLock::new(vec![canonical_project])),
        };
        workspace.set_extra_roots(extra);
        workspace
    }

    /// Replaces every root but the project root. Returns the roots dropped
    /// because they could not be canonicalized, for the caller to tell the
    /// developer; never swallow them.
    pub fn set_extra_roots(&self, extra: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut roots = self.roots.write().unwrap_or_else(|e| e.into_inner());
        roots.truncate(1);
        let mut dropped = Vec::new();
        for root in extra {
            match root.canonicalize() {
                Ok(canonical) => {
                    if !roots.contains(&canonical) {
                        roots.push(canonical);
                    }
                }
                Err(_) => dropped.push(root),
            }
        }
        dropped
    }

    /// The project root, which relative paths resolve against.
    pub fn project_root(&self) -> PathBuf {
        self.read_roots()[0].clone()
    }

    /// Every root the workspace reaches, canonical, the project root first.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.read_roots().clone()
    }

    fn read_roots(&self) -> std::sync::RwLockReadGuard<'_, Vec<PathBuf>> {
        self.roots.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Resolves a tool's path argument, refusing to leave the workspace.
    /// Synchronous on purpose: a few `canonicalize` calls cost less than a
    /// `spawn_blocking` hop.
    ///
    /// A relative path joins the project root; an absolute one is taken as
    /// given (a second root must be addressable). Containment is decided on
    /// resolved paths only, never the path as typed (on macOS `/tmp` and
    /// `/var` are symlinks). Both forms must be inside a root:
    ///
    /// 1. The lexically normalized path with its existing prefix
    ///    canonicalized: covers a target not yet created, and a symlink inside
    ///    a root pointing out.
    /// 2. The filesystem's own resolution, when the path exists: after a
    ///    symlink, real `..` differs from lexical `..`.
    ///
    /// Returns the normalized path, so what (1) checked is what gets opened.
    ///
    /// # Errors
    ///
    /// [`ToolError::PathEscapesWorkspace`] when either form lies outside
    /// every root; [`ToolError::Io`] when the existing prefix cannot be
    /// canonicalized.
    pub fn resolve(&self, path_str: &str) -> Result<PathBuf, ToolError> {
        let candidate = Path::new(path_str);
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.project_root().join(candidate)
        };
        let escapes = || ToolError::PathEscapesWorkspace {
            path: path_str.to_string(),
            roots: self.describe(),
        };

        let normalized = normalize_lexically(&joined);
        let canonical =
            canonicalize_existing_prefix(&normalized).map_err(|source| ToolError::Io {
                path: joined.clone(),
                source,
            })?;
        if !self.contains(&canonical) {
            return Err(escapes());
        }
        if let Ok(real) = joined.canonicalize() {
            if !self.contains(&real) {
                return Err(escapes());
            }
        }

        Ok(normalized)
    }

    /// Whether an already-canonical absolute path sits under any root.
    fn contains(&self, path: &Path) -> bool {
        self.read_roots().iter().any(|root| path.starts_with(root))
    }

    /// How the roots read in a message to the developer or the model.
    pub(crate) fn describe(&self) -> String {
        self.read_roots()
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Canonicalizes the longest existing ancestor of `path` and re-appends the
/// missing tail lexically, since the target may not exist yet.
/// `path` must be lexically normalized: the ancestor walk does not interpret
/// `..`.
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
                let Some(parent) = ancestor.parent() else {
                    return Err(err);
                };
                if let Some(name) = ancestor.file_name() {
                    tail.push(name);
                }
                ancestor = parent;
            }
        }
    }
}

/// Collapses `.`/`..` without touching the filesystem; the target may not
/// exist yet.
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

    // Real temp directories: `Workspace` canonicalizes its roots, so they
    // must exist.

    #[test]
    fn ordinary_relative_paths_resolve_under_the_root() {
        let root = tempdir().unwrap();
        let ws = Workspace::new(root.path());
        let resolved = ws.resolve("src/main.rs").unwrap();
        assert_eq!(resolved, ws.project_root().join("src/main.rs"));
    }

    #[test]
    fn an_absolute_path_outside_every_root_is_rejected() {
        let root = tempdir().unwrap();
        let ws = Workspace::new(root.path());
        let err = ws.resolve("/etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesWorkspace { .. }));
    }

    #[test]
    fn dot_dot_climbing_above_the_root_is_rejected() {
        let root = tempdir().unwrap();
        let ws = Workspace::new(root.path());
        let err = ws.resolve("../../etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesWorkspace { .. }));
    }

    #[test]
    fn dot_dot_that_stays_inside_the_root_is_allowed() {
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let ws = Workspace::new(root.path());
        assert!(ws.resolve("src/../src/main.rs").is_ok());
    }

    #[test]
    fn a_nonexistent_target_under_a_real_directory_is_allowed() {
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let ws = Workspace::new(root.path());
        assert!(ws.resolve("src/new.rs").is_ok());
    }

    /// ADR 0007.
    #[test]
    fn a_second_root_is_reachable_by_absolute_path() {
        let project = tempdir().unwrap();
        let sibling = tempdir().unwrap();
        std::fs::write(sibling.path().join("notes.md"), "x").unwrap();

        let ws = Workspace::with_roots(project.path(), vec![sibling.path().to_path_buf()]);
        let target = sibling.path().join("notes.md");
        assert!(ws.resolve(target.to_str().unwrap()).is_ok());
    }

    #[test]
    fn a_sibling_that_was_not_declared_a_root_stays_out_of_reach() {
        let project = tempdir().unwrap();
        let sibling = tempdir().unwrap();
        std::fs::write(sibling.path().join("notes.md"), "x").unwrap();

        let ws = Workspace::new(project.path());
        let target = sibling.path().join("notes.md");
        assert!(matches!(
            ws.resolve(target.to_str().unwrap()).unwrap_err(),
            ToolError::PathEscapesWorkspace { .. }
        ));
    }

    #[test]
    fn a_root_that_does_not_exist_is_dropped_rather_than_failing_startup() {
        let project = tempdir().unwrap();
        let ws = Workspace::with_roots(project.path(), vec![PathBuf::from("/nope/not/here")]);
        assert_eq!(ws.roots().len(), 1);
    }

    /// Regression: a lexical check passed a symlink pointing outside.
    #[test]
    #[cfg(unix)]
    fn a_symlink_inside_a_root_pointing_outside_it_is_rejected() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("out")).unwrap();

        let ws = Workspace::new(root.path());
        assert!(matches!(
            ws.resolve("out/x").unwrap_err(),
            ToolError::PathEscapesWorkspace { .. }
        ));
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_inside_a_root_pointing_inside_it_is_allowed() {
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("real")).unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("link")).unwrap();

        let ws = Workspace::new(root.path());
        assert!(ws.resolve("link/x.rs").is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_inside_a_root_pointing_outside_it_is_rejected_even_for_a_new_file() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("out")).unwrap();

        let ws = Workspace::new(root.path());
        assert!(matches!(
            ws.resolve("out/new_file.rs").unwrap_err(),
            ToolError::PathEscapesWorkspace { .. }
        ));
    }

    /// Regression: a check on the path as typed refused this (every `/tmp`
    /// path on macOS).
    #[test]
    #[cfg(unix)]
    fn an_absolute_path_reaching_a_root_through_a_symlink_is_allowed() {
        let real = tempdir().unwrap();
        std::fs::write(real.path().join("f.txt"), "x").unwrap();
        let holder = tempdir().unwrap();
        let alias = holder.path().join("alias");
        std::os::unix::fs::symlink(real.path(), &alias).unwrap();

        let ws = Workspace::new(real.path());
        assert!(ws.resolve(alias.join("f.txt").to_str().unwrap()).is_ok());
    }

    /// `out/../secret` normalizes inside but opens `<outside>/secret`.
    #[test]
    #[cfg(unix)]
    fn dot_dot_after_a_symlink_cannot_be_used_to_climb_out() {
        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::create_dir(outside.path().join("d")).unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path().join("d"), root.path().join("out")).unwrap();

        let ws = Workspace::new(root.path());
        assert!(matches!(
            ws.resolve("out/../secret").unwrap_err(),
            ToolError::PathEscapesWorkspace { .. }
        ));
    }

    #[test]
    fn replaced_roots_are_seen_by_every_clone_and_dropped_ones_are_reported() {
        let project = tempdir().unwrap();
        let sibling = tempdir().unwrap();
        let ws = Workspace::new(project.path());
        let held_by_a_tool = ws.clone();

        let dropped = ws.set_extra_roots(vec![
            sibling.path().to_path_buf(),
            PathBuf::from("/nope/not/here"),
        ]);
        assert_eq!(dropped, vec![PathBuf::from("/nope/not/here")]);
        assert_eq!(held_by_a_tool.roots().len(), 2);

        ws.set_extra_roots(vec![]);
        assert_eq!(
            held_by_a_tool.roots().len(),
            1,
            "the project root is never replaced"
        );
    }
}
