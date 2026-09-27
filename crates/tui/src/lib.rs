//! ratatui frontend rendering core's event stream. Spec:
//! `.claude/spec/aldwin-tui.md`.
//!
//! `App` (`app.rs`) holds all state and the key/event logic, tested without
//! a terminal; `ui/` renders it; `run.rs` is the untested glue that drives
//! the terminal. `ui/tests.rs` asserts frame-level facts;
//! `tests/render_snapshot.rs` pins every cell of every scene.
//!
//! Shift+Enter needs the Kitty disambiguation flag `run.rs` pushes;
//! `App::handle_key` keeps Alt+Enter and Ctrl+J as fallbacks.

mod app;
mod draft;
mod list;
mod log;
mod palette;
mod resume;
mod review;
mod run;
mod scroll;
// The flat palettes and glyph table are read only by `design_palette` and
// `design_glyphs`; the app draws through the named roles and marks.
#[cfg_attr(not(feature = "test-util"), allow(dead_code))]
mod tokens;
mod ui;
mod version;

pub use app::{CommandChoice, ModelChoice, ProviderChoice};
pub use palette::Theme;
pub use resume::SessionChoice;
pub use run::{run, SessionProvider};
pub use version::VERSION_FULL;

/// What `tests/render_snapshot.rs` and `examples/preview.rs` seed and draw
/// a scene with. `test-util` only; aldwin-cli never sees these.
#[cfg(feature = "test-util")]
pub use {
    app::{App, StatusInfo},
    log::{LogEntry, Verb, WorkItem},
    review::Review,
    ui::draw,
};

/// Every colour the app may paint in `theme`, generated from
/// `.claude/design/tokens/`; for `tests/render_snapshot.rs`.
#[cfg(feature = "test-util")]
pub fn design_palette(theme: Theme) -> &'static [ratatui::style::Color] {
    match theme {
        Theme::Dark => &tokens::DARK_VALUES,
        Theme::Light => &tokens::LIGHT_VALUES,
    }
}

/// The closed glyph table, and the glyphs licensed by exception (ADR 0002,
/// `crates/review/baseline.json`).
#[cfg(feature = "test-util")]
pub fn design_glyphs() -> (&'static [char], &'static [char]) {
    (&tokens::MARKS, &tokens::MARKS_BY_EXCEPTION)
}
