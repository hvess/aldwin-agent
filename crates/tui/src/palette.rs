//! Themeable color palette for `ui.rs`. Two fixed instances — `DARK` (the
//! only palette that existed before 2026-09-02, and still the default) and
//! `LIGHT` — selected once at startup via `Theme::from_config` and carried
//! explicitly from there: `App::theme` for the handful of render functions
//! that already take `&App`, an explicit `pal: &Palette` parameter for the
//! rest (see `ui.rs`'s own module doc comment). Deliberately *not* a global/
//! `OnceLock` — this crate's `cargo test` runs many tests in parallel inside
//! one process, and a shared mutable "current theme" would make one test's
//! theme choice leak into another's; explicit threading keeps every test
//! (and every real render) fully self-contained no matter how it's
//! scheduled. Runtime theme-switching mid-session was never asked for and
//! isn't supported — `tui.yaml`'s `theme` field is read once at session
//! start (mjolnir-config's own "TUI preferences" are already documented as
//! that kind of setting).
//!
//! `DARK`'s values are extracted verbatim (including field-level doc
//! comments — they encode real developer-decision history, not filler)
//! from `ui.rs`'s former top-of-file `const` block; no `DARK` value changed
//! in this module's introduction except `DIM`/`BRIGHT`, already fixed
//! straight to RGB in the 2026-09-02 Solarized-Light incident (see
//! mjolnir-tui.md's Progress notes of the same name) before `LIGHT` existed
//! at all.
//!
//! `LIGHT` is new, added directly at developer request once that same
//! incident's fix (fixed-RGB `DIM`/`BRIGHT`, immune to terminal-palette
//! remapping) was shipped and the developer asked for an actual light
//! *theme*, not just a dark theme that no longer breaks under a light
//! terminal profile. Verified the same way every prior palette change in
//! this file was — a real xterm session (via Xvfb, no tmux — see this
//! session's own transcript) rendering the real built app, screenshotted
//! before/after — not hand-waved from the RGB values alone.

use ratatui::style::Color;

