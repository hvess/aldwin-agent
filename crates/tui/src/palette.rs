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
/// theme maps that ANSI slot to. The tint itself is `BG_ELEMENT` below —
/// message bubbles are one of that scale's "element" surfaces, not a
/// separately hand-tuned color.
pub(crate) const USER_FG: Color = Color::Rgb(190, 190, 195);

/// Three-tier neutral background scale for the opaque-surfaces redesign —
/// values are OpenCode's own tuned dark-theme `darkStep1/2/3`
/// (github.com/anomalyco/opencode, `theme/assets/opencode.json`), reused
/// verbatim rather than re-derived, since ratatui has no runtime
/// alpha-compositing to recompute them from. Each tier is one step lighter
/// than the one "behind" it, so nesting reads as depth without needing more
/// border: `BG_BASE` fills the whole frame (see `ui::draw`) so gaps between
/// panels are opaque instead of the terminal's own background; `BG_PANEL`
/// fills the log panel and sidebar; `BG_ELEMENT` fills the "card" surfaces —
/// the chat input, user message bubbles, and approval/prompt cards. Same
/// fixed-RGB-not-named-ANSI reasoning as every other color in this file.
pub(crate) const BG_BASE: Color = Color::Rgb(10, 10, 10);
pub(crate) const BG_PANEL: Color = Color::Rgb(20, 20, 20);
pub(crate) const BG_ELEMENT: Color = Color::Rgb(30, 30, 30);

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

/// Chrome color for the log panel, sidebar panel, and (while a card or
/// prompt is pending) the dimmed input panel border. A muted, desaturated
/// tint of `ACCENT`'s own hue — not `DIM` gray — per a UX pass modeled on
/// darrenburns/posting (a Textual TUI whose polish comes substantially from
/// exactly this discipline: every panel border carries the app's one accent
/// hue at reduced intensity — Textual's `.section { border: round $accent
/// 40%; &:focus-within { border: round $accent 100%; } }` — rather than
/// switching between an unrelated neutral gray and the accent). Ratatui has
/// no runtime alpha-blend-over-background primitive, so this is a
/// hand-picked fixed RGB approximating "cyan at ~35% intensity over a near-
/// black background" rather than a computed blend. This *refines* rather
/// than reopens the "one accent, not scattered" rule below: it's still a
/// single hue family used consistently for all chrome, never a second
/// unrelated color — `ACCENT` itself stays reserved for the moments that
/// should visually outrank ordinary chrome (the approval/prompt card, the
/// focused input border, and the log panel's own live/scrolled title badge
/// — see `ui::draw`).
pub(crate) const PANEL_BORDER: Color = Color::Rgb(45, 82, 87);

/// A small fixed set of hues for giving each distinct tool *name* a stable,
/// repeatable color in the sidebar's running-tools list — modeled on
/// posting's per-HTTP-method color coding (`method-get`/`method-post`/etc.,
/// each a distinct hue so a scan of a request list reads categories at a
/// glance without reading the text). `ui::tool_color` picks one of these
/// deterministically from the tool's name (a stable hash, not an
/// incrementing counter, so the same tool name always gets the same color
/// across draws/sessions without needing to track an assignment table).
/// Deliberately excludes `ACCENT`/`WARNING_FG`/the diff colors — those
/// already carry specific meaning (focus/attention, retry, add/remove)
/// elsewhere, and reusing them here would blur that meaning.
pub(crate) const TOOL_PALETTE: [Color; 6] =
    [Color::Rgb(122, 162, 247), Color::Rgb(158, 206, 106), Color::Rgb(224, 138, 90), Color::Rgb(187, 154, 247), Color::Rgb(125, 207, 255), Color::Rgb(247, 118, 142)];
