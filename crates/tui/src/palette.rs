//! Themeable color palette for `ui.rs` — ported from the Mjolnir Design
//! System (`claude.ai/design`, project "Mjolnir Design System", synced
//! 2026-09-02) rather than hand-picked. Field names mirror the design
//! system's own `--tui-*` semantic tokens (`tokens/semantic.css`), so a
//! value here can be checked directly against that source instead of
//! against another layer of local naming. Two fixed instances — `DARK`
//! (the system's default theme) and `LIGHT` (the system's `.tui-light`
//! scope) — selected once at startup via `Theme::from_config` and carried
//! explicitly from there: `App::theme` for the handful of render functions
//! that already take `&App`, an explicit `pal: &Palette` parameter for the
//! rest (see `ui.rs`'s own module doc comment). Deliberately *not* a
//! global/`OnceLock` — this crate's `cargo test` runs many tests in
//! parallel inside one process, and a shared mutable "current theme" would
//! make one test's theme choice leak into another's; explicit threading
//! keeps every test (and every real render) fully self-contained no matter
//! how it's scheduled. Runtime theme-switching mid-session is supported
//! (`/theme`, see `App::apply_event`'s `ThemeChanged` arm) since `App::theme`
//! is a plain field and `ui::draw` reads it fresh every frame.
//!
//! Colors here are the design system's resolved hex values
//! (`tokens/palette.css`, `tokens/semantic.css`, and the project's own
//! `readme.md` token table) — a terminal needs explicit RGB, so this reads
//! the same values a browser would resolve from the CSS custom properties.
//!
//! # Local deviation, 2026-09-03 — eleven fields are ahead of the tokens
//!
//! `.claude/CLAUDE.md` says the visual design is imported, not invented
//! locally, and that is still the rule. These eleven are a deliberate,
//! temporary exception, made on the developer's explicit direction after
//! the design-system push was blocked, and they must be pushed back into
//! `tokens/palette.css` + `tokens/semantic.css` and then re-synced. Until
//! that happens this file and the token layer disagree, and *this file is
//! the one that is wrong* — the design system is still the source of truth.
//!
//! What moved, and why (WCAG contrast against each theme's own ground;
//! floors are 4.5 for text, 3.0 for non-text):
//!
//! | field | was | now | ratio |
//! |---|---|---|---|
//! | `DARK.label` / `DARK.context` | neutral-600 `#75798c` | neutral-550 `#84889b` | 4.08 → 5.01 |
//! | `DARK.dim` / `DARK.glyph_pending` | neutral-700 `#595d6c` | neutral-650 `#676b7c` | 2.69 → 3.33 |
//! | `DARK.bar_bottom` | `#1b1d2b` | `#1f222f` | 1.05 → 1.11 |
//! | `DARK.diff_box` | `#1a1c29` | `#1e202e` | 1.04 → 1.09 |
//! | `LIGHT.label` / `LIGHT.context` | neutral-600 `#75798c` | neutral-650 `#676b7c` | 3.96 → 4.86 |
//! | `LIGHT.quiet` | neutral-600 `#75798c` | neutral-700 `#595d6c` | 3.96 → 6.02 |
//! | `LIGHT.bar` / `bar_bottom` / `diff_box` | all `#e4e7f5` | `#e2e5f2` / `#e7e9f7` / `#e9ebf9` | one hex → three |
//!
//! `neutral-550` and `neutral-650` are new half-steps, placed midway
//! between their documented neighbours in OKLCH L, C and H, so the ramp
//! keeps its regular spacing and no existing step moves. The two ground
//! moves are at the grounds' own hue (277.5) and chroma (0.026), so the
//! indigo cast is unchanged — only the spacing is. Both sat below every
//! shipping dark UI measured (VS Code 1.09, GitHub 1.09, One Dark 1.10);
//! `DARK.bar` at 1.16 was already above them and is deliberately untouched,
//! which is what keeps `band` reading as a lift above it rather than a hole.
//! The four `_bg` diff tints were re-blended over the new `diff_box`.
//! See mjolnir-tui.md's 2026-09-03 colour-transport entry for the full
//! derivation and for why this is not the same thing as drift.
//! The two `_bg` diff-row tints are the one exception: the source uses CSS
//! `rgba(...)` alpha over the diff box background, which ratatui's `Color`
//! has no runtime alpha-blend for — each is pre-blended by hand over
//! `diff_box` (the surface a diff row actually renders on) and documented
//! with the source rgba + the blend base, so the arithmetic can be checked
//! independently of trusting the hardcoded result.

