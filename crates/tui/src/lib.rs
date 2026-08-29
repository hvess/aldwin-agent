//! ratatui frontend rendering core's event stream. See
//! `.claude/spec/amundsen-tui.md`.
//!
//! `App` (`app.rs`) holds all state and the pure event/key handling logic —
//! unit-tested without a terminal. `ui.rs` renders it (tested against
//! `ratatui::backend::TestBackend`). `run.rs` is the thin, effectively
//! untestable glue: multiplexing crossterm input and core events onto one
//! `tokio::select!` and driving the terminal.
//!
//! Two things from amundsen-tui.md's Pitfalls not fully addressed: tool-
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
mod run;
mod scroll;
mod ui;

pub use app::App;
pub use log::{LogEntry, ToolActivityEntry, ToolActivityStatus, TurnEndReasonKind};
pub use run::run;
pub use scroll::ScrollState;
