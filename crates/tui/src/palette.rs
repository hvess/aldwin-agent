//! Centralized color constants for `ui.rs`. Extracted verbatim (including
//! doc comments — they encode real developer-decision history, not filler)
//! from `ui.rs`'s former top-of-file `const` block, purely to keep that file
//! from growing indefinitely as the redesign adds header/footer/sidebar
//! rendering. No color *values* changed in this extraction; see the
//! `WARNING_FG`/`PANEL_BORDER` additions below for the only new constants.

use ratatui::style::Color;

/// Bright sky-blue rather than named ANSI `Cyan` — sampled directly (via
/// ImageMagick pixel-sampling, not eyeballed) from a real OpenCode
/// screenshot the developer pointed at as the actual reference after the
/// first pass at this redesign (fixed-RGB values guessed from OpenCode's
/// theme *source* rather than measured from a rendered screenshot) came
/// back rejected as "horrific" — too low-contrast and the wrong hue
/// entirely. Reused verbatim from `TOOL_PALETTE`'s existing sky-blue entry
/// rather than introduced as a fourth near-identical blue.
pub(crate) const ACCENT: Color = Color::Rgb(125, 207, 255);

/// `DIM`/`BRIGHT` were the one exception to this file's own "fixed RGB, not
/// a named ANSI color" rule (see `USER_FG`'s comment right below, which
/// states the rule explicitly) — left as `Color::DarkGray`/`Color::White`
/// since this file's text hierarchy predates the opaque-background redesign
/// (`BG_BASE` etc.) that made the app paint its own always-dark surfaces
/// regardless of the developer's terminal theme. That made the gap a live
/// bug, not just an inconsistency: `Color::DarkGray`/`Color::White` are ANSI
/// palette indices 8/15, which a real light-mode terminal theme remaps for
/// *its own* readability against a light background — Solarized Light, a
/// widely-used real scheme, maps index 8 to `#002b36` (near-black navy).
/// Confirmed directly: an xterm session with Solarized Light's actual 16-
/// color table rendered every `DIM` span (status-line metadata, timestamps,
/// dim labels) as near-invisible dark-navy-on-this-app's-own-dark-navy —
/// `BRIGHT` happened to survive in that specific scheme (Solarized's index
/// 15 is a light cream) but was exposed to the identical failure mode by
/// construction, just not tripped by that one example palette. Reported
/// directly as "text is dark on light mode and it clashes with the dark
/// background." Fixed by giving both fixed RGB values, same as every other
/// constant in this file — chosen to read clearly against the `BG_BASE`/
/// `BG_ELEMENT`/`BG_INPUT` dark-navy family regardless of any terminal
/// palette, verified against the same reproducing Solarized Light xterm
/// session before/after.
pub(crate) const DIM: Color = Color::Rgb(140, 143, 163);
pub(crate) const BRIGHT: Color = Color::Rgb(232, 232, 238);

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

/// Background scale for the opaque-surfaces redesign. The first attempt
/// used a *neutral* near-black scale (10/10/10 → 20/20/20 → 30/30/30) taken
/// from OpenCode's theme JSON *source* — rejected by the developer as
/// "horrific": a 10-unit neutral-gray step is essentially imperceptible on
/// a real screen, so panels and cards didn't read as distinct filled
/// surfaces at all, just as one flat dark mass. These values are pixel-
/// sampled instead (ImageMagick `convert … -format "%[pixel:p{x,y}]"`,
/// exact coordinates and readings kept in the session transcript, not
/// re-derivable from this file alone) from a real rendered OpenCode
/// screenshot the developer linked as the actual reference. Two things the
/// neutral scale missed, both confirmed by the samples: the steps are
/// larger (~7-10 units *per channel*, not ~10 total) and every tier is
/// blue-shifted (B noticeably higher than R/G, not R=G=B) rather than
/// neutral gray — an indigo-slate family, not a black-and-white one. Each
/// tier is one step lighter than the one "behind" it: `BG_BASE` fills the
/// whole frame (see `ui::draw`) and is also what the log panel's own
/// scrollback content sits directly on (confirmed by sampling *between*
/// message cards in the reference — it's the same color as the outer
/// frame, not a separate panel tint); `BG_ELEMENT` fills message-bubble and
/// approval/prompt cards; `BG_INPUT` fills the chat input — sampled as the
/// *lightest* of the tiers, one step past `BG_ELEMENT`, since the input is
/// the one surface that's always active/focused rather than passive
/// content. A fourth tier, `BG_PANEL`, filled the sidebar panel this scale
/// was originally sampled for — removed along with the sidebar itself in
/// the 2026-08-31 status-line correction (see `mjolnir-tui.md`'s Progress
/// note of the same name); the other three tiers are unchanged.
pub(crate) const BG_BASE: Color = Color::Rgb(34, 36, 53);
pub(crate) const BG_ELEMENT: Color = Color::Rgb(47, 49, 72);
pub(crate) const BG_INPUT: Color = Color::Rgb(54, 56, 83);

/// Inline `` `code` `` in assistant prose used `Modifier::REVERSED` (fg/bg
/// swapped) to stand out, which reads as a jarring bright-white block
/// against most terminal themes — per explicit developer feedback, swapped
/// for a plain distinguishing color, same fixed-RGB-not-named-ANSI
/// reasoning as `USER_FG`/`USER_BG` above.
pub(crate) const CODE_FG: Color = Color::Rgb(224, 175, 104);

/// Background for a fenced fixed-width code block — its own darker tier,
/// not `BG_ELEMENT` (chat bubbles and cards) — per explicit developer
/// feedback that a code block should look like "a real code block in a
/// document," a distinct dark box with a language label, not the hand-drawn
/// `╭─ lang` / `│ ` / `╰─` ASCII border it replaces (see
/// `ui::render_assistant_text`). Darker than every other tier so a code
/// block still reads as its own surface even nested inside the assistant
/// message's own `BG_ELEMENT` bubble.
pub(crate) const CODE_BG: Color = Color::Rgb(22, 23, 35);

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

/// A muted, desaturated tint of `ACCENT`'s own hue — not `DIM` gray — for
/// chrome that shouldn't outrank `ACCENT` itself: a card's left accent bar
/// at rest (see `card_line`/`LogEntry::UserMessage`), diff context lines,
/// and the log panel's own scrollbar. No longer a *border* color in the
/// literal sense — the log panel and (former) sidebar dropped their drawn
/// 4-sided borders in the same pass that replaced `BG_BASE`/`BG_ELEMENT`
/// above: the reference screenshot that prompted that replacement also
/// showed no box anywhere around the conversation, only filled cards
/// floating directly on the frame background, and keeping our own outer
/// panel border was producing a messy doubled line everywhere a card's own
/// left bar met it. The name stays (and the constant itself is unchanged)
/// since it's still doing the same "one accent hue, reduced intensity" job
/// the original comment described, just applied to bars now rather than
/// 4-sided outlines.
pub(crate) const PANEL_BORDER: Color = Color::Rgb(45, 82, 87);

/// A small fixed set of hues for giving each distinct tool *name* a stable,
/// repeatable color in the status line's running-tools list — modeled on
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