/// One themeable surface. Field names mirror the old flat `const` names
/// (lowercased) so `ui.rs`'s call sites read the same as before, just as
/// `pal.dim` / `pal.bright` instead of bare `DIM` / `BRIGHT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// Bright sky-blue in `DARK` rather than named ANSI `Cyan` — sampled
    /// directly (via ImageMagick pixel-sampling, not eyeballed) from a real
    /// OpenCode screenshot the developer pointed at as the actual reference
    /// after the first pass at the visual redesign (fixed-RGB values
    /// guessed from OpenCode's theme *source* rather than measured from a
    /// rendered screenshot) came back rejected as "horrific." Applied to the
    /// approval card border, focused-input highlight, the log panel's live/
    /// scrolled status badge, and the welcome banner's mascot art/wordmark —
    /// deliberately *not* widened to ordinary panel borders (`panel_border`
    /// carries those), keeping accent meaning "this needs your attention"
    /// rather than "this is a panel."
    pub accent: Color,
    /// Secondary text tier: tool metadata, header/footer text, a slash
    /// command as user input. Fixed RGB, not a named ANSI color — see this
    /// module's own doc comment on why that was a live bug, not just an
    /// inconsistency.
    pub dim: Color,
    /// Primary text tier: assistant output (bold earned via markdown, not
    /// blanket-applied), headings, ordinary prose. Fixed RGB for the same
    /// reason as `dim`.
    pub bright: Color,
    /// Plain user chat-bubble text. A dedicated LightGreen was tried first
    /// for user/assistant separation but read as too loud against real
    /// terminal color schemes — swapped for a muted tone plus a subtle
    /// background tint (`bg_element`), which separates user input from both
    /// assistant text (`bright`, no bg) and dim metadata without fighting
    /// the terminal's own palette.
    pub user_fg: Color,
    /// Background scale for the opaque-surfaces redesign, one step lighter
    /// each: `bg_base` fills the whole frame and is what the log panel's own
    /// scrollback content sits directly on; `bg_element` fills message-
    /// bubble and approval/prompt cards; `bg_input` fills the chat input —
    /// the *lightest* of the tiers in `DARK`, since the input is the one
    /// surface that's always active/focused rather than passive content.
    /// `DARK`'s values are pixel-sampled (not guessed) from a real rendered
    /// OpenCode reference screenshot, confirming an indigo-slate family
    /// (blue-shifted, not neutral gray) rather than the flat neutral scale
    /// tried first and rejected as "horrific" (imperceptible steps).
    pub bg_base: Color,
    pub bg_element: Color,
    pub bg_input: Color,
    /// Inline `` `code` `` in assistant prose. Used `Modifier::REVERSED`
    /// originally, which read as a jarring bright-white block against most
    /// terminal themes — swapped for a plain distinguishing color instead.
    pub code_fg: Color,
    /// Background for a fenced fixed-width code block. Deliberately the
    /// *same dark* value in both `DARK` and `LIGHT` — a code block reading
    /// as "a real code block in a document" (its own distinct dark box with
    /// a language label, per the redesign that introduced it) is a common,
    /// expected pattern independent of the surrounding app's own theme (many
    /// light-themed editors and renderers keep code blocks dark), and it
    /// means the syntax-highlighted text inside it (`highlight.rs`'s
    /// `base16-ocean.dark` syntect theme) never has to change per app theme
    /// either — one less thing to keep in sync, not a limitation.
    pub code_bg: Color,
    /// Approval-card diff coloring: a full-width background tint behind
    /// added/removed lines so a diff reads at a glance instead of every line
    /// rendering in the same plain `bright`.
    pub diff_add_bg: Color,
    pub diff_add_fg: Color,
    pub diff_del_bg: Color,
    pub diff_del_fg: Color,
    /// Retry/warning entries — previously shared plain `dim`, giving them no
    /// more visual weight than routine tool-activity metadata even though a
    /// retry is worth noticing.
    pub warning_fg: Color,
    /// A muted, desaturated tint of `accent`'s own hue — not `dim` gray —
    /// for chrome that shouldn't outrank `accent` itself: a card's left
    /// accent bar at rest, diff context lines, and the log panel's own
    /// scrollbar. Not a *border* color in the literal 4-sided sense — the
    /// log panel and former sidebar dropped their drawn borders in the same
    /// pass that introduced the `bg_*` opaque-surface scale.
    pub panel_border: Color,
    /// A small fixed set of hues for giving each distinct tool *name* a
    /// stable, repeatable color in the status line's running-tools list —
    /// modeled on posting's per-HTTP-method color coding. `ui::tool_color`
    /// picks one of these deterministically from the tool's name (a stable
    /// hash, not an incrementing counter). Deliberately excludes `accent`/
    /// `warning_fg`/the diff colors — those already carry specific meaning
    /// elsewhere, and reusing them here would blur that meaning.
    pub tool_palette: [Color; 6],
}

pub(crate) const DARK: Palette = Palette {
    accent: Color::Rgb(125, 207, 255),
    // Fixed RGB since 2026-09-02 (previously `Color::DarkGray`/`Color::White`
    // — named ANSI indices 8/15, remapped by a real light-mode terminal
    // theme for *its own* readability; confirmed directly with Solarized
    // Light's published 16-color table, which renders index 8 as near-black
    // navy. See mjolnir-tui.md's matching Progress note for the full
    // incident — reported as "text is dark on light mode and it clashes
    // with the dark background.")
    dim: Color::Rgb(140, 143, 163),
    bright: Color::Rgb(232, 232, 238),
    user_fg: Color::Rgb(190, 190, 195),
    bg_base: Color::Rgb(34, 36, 53),
    bg_element: Color::Rgb(47, 49, 72),
    bg_input: Color::Rgb(54, 56, 83),
    code_fg: Color::Rgb(224, 175, 104),
    code_bg: Color::Rgb(22, 23, 35),
    diff_add_bg: Color::Rgb(28, 46, 30),
    diff_add_fg: Color::Rgb(150, 210, 160),
    diff_del_bg: Color::Rgb(48, 28, 28),
    diff_del_fg: Color::Rgb(220, 150, 150),
    warning_fg: Color::Rgb(212, 163, 60),
    panel_border: Color::Rgb(45, 82, 87),
    tool_palette: [
        Color::Rgb(122, 162, 247),
        Color::Rgb(158, 206, 106),
        Color::Rgb(224, 138, 90),
        Color::Rgb(187, 154, 247),
        Color::Rgb(125, 207, 255),
        Color::Rgb(247, 118, 142),
    ],
};

