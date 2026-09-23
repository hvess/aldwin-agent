//! ratatui frontend rendering core's event stream. See
//! `.claude/spec/aldwin-tui.md`.
//!
//! `App` (`app.rs`) holds all state and the pure event/key handling logic —
//! unit-tested without a terminal. `ui/` renders it: `ui/mod.rs` owns the
//! two screens' band layout and nothing else, and each of its submodules
//! owns one job (see that module's own doc comment for the table).
//! `review.rs` is the review's state and `list.rs` the one list control
//! every question is asked with. `run.rs` is the thin, effectively
//! untestable glue: multiplexing crossterm input and core events onto one
//! `tokio::select!` and driving the terminal.
//!
//! Rendering is verified two ways. `ui/tests.rs` asserts frame-level facts
//! against `ratatui::backend::TestBackend` — why each thing is where it is.
//! `tests/render_snapshot.rs` pins every cell, colour and modifier of every
//! scene at four frame sizes in both themes, so a refactor that was meant
//! to preserve output can be shown to have done so.
//!
//! Shift+Enter does not rely on the terminal happening to report it:
//! `run.rs` pushes the Kitty keyboard protocol's disambiguation flag
//! unconditionally, which is what makes the key distinguishable from Enter
//! at all where the terminal implements it, and `App::handle_key` carries
//! Alt+Enter and Ctrl+J as fallbacks for terminals that don't. `draft.rs`
//! owns the multi-line draft those keys produce, along with the bracketed
//! pastes that produce much larger ones.

mod app;
mod draft;
mod list;
mod log;
mod palette;
mod resume;
mod review;
mod run;
mod scroll;
mod tokens;
mod ui;
mod version;

pub use app::{App, Asker, Asking, CommandMenu, Mode, ModelChoice, ProviderChoice, StatusInfo, COMMANDS};
pub use list::{List, ListRow};
pub use log::{LogEntry, WorkItem};
pub use palette::Theme;
pub use resume::SessionChoice;
pub use review::Review;
pub use run::{run, SessionProvider};
pub use scroll::ScrollState;
pub use version::{GIT_HASH, VERSION, VERSION_FULL};

/// Exposed only for `examples/` and `tests/render_snapshot.rs` — the
/// harnesses that seed an `App` and draw it once outside the normal
/// core/channel wiring. Not part of the supported public API.
#[doc(hidden)]
pub use ui::draw as __preview_draw;

/// The design system as data, for the conformance tests in
/// `tests/render_snapshot.rs`: every colour the app may paint in a theme.
/// `tokens.rs` is generated from `.claude/design/tokens/`, so asserting
/// against these values *is* asserting against the design.
#[doc(hidden)]
pub fn __design_palette(theme: Theme) -> &'static [ratatui::style::Color] {
    match theme {
        Theme::Dark => &tokens::DARK_VALUES,
        Theme::Light => &tokens::LIGHT_VALUES,
    }
}

/// The closed glyph table, and the glyphs a recorded design contradiction
/// licenses on top of it — see `crates/review/baseline.json`.
#[doc(hidden)]
pub fn __design_glyphs() -> (&'static [char], &'static [char]) {
    (&tokens::MARKS, &tokens::MARKS_BY_EXCEPTION)
}