use ratatui::style::Color;

/// One themeable surface, matching the design system's `--tui-*` roles
/// one-to-one (see this module's doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// Which half of the system this palette is — carried on the palette
    /// itself so anything already holding one (`ui::render_assistant_text`
    /// and the syntax highlighter it calls) can ask, without a second
    /// `Theme` threaded down beside it purely to answer the same question.
    pub theme: Theme,
    /// `--tui-ground` — the frame background every panel ultimately sits on.
    pub ground: Color,
    /// `--tui-bar` — chrome surfaces: the top bar and the decision panel
    /// body.
    pub bar: Color,
    /// `--tui-bar-bottom` — the bottom bar (composer + status line).
    pub bar_bottom: Color,
    /// `--tui-line` — structural one-cell borders/rules between panels.
    pub line: Color,
    /// A flat separator rule, one step more muted than `line` — the design
    /// system's revision log ("Rules") settled freestanding rules (turn
    /// breaks, the rule above permission options, first-run step
    /// separators) as flat single-color rows, one step dimmer than a real
    /// border, not Nocturne's fading-gradient treatment the token layer
    /// still ships (`--rule-fade`) — the revision log states that token is
    /// unused by the five reference screens. A single dim row is also the
    /// natural terminal-cell rendering anyway: "in a terminal that is one
    /// row of the dimmest available colour, so nothing here needs
    /// approximating."
    pub rule: Color,
    /// `--tui-text` — primary text: paths that change, the current row,
    /// the composer draft, the "you" turn's content.
    pub text: Color,
    /// `--tui-body` — agent prose, ordinary secondary content.
    pub body: Color,
    /// `--tui-code` — code text (fenced blocks, inline `` `code` ``).
    pub code: Color,
    /// `--tui-context` — a running tool's stdout.
    pub context: Color,
    /// `--tui-value` — right-flush facts and permission "off" values.
    pub value: Color,
    /// `--tui-label` — muted labels: tool names, `in`/`writes`/`network`,
    /// the label column when not the active row.
    pub label: Color,
    /// `--tui-dim` — the dimmest metadata tier: timestamps, tool result
    /// summaries.
    pub dim: Color,
    /// `--tui-quiet` — quieter than `dim`, used for key-hint verbs and
    /// unmatched permission-pattern text.
    pub quiet: Color,
    /// `--tui-mark` — the accent `▌`/`▶`/caret: session identity is *not*
    /// marked with this per the design system's revision log ("the top bar
    /// carries no accent mark... a pip there indicated nothing") — reserved
    /// for selection, the caret, and the composer prompt.
    pub mark: Color,
    /// `--tui-mark-idle` — an unselected row's `▌`.
    pub mark_idle: Color,
    /// `--tui-band` — the selection band, always paired with `mark` (never
    /// one without the other — see the design system's States section).
    pub band: Color,
    /// `--tui-accent-text` — accent-toned text: a panel title, a filtered
    /// command's highlighted name.
    pub accent_text: Color,
    /// `--tui-speaker-you` — the `you` turn label.
    pub speaker_you: Color,
    /// `--tui-speaker-agent` — the `harness` turn label.
    pub speaker_agent: Color,
    pub gauge_fill: Color,
    pub gauge_track: Color,
    /// `--tui-glyph-done` — a finished tool call (`●`).
    pub glyph_done: Color,
    /// `--tui-glyph-running` — a running tool call / spinner (`◐◓◑◒`).
    pub glyph_running: Color,
    /// `--tui-glyph-pending` — a pending hunk/step (`○`).
    pub glyph_pending: Color,
    pub hunk_header: Color,
    /// `--tui-modal-line` — the decision panel's own top rule.
    pub modal_line: Color,
    /// `--tui-diff-box` — the surface a quoted diff renders on.
    pub diff_box: Color,
    pub add: Color,
    /// `--tui-add-bg` pre-blended over `diff_box` — see this module's doc
    /// comment.
    pub add_bg: Color,
    pub add_code: Color,
    pub del: Color,
    /// `--tui-del-bg` pre-blended over `diff_box` — see this module's doc
    /// comment.
    pub del_bg: Color,
    pub del_code: Color,
}

