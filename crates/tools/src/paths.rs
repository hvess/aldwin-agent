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
pub fn resolve_in_project(project_root: &Path, path_str: &str) -> Result<PathBuf, ToolError> {
    let candidate = Path::new(path_str);
    if candidate.is_absolute() {
        return Err(ToolError::PathEscapesProject { path: path_str.to_string() });
    }

    let joined = project_root.join(candidate);
    if !normalize_lexically(&joined).starts_with(normalize_lexically(project_root)) {
        return Err(ToolError::PathEscapesProject { path: path_str.to_string() });
    }
    Ok(joined)
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

    #[test]
    fn ordinary_relative_paths_resolve_under_the_root() {
        let root = Path::new("/home/user/project");
        let resolved = resolve_in_project(root, "src/main.rs").unwrap();
        assert_eq!(resolved, Path::new("/home/user/project/src/main.rs"));
    }

    #[test]
    fn absolute_paths_are_rejected_outright() {
        let root = Path::new("/home/user/project");
        let err = resolve_in_project(root, "/etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesProject { .. }));
    }

    #[test]
    fn dot_dot_climbing_above_the_root_is_rejected() {
        let root = Path::new("/home/user/project");
        let err = resolve_in_project(root, "../../etc/passwd").unwrap_err();
        assert!(matches!(err, ToolError::PathEscapesProject { .. }));
    }

    #[test]
    fn dot_dot_that_stays_inside_the_root_is_allowed() {
        let root = Path::new("/home/user/project");
        let resolved = resolve_in_project(root, "src/../src/main.rs").unwrap();
        assert_eq!(resolved, Path::new("/home/user/project/src/../src/main.rs"));
    }
}
