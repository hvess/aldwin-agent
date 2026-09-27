//! The past sessions bare `/resume` offers, formatted by aldwin-cli; this
//! crate reads no files. Picking one (`App::open_session_question`) submits
//! `/resume <id>` as if typed, so the interceptor alone knows what resuming
//! does.

/// One past session, already formatted for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionChoice {
    /// What `/resume <id>` is composed with.
    pub id: String,
    /// First user message, one line.
    pub title: String,
    /// When it started, already formatted (`2026-09-20 18:11`).
    pub when: String,
    /// Turns it ran.
    pub turns: usize,
}
