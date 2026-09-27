/// Persistence tier on disk. Not every domain exists at every scope —
/// `tui` is global-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The project's own `.aldwin/`, at the project root.
    Project,
    /// The developer's `~/.aldwin/`, shared by every project.
    Global,
}
