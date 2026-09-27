use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::error::ToolError;

/// The workspace: the directories tools may be pointed at, and the only
/// directories a process Aldwin starts may write to (ADR 0007, and ADR 0011,
/// which makes it the whole boundary).
///
/// Roots are stated, never inferred: the first is the project root and the
/// rest come from `roots:` in the project's `.aldwin/permissions.yaml`. A
/// *list* rather than a single root because a developer working across
/// sibling checkouts needs a sanctioned way to say so, or the unsanctioned
/// way carries the work.
///
/// Two halves hold the line. Every tool's path argument — `read`, `edit`,
/// `explain`, `run`'s `cwd` — resolves through [`Workspace::resolve`], which
/// refuses anything outside, symlinks included. And every process a tool
/// starts runs in `crate::sandbox`, which lets it write nowhere else. What
/// a process *reads* is not bounded; ADR 0011 says why.
///
/// The roots are shared between clones: every tool holds a `Workspace`, and
/// `/reload-config` replacing the list through one of them is visible to all
/// of them on the next call.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Canonical, absolute. `roots[0]` is the project root — the directory
    /// relative paths resolve against and the default working directory —
    /// and is never replaced.
    roots: Arc<RwLock<Vec<PathBuf>>>,
}

impl Workspace {
    /// The single-root case: reach is the project root and nothing else.
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self::with_roots(project_root, Vec::new())
    }

    /// `extra` widens reach beyond the project root. See [`set_extra_roots`]
    /// for what happens to a root that does not exist.
    pub fn with_roots(project_root: impl Into<PathBuf>, extra: Vec<PathBuf>) -> Self {
        let project_root = project_root.into();
        let canonical_project = project_root.canonicalize().unwrap_or(project_root);
        let workspace = Self {
            roots: Arc::new(RwLock::new(vec![canonical_project])),
        };
        workspace.set_extra_roots(extra);
        workspace
    }

    /// Replaces every root but the project root, and returns the ones that
    /// were **dropped** because they could not be canonicalized (they do not
    /// exist). An unreachable root grants nothing, so the safe reading of a
    /// typo is a narrower workspace rather than a broken session — but it is
    /// returned rather than swallowed, so the caller can tell the developer
    /// that the reach they wrote down is not the reach they have.
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

    /// The first root: the project directory, which relative paths resolve
    /// against.
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
    ///
    /// Synchronous on purpose, though tools call it from async code: it is a
    /// few `canonicalize` calls, microseconds each, and a `spawn_blocking`
    /// hop would cost more than it saves.
    ///
    /// A relative path resolves against the project root. An absolute path is
    /// taken as given — ADR 0004 §5 rejected those outright, because
    /// `PathBuf::join` silently *discards* its base when the joined path is
    /// absolute (`root.join("/etc/passwd")` is `/etc/passwd`, not an error),
    /// and a check against the model's literal argument had no way to see
    /// the escape. With reach checked against canonical roots that
    /// hazard is closed directly, and refusing absolute paths would make a
    /// second root unaddressable.
    ///
    /// **Containment is decided on resolved paths only.** The roots are
    /// canonical, so a purely lexical comparison against the path *as typed*
    /// refuses anything that reaches a root through a symlink — on macOS
    /// that is every `/tmp/…` and `/var/…` path, since both are links into
    /// `/private`. Two resolved forms are checked, and both must be inside:
    ///
    /// 1. The lexically-normalized path with its existing prefix
    ///    canonicalized. This is the form that works for a target that does
    ///    not exist yet (Edit writing a new file), and it is what catches a
    ///    symlink planted *inside* a root pointing out of it — legal in a
    ///    git repo, where symlinks are ordinary blobs.
    /// 2. The path as the *filesystem* resolves it, when it exists. Lexical
    ///    `..` and real `..` disagree after a symlink: with `out -> /else/d`,
    ///    `out/../x` normalizes to `x` but opens `/else/x`.
    ///
    /// The **normalized** path is what is returned and used for I/O, so what
    /// was checked in (1) is what gets opened.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::PathEscapesWorkspace`] when either resolved form
    /// lies outside every root, and [`ToolError::Io`] when the path's
    /// existing prefix cannot be canonicalized.
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

/// Collapses `.`/`..` components without touching the filesystem (the
/// target may not exist yet, e.g. Edit writing a new file).
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
    // — `Workspace` canonicalizes its roots, which requires them to exist.

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
        // Edit writing a brand-new file: `src/` exists, `new.rs` doesn't yet.
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let ws = Workspace::new(root.path());
        assert!(ws.resolve("src/new.rs").is_ok());
    }

    /// The gap ADR 0007 closes: a second declared root is addressable, by
    /// absolute path, from a session rooted elsewhere.
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

    /// Regression test for the symlink-escape gap found in the tools audit:
    /// a symlink lexically inside a root but pointing outside it used to pass
    /// the lexical containment check even though the real I/O it enables
    /// reaches outside the workspace.
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

    /// A symlink that stays inside the workspace must not be rejected as a
    /// false positive — only escaping symlinks are a problem.
    #[test]
    #[cfg(unix)]
    fn a_symlink_inside_a_root_pointing_inside_it_is_allowed() {
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("real")).unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("link")).unwrap();

        let ws = Workspace::new(root.path());
        assert!(ws.resolve("link/x.rs").is_ok());
    }

    /// Same escape, but through a symlink to a not-yet-existing file (the
    /// Edit-writing-a-new-file case).
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

    /// Audit finding: roots are canonical, and the old lexical pre-check
    /// compared them against the path *as typed* — so an absolute path that
    /// reached a root through a symlink was refused. On macOS that is every
    /// `/tmp/…` path.
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

    /// Lexical `..` and real `..` disagree after a symlink. With
    /// `out -> <outside>/d`, `out/../secret` normalizes to `secret` — inside
    /// — but the filesystem opens `<outside>/secret`.
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

    /// `/reload-config` replaces the extra roots through one clone and every
    /// other clone sees it; a root that does not exist is reported back.
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
