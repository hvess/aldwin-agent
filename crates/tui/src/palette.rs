//! Centralized color constants for `ui.rs`. Extracted verbatim (including
//! doc comments — they encode real developer-decision history, not filler)
//! from `ui.rs`'s former top-of-file `const` block, purely to keep that file
//! from growing indefinitely as the redesign adds header/footer/sidebar
//! rendering. No color *values* changed in this extraction; see the
//! `WARNING_FG`/`PANEL_BORDER` additions below for the only new constants.

use ratatui::style::Color;

pub(crate) const ACCENT: Color = Color::Cyan;
pub(crate) const DIM: Color = Color::DarkGray;
pub(crate) const BRIGHT: Color = Color::White;

/// A dedicated LightGreen was tried first for user/assistant separation
/// (see the git history) but read as too loud against real terminal color
/// schemes, per explicit developer feedback — swapped for a muted gray text
/// color plus a subtle background tint, which separates user input from
/// both assistant text (BRIGHT, no bg) and dim metadata without fighting
/// the terminal's own palette. Fixed RGB rather than a named ANSI color so
/// the "subtle" tint doesn't get reinterpreted by whatever the terminal
/// theme maps that ANSI slot to.
pub(crate) const USER_FG: Color = Color::Rgb(190, 190, 195);
pub(crate) const USER_BG: Color = Color::Rgb(40, 40, 46);

/// Inline `` `code` `` in assistant prose used `Modifier::REVERSED` (fg/bg
/// swapped) to stand out, which reads as a jarring bright-white block
/// against most terminal themes — per explicit developer feedback, swapped
/// for a plain distinguishing color, same fixed-RGB-not-named-ANSI
/// reasoning as `USER_FG`/`USER_BG` above.
pub(crate) const CODE_FG: Color = Color::Rgb(224, 175, 104);

/// Approval-card diff coloring: a full-width background tint (same
/// technique as `USER_BG`) behind added/removed lines so a diff reads at a
/// glance instead of every line rendering in the same plain `BRIGHT`.
pub(crate) const DIFF_ADD_BG: Color = Color::Rgb(28, 46, 30);
pub(crate) const DIFF_ADD_FG: Color = Color::Rgb(150, 210, 160);
pub(crate) const DIFF_DEL_BG: Color = Color::Rgb(48, 28, 28);
pub(crate) const DIFF_DEL_FG: Color = Color::Rgb(220, 150, 150);

/// New for the visual redesign — retry/warning entries previously shared
/// plain `DIM`, giving them no more visual weight than routine tool-activity
/// metadata even though a retry is worth noticing. Fixed RGB, same
/// not-reinterpreted-by-terminal-theme reasoning as every other fixed-RGB
/// constant here.
pub(crate) const WARNING_FG: Color = Color::Rgb(212, 163, 60);

/// Neutral chrome color for the log panel, sidebar panel, and (while a card
/// or prompt is pending) the dimmed input panel border. Deliberately an
/// alias for `DIM` rather than a new hue — introduced only so "this is panel
/// chrome" reads as an intentional choice at call sites, not because the
/// underlying color needed to differ from ordinary dim text. Keeps `ACCENT`
/// scoped to the approval/prompt card and the focused input border only, per
/// the "one accent, not scattered" rule.
pub(crate) const PANEL_BORDER: Color = DIM;
