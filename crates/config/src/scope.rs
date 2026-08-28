/// Persistence tier on disk. Not every domain exists at every scope —
/// `context_files` is project-only, `tui` is global-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Project,
    Global,
}
