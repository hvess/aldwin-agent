//! ratatui frontend rendering core's event stream. See
//! `.claude/spec/mjolnir-tui.md`.
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
//! Two things from mjolnir-tui.md's Pitfalls not fully addressed: tool-
//! activity groups render each call as one bounded summary line (via
//! `log::summarise`) rather than literally collapsing after a timed delay —
//! this bounds flooding without needing a redraw timer, but isn't the
//! spec's literal mechanism. And Shift+Enter's terminal-dependence couldn't
//! be verified against real kitty/iTerm2/xterm sessions in this sandbox
//! (no terminal to attach to) — Ctrl+J is wired as a fallback, but that's
//! reasoning about the failure mode, not empirical testing under those
//! terminals.

mod app;
mod highlight;
mod log;
mod palette;
mod run;
mod scroll;
mod ui;

pub use app::App;
pub use log::{LogEntry, ToolActivityEntry, ToolActivityStatus, TurnEndReasonKind};
pub use palette::Theme;
pub use run::run;
pub use scroll::ScrollState;

/// Exposed only for `examples/preview.rs` — the design-iteration harness
/// that seeds an `App` with representative `LogEntry`s and draws it once
/// outside the normal core/channel wiring. Not part of the supported public
/// API.
#[doc(hidden)]
pub use app::{PendingApproval as __PreviewPendingApproval, PendingPrompt as __PreviewPendingPrompt, RunningTool as __PreviewRunningTool};
#[doc(hidden)]
pub use ui::draw as __preview_draw;
