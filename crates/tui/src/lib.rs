//! ratatui frontend rendering core's event stream. See
//! `.claude/spec/aldwin-tui.md`.
//!
//! `App` (`app.rs`) holds all state and the pure event/key handling logic —
//! unit-tested without a terminal. `ui/` renders it: `ui/mod.rs` owns the
//! frame's band layout and nothing else, and each of its submodules owns
//! one job (see that module's own doc comment for the table). `run.rs` is
//! the thin, effectively untestable glue: multiplexing crossterm input and
//! core events onto one `tokio::select!` and driving the terminal.
//!
//! Rendering is verified two ways. `ui/tests.rs` asserts frame-level facts
//! against `ratatui::backend::TestBackend` — why each thing is where it is.
//! `tests/render_snapshot.rs` pins every cell, colour and modifier of every
//! scene at four frame sizes in both themes, so a refactor that was meant
//! to preserve output can be shown to have done so.
//!
//! One thing from aldwin-tui.md's Pitfalls is not fully addressed: tool-
//! activity groups render each call as one bounded summary line (via
//! `log::summarise`) rather than literally collapsing after a timed delay —
//! this bounds flooding without needing a redraw timer, but isn't the
//! spec's literal mechanism.
//!
//! Shift+Enter no longer relies on the terminal happening to report it:
//! `run.rs` asks for the Kitty keyboard protocol's disambiguation flag
//! where the terminal says it supports it, which is what makes the key
//! distinguishable from Enter at all, and `App::handle_key` carries
//! Alt+Enter and Ctrl+J as fallbacks for terminals that don't. `draft.rs`
//! owns the multi-line draft those keys produce, along with the bracketed
//! pastes that produce much larger ones.

mod app;
mod draft;
mod first_run;
mod highlight;
mod log;
mod palette;
mod picker;
mod resume;
mod run;
mod scroll;
mod tokens;
mod ui;
mod version;

pub use app::App;
pub use log::{LogEntry, PromptResolution, ToolActivityEntry, ToolActivityStatus, TurnEndReasonKind};
pub use first_run::{run as run_first_run, AccessTier, Answers as FirstRunAnswers, Configured, ModelChoice, ProviderChoice};
pub use palette::Theme;
pub use resume::SessionChoice;
pub use run::{run, SessionProvider};
pub use scroll::ScrollState;
pub use version::{GIT_HASH, VERSION, VERSION_FULL};

/// Exposed only for `examples/preview.rs` — the design-iteration harness
/// that seeds an `App` with representative `LogEntry`s and draws it once
/// outside the normal core/channel wiring. Not part of the supported public
/// API.
#[doc(hidden)]
pub use app::{PendingApproval as __PreviewPendingApproval, PendingPrompt as __PreviewPendingPrompt, RunningTool as __PreviewRunningTool};
#[doc(hidden)]
pub use ui::draw as __preview_draw;
/// The desk a fixture draws its frame on — `--tui-scrim` for that theme.
///
/// `scrim` is the one palette role a real terminal has no use for (a
/// terminal has no outside), so the HTML fixtures are its only consumer and
/// it would otherwise be reachable only from inside the crate. It was a
/// literal in `examples/snapshot.rs` until an audit caught the literal
/// still carrying the Turn 14 light desk `#a39fac` two turns after the
/// token moved to `#cfcad9` — with a comment claiming it was "the same
/// value `Palette::scrim` carries". Reading the palette is what makes that
/// claim true.
#[doc(hidden)]
pub fn __preview_scrim_hex(theme: Theme) -> String {
    match theme.palette().scrim {
        ratatui::style::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => unreachable!("scrim is always an Rgb palette value, got {other:?}"),
    }
}

/// The design system as data, for the conformance tests in
/// `tests/render_snapshot.rs`.
///
/// Those tests assert that every cell the app paints carries a colour from
/// the design and a glyph from its closed table. They ran against a real
/// terminal until 2026-09-20, through the review harness's capture stack,
/// which meant the check cost a compositor, a subprocess and 2m45s and was
/// not hermetic. The cells a `TestBackend` buffer holds are the same
/// declared cells, so the check moved here and the terminal is now only
/// needed for the pictures a human or a judge looks at.
///
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

/// The permission panel's stated band height, from `cells.css`.
#[doc(hidden)]
pub fn __design_panel_rows() -> usize {
    tokens::PANEL_PERMISSION_H
}

/// Same, for the first-run screen. It runs its own terminal loop
/// (`first_run::run`) rather than being a mode inside `App`, so the
/// snapshot harness cannot reach it through `__preview_draw` and needs the
/// screen's own draw plus the state type it takes.
#[doc(hidden)]
pub use first_run::FirstRun as __PreviewFirstRun;
#[doc(hidden)]
pub fn __preview_draw_first_run(frame: &mut ratatui::Frame, state: &first_run::FirstRun, theme: Theme) {
    ui::first_run::draw(frame, state, theme.palette());
}