// Light-tinted mirror of `DARK`'s indigo-slate background scale — a light
// lavender-white family (still blue-shifted, not neutral gray, keeping the
// same hue identity `DARK`'s own doc comment describes) rather than flat
// white, and every foreground/accent color re-tuned for contrast against
// *that* scale specifically, not just "the opposite of dark." `code_bg`
// stays `DARK`'s own dark value on purpose — see the field's own doc
// comment above. Verified via a real xterm render (Xvfb, not `TestBackend`
// alone) before shipping, same discipline as every other palette change in
// this file's history.
pub(crate) const LIGHT: Palette = Palette {
    accent: Color::Rgb(15, 111, 178),
    dim: Color::Rgb(117, 120, 138),
    bright: Color::Rgb(28, 30, 44),
    user_fg: Color::Rgb(70, 73, 92),
    bg_base: Color::Rgb(246, 247, 250),
    bg_element: Color::Rgb(233, 235, 243),
    bg_input: Color::Rgb(255, 255, 255),
    code_fg: Color::Rgb(146, 97, 18),
    code_bg: Color::Rgb(22, 23, 35),
    diff_add_bg: Color::Rgb(222, 242, 226),
    diff_add_fg: Color::Rgb(24, 107, 42),
    diff_del_bg: Color::Rgb(250, 226, 226),
    diff_del_fg: Color::Rgb(153, 32, 32),
    warning_fg: Color::Rgb(158, 106, 8),
    panel_border: Color::Rgb(188, 210, 227),
    tool_palette: [
        Color::Rgb(36, 92, 199),
        Color::Rgb(56, 126, 33),
        Color::Rgb(175, 89, 15),
        Color::Rgb(112, 65, 191),
        Color::Rgb(15, 130, 168),
        Color::Rgb(191, 40, 90),
    ],
};

/// Which fixed `Palette` a session renders with — selected once at startup
/// (`Theme::from_config`, `App::theme`) and never afterward; see this
/// module's own doc comment for why this isn't a runtime-global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub(crate) fn palette(self) -> &'static Palette {
        match self {
            Theme::Dark => &DARK,
            Theme::Light => &LIGHT,
        }
    }

    /// Parses `tui.yaml`'s `theme` field (`mjolnir_config::TuiConfig::theme`)
    /// — case-insensitive `"light"` selects `Light`; `None`, `"dark"`, or
    /// anything unrecognized selects `Dark`, the long-standing default. An
    /// unrecognized value doesn't refuse to start: consistent with this
    /// being a purely cosmetic setting, a typo shouldn't block the session
    /// the way a malformed permissions or provider file does. `pub`, not
    /// `pub(crate)` — `mjolnir-cli`'s bootstrap calls this to resolve
    /// `Config::global_tui().theme` before constructing the TUI's `App`.
    pub fn from_config(theme: Option<&str>) -> Theme {
        match theme {
            Some(s) if s.eq_ignore_ascii_case("light") => Theme::Light,
            _ => Theme::Dark,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_selects_light_case_insensitively() {
        assert_eq!(Theme::from_config(Some("light")), Theme::Light);
        assert_eq!(Theme::from_config(Some("Light")), Theme::Light);
        assert_eq!(Theme::from_config(Some("LIGHT")), Theme::Light);
    }

    #[test]
    fn from_config_defaults_to_dark_for_none_dark_or_garbage() {
        assert_eq!(Theme::from_config(None), Theme::Dark);
        assert_eq!(Theme::from_config(Some("dark")), Theme::Dark);
        assert_eq!(Theme::from_config(Some("nonsense")), Theme::Dark);
        assert_eq!(Theme::from_config(Some("")), Theme::Dark);
    }

    #[test]
    fn theme_default_is_dark() {
        assert_eq!(Theme::default(), Theme::Dark);
    }

    #[test]
    fn dark_and_light_are_distinct_palettes() {
        assert_ne!(DARK, LIGHT);
    }

    /// Not a full perceptual-contrast checker — just the cheap, real
    /// invariant a light/dark pair must satisfy: `LIGHT`'s background is
    /// lighter than `DARK`'s, and `LIGHT`'s primary text is darker than
    /// `DARK`'s, using luma as the same "lightness" proxy for both.
    #[test]
    fn light_background_is_lighter_and_light_text_is_darker_than_dark() {
        fn luma(c: Color) -> u32 {
            match c {
                Color::Rgb(r, g, b) => r as u32 * 3 + g as u32 * 6 + b as u32,
                other => panic!("expected an Rgb color, got {other:?}"),
            }
        }
        assert!(luma(LIGHT.bg_base) > luma(DARK.bg_base));
        assert!(luma(LIGHT.bright) < luma(DARK.bright));
    }
}