pub(crate) const DARK: Palette = Palette {
    theme: Theme::Dark,
    ground: Color::Rgb(0x16, 0x18, 0x26),
    bar: Color::Rgb(0x23, 0x25, 0x32),
    bar_bottom: Color::Rgb(0x1f, 0x22, 0x2f), // elev-2 (was #1b1d2b, 1.05 vs ground -> 1.11)
    line: Color::Rgb(0x3f, 0x42, 0x4d),  // neutral-800
    rule: Color::Rgb(0x29, 0x2b, 0x31),  // neutral-900
    text: Color::Rgb(0xe9, 0xe9, 0xed),
    body: Color::Rgb(0xcf, 0xd3, 0xe5),  // neutral-300
    code: Color::Rgb(0xe4, 0xe7, 0xf5),  // neutral-200
    context: Color::Rgb(0x84, 0x88, 0x9b), // neutral-550
    value: Color::Rgb(0xb2, 0xb6, 0xca), // neutral-400
    label: Color::Rgb(0x84, 0x88, 0x9b), // neutral-550
    dim: Color::Rgb(0x67, 0x6b, 0x7c),   // neutral-650
    quiet: Color::Rgb(0x93, 0x97, 0xab), // neutral-500
    mark: Color::Rgb(0x84, 0xae, 0xd9),  // accent
    mark_idle: Color::Rgb(0x3f, 0x42, 0x4d), // neutral-800
    band: Color::Rgb(0x20, 0x2d, 0x39),  // accent-900
    accent_text: Color::Rgb(0xc1, 0xd7, 0xee), // accent-300
    speaker_you: Color::Rgb(0x95, 0xbc, 0xe4),  // accent-400
    speaker_agent: Color::Rgb(0xb2, 0xb6, 0xca), // neutral-400
    gauge_fill: Color::Rgb(0x56, 0x7e, 0xa7),   // accent-600
    gauge_track: Color::Rgb(0x3f, 0x42, 0x4d),  // neutral-800
    glyph_done: Color::Rgb(0x40, 0x61, 0x81),   // accent-700
    glyph_running: Color::Rgb(0x84, 0xae, 0xd9), // accent
    glyph_pending: Color::Rgb(0x67, 0x6b, 0x7c), // neutral-650
    hunk_header: Color::Rgb(0x56, 0x7e, 0xa7),  // accent-600
    modal_line: Color::Rgb(0x40, 0x61, 0x81),   // accent-700
    diff_box: Color::Rgb(0x1e, 0x20, 0x2e), // elev-1 (was #1a1c29, 1.04 vs ground -> 1.09)
    add: Color::Rgb(0x70, 0xcf, 0x75),
    // rgba(112,207,117,.13) over diff_box #1e202e -> #293737.
    add_bg: Color::Rgb(0x29, 0x37, 0x37),
    add_code: Color::Rgb(0xa0, 0xe8, 0xa1),
    del: Color::Rgb(0xe8, 0x6c, 0x68),
    // rgba(232,108,104,.14) over diff_box #1e202e -> #3a2b36
    del_bg: Color::Rgb(0x3a, 0x2b, 0x36),
    del_code: Color::Rgb(0xff, 0x9d, 0x96),
};

