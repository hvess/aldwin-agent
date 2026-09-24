//! What bare `/resume` offers: the past sessions, as display halves handed
//! in by aldwin-cli's bootstrap. aldwin-tui reads no files, which is why
//! the transcript directory is not mentioned anywhere in this crate. The
//! question itself is asked with the one list control (`App::open_session_question`),
//! and answering it submits `/resume <id>` exactly as if it had been typed,
//! so the interceptor stays the one place that knows what resuming does.

/// One past session, already rendered for display — this crate never sees
/// a timestamp or a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionChoice {
    /// What `/resume <id>` is composed with.
    pub id: String,
    /// First user message, one line — what the developer will recognise the
    /// session by.
    pub title: String,
    /// When it started, already formatted (`2026-09-20 18:11`).
    pub when: String,
    pub turns: usize,
}