/// The design system's `.tui-light` scope — same layout, ramps flipped;
/// selection band darker than the page, not lighter (per the source's own
/// note: "on a light ground the selection band must be darker than the
/// page, not lighter").
pub(crate) const LIGHT: Palette = Palette {
    theme: Theme::Light,
    ground: Color::Rgb(0xf3, 0xf5, 0xfe),  // neutral-100
    bar: Color::Rgb(0xe2, 0xe5, 0xf2),     // elev-3 (was #e4e7f5, shared with the two below)
    bar_bottom: Color::Rgb(0xe7, 0xe9, 0xf7), // elev-2
    line: Color::Rgb(0xcf, 0xd3, 0xe5),    // neutral-300
    rule: Color::Rgb(0xe4, 0xe7, 0xf5),    // neutral-200
    text: Color::Rgb(0x29, 0x2b, 0x31),    // neutral-900
    body: Color::Rgb(0x3f, 0x42, 0x4d),    // neutral-800
    code: Color::Rgb(0x29, 0x2b, 0x31),    // neutral-900
    context: Color::Rgb(0x67, 0x6b, 0x7c), // neutral-650
    value: Color::Rgb(0x59, 0x5d, 0x6c),   // neutral-700
    label: Color::Rgb(0x67, 0x6b, 0x7c),   // neutral-650
    dim: Color::Rgb(0x75, 0x79, 0x8c),     // neutral-600
    quiet: Color::Rgb(0x59, 0x5d, 0x6c),   // neutral-700
    mark: Color::Rgb(0x56, 0x7e, 0xa7),    // accent-600
    mark_idle: Color::Rgb(0xcf, 0xd3, 0xe5), // neutral-300
    band: Color::Rgb(0xc1, 0xd7, 0xee),    // accent-300
    accent_text: Color::Rgb(0x40, 0x61, 0x81), // accent-700
    speaker_you: Color::Rgb(0x40, 0x61, 0x81),  // accent-700
    speaker_agent: Color::Rgb(0x59, 0x5d, 0x6c), // neutral-700
    gauge_fill: Color::Rgb(0x56, 0x7e, 0xa7),   // accent-600
    gauge_track: Color::Rgb(0x93, 0x97, 0xab),  // neutral-500
    glyph_done: Color::Rgb(0x56, 0x7e, 0xa7),   // accent-600
    glyph_running: Color::Rgb(0x56, 0x7e, 0xa7), // accent-600
    glyph_pending: Color::Rgb(0x93, 0x97, 0xab), // neutral-500
    hunk_header: Color::Rgb(0x56, 0x7e, 0xa7),  // accent-600
    modal_line: Color::Rgb(0x56, 0x7e, 0xa7),   // accent-600
    diff_box: Color::Rgb(0xe9, 0xeb, 0xf9),     // elev-1
    add: Color::Rgb(0x0a, 0x75, 0x20),
    // rgba(10,117,32,.16) over diff_box #e9ebf9 -> #c5d8d6
    add_bg: Color::Rgb(0xc5, 0xd8, 0xd6),
    add_code: Color::Rgb(0x09, 0x41, 0x12),
    del: Color::Rgb(0xb3, 0x11, 0x24),
    // rgba(179,17,36,.14) over diff_box #e9ebf9 -> #e1ccdb
    del_bg: Color::Rgb(0xe1, 0xcc, 0xdb),
    del_code: Color::Rgb(0x62, 0x14, 0x17),
};

/// Which fixed `Palette` a session renders with — selected once at startup
/// (`Theme::from_config`, `App::theme`), switchable live via `/theme`; see
/// this module's doc comment for why this isn't a runtime-global.
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

/// How much of its own color the transcript keeps while a decision panel is
/// open — the reference's `opacity:.35` on the conversation column in both
/// panel scenes of `Agent TUI v2.dc.html`. A terminal cell has no alpha
/// channel, so the effect is composited here instead (see `fade`): the same
/// arithmetic the browser does, done ahead of time.
pub(crate) const PANEL_TRANSCRIPT_OPACITY: f32 = 0.35;

/// `fg` composited over `onto` at `alpha` — CSS `opacity` for a medium with
/// no alpha channel. Only `Color::Rgb` blends; anything else (notably
/// `Color::Reset`, which is whatever the terminal itself paints and so has
/// no value to mix) is returned untouched rather than guessed at.
pub(crate) fn fade(fg: Color, onto: Color, alpha: f32) -> Color {
    let (Color::Rgb(fr, fg_, fb), Color::Rgb(br, bg_, bb)) = (fg, onto) else {
        return fg;
    };
    let mix = |f: u8, b: u8| (f as f32 * alpha + b as f32 * (1.0 - alpha)).round() as u8;
    Color::Rgb(mix(fr, br), mix(fg_, bg_), mix(fb, bb))
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
        assert!(luma(LIGHT.ground) > luma(DARK.ground));
        assert!(luma(LIGHT.text) < luma(DARK.text));
    }
}
