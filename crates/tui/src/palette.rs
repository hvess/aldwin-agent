//! Themeable color palette for `ui.rs` — ported from the Mjolnir Design
//! System (`claude.ai/design`, synced 2026-09-06 from the bound copy in the
//! "Design system tokens discussion" project, which is the current one —
//! see `.claude/design/IMPORT.md`) rather than hand-picked. Field names
//! mirror the design
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
//! # The ground ladder carries every boundary
//!
//! Nothing inside a frame is stroked any more. The design system's Turn 13
//! rebuild removed every rule, pane divider and box outline and made a
//! band's *tone* the thing that separates it from its neighbour, so the
//! seven `--color-ground-0…6` steps are load-bearing structure rather than
//! decoration. `palette.css` states the consequence directly: "Changing a
//! step's lightness removes a boundary." The roles that read those steps
//! are [`Palette::scrim`], [`Palette::recess`], [`Palette::break_`],
//! [`Palette::ground`], [`Palette::bar_bottom`], [`Palette::bar`] and
//! [`Palette::panel_title`], in ascending order in the dark theme and
//! descending in the light one.
//!
//! That is also why the previous local contrast deviation is gone. Eleven
//! fields here used to sit ahead of the tokens, hand-nudged for WCAG
//! headroom because the hand-tuned palette kept failing at the dim steps.
//! The upstream ramps are now *generated* — one hue (300°), lightness
//! climbing in even OKLCH steps, chroma falling as lightness rises — which
//! is the systematic fix that deviation was standing in for, and the
//! measurements are recorded upstream (the muted ramp clears 3.3:1 at the
//! dim step on the chrome bar, the binding band in a dark theme; the light
//! theme clears 3.0:1 on its recessed field). Every value below is now the
//! token's own, so this file and the token layer agree again.
//!
//! # Why there is no alpha blending here any more
//!
//! A diff row's fill used to be a CSS `rgba(...)` tint that had to be
//! pre-blended by hand over `diff_box`, since ratatui's `Color` has no
//! runtime alpha. The system now ships a *resolved solid* for exactly this
//! case — `--tui-add-row` / `--tui-del-row` alongside the `-bg` tints — so
//! the row fills below are the tokens' own opaque hexes and no arithmetic
//! stands between the source and this file. The `-bg` rgba tints have no
//! terminal rendering and are deliberately not carried.

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
    /// `--tui-bar-bottom` — the bottom bar (composer + status line), one
    /// step *above* the ground in the dark theme.
    pub bar_bottom: Color,
    /// `--tui-recess` — a field sunk below the frame ground: the inline
    /// diff, the permission command block, the review file pane, and the
    /// one-row separator above the permission options.
    pub recess: Color,
    /// `--tui-break` — the one-row band that separates transcript turns and
    /// first-run steps. This *replaces* the flat rule glyph: the design
    /// system's Turn 13 rebuild settled separators as "a full row of a
    /// different ground, never a rule", and notes that in a terminal that is
    /// "a single `Style::bg` on a one-row rect, so nothing here needs
    /// approximating".
    ///
    /// Named with a trailing underscore only because `break` is a Rust
    /// keyword; it mirrors `--tui-break` one-to-one like every other field.
    pub break_: Color,
    /// `--tui-panel-title` — an overlay panel's title row, the top of the
    /// ground ladder. A *lift*, not a well: the design system moved this
    /// off the accent field (which read as a filled accent band and broke
    /// the "accent is a mark, never a field" rule) and then off the
    /// recessed tone (which made a header the darkest strip in the frame).
    pub panel_title: Color,
    /// `--tui-scrim` — the desk outside the terminal window. Unused by a
    /// real terminal, which has no outside, but carried so the palette
    /// stays one-to-one with the token layer and the snapshot fixtures can
    /// render a framed scene.
    pub scrim: Color,
    /// `--tui-line` — legacy. Nothing inside a frame is stroked any more
    /// (see this module's doc comment), so this survives for the one thing
    /// the design system has no token for: the transcript scrollbar, which
    /// is terminal chrome rather than part of a designed frame.
    pub line: Color,
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
    /// `--tui-reverse-bg` / `--tui-reverse-ink` — reverse video, the accent
    /// as ground with the desk as ink. The design system uses this for the
    /// wordmark; it is the one place the accent is allowed to be a filled
    /// field.
    pub reverse_bg: Color,
    pub reverse_ink: Color,
    /// `--tui-diff-box` — the surface a quoted diff renders on.
    pub diff_box: Color,
    pub add: Color,
    /// `--tui-add-row` — the resolved solid fill of an added row. Not the
    /// `--tui-add-bg` rgba tint, which has no terminal rendering; see this
    /// module's doc comment.
    pub add_row: Color,
    pub add_code: Color,
    pub del: Color,
    /// `--tui-del-row` — the resolved solid fill of a removed row.
    pub del_row: Color,
    pub del_code: Color,
}

pub(crate) const DARK: Palette = Palette {
    theme: Theme::Dark,
    // The ground ladder, darkest to lightest: --color-ground-0…6. These
    // seven are the frame's entire structure now that nothing is stroked.
    scrim: Color::Rgb(0x0c, 0x0a, 0x11),       // ground-0
    recess: Color::Rgb(0x0f, 0x0b, 0x15),      // ground-1
    break_: Color::Rgb(0x1e, 0x1a, 0x26),      // ground-2
    ground: Color::Rgb(0x27, 0x23, 0x2f),      // ground-3
    bar_bottom: Color::Rgb(0x36, 0x31, 0x3f),  // ground-4
    bar: Color::Rgb(0x47, 0x42, 0x51),         // ground-5
    panel_title: Color::Rgb(0x5d, 0x57, 0x6b), // ground-6
    line: Color::Rgb(0x47, 0x42, 0x51),        // neutral-800 (legacy)
    text: Color::Rgb(0xf4, 0xf2, 0xf9),        // neutral-100
    body: Color::Rgb(0xe3, 0xdf, 0xeb),        // neutral-200
    code: Color::Rgb(0xec, 0xe9, 0xf3),
    context: Color::Rgb(0x9a, 0x95, 0xa4),     // neutral-500
    value: Color::Rgb(0xc9, 0xc5, 0xd2),       // neutral-300
    label: Color::Rgb(0xb1, 0xad, 0xbb),       // neutral-400
    dim: Color::Rgb(0x9a, 0x95, 0xa4),         // neutral-500
    quiet: Color::Rgb(0xc9, 0xc5, 0xd2),       // neutral-300
    mark: Color::Rgb(0xbe, 0x9d, 0xf7),        // accent-400
    mark_idle: Color::Rgb(0x5d, 0x57, 0x6a),   // neutral-700
    band: Color::Rgb(0x60, 0x47, 0x88),        // band-dark
    accent_text: Color::Rgb(0xdf, 0xd1, 0xfb), // accent-200
    speaker_you: Color::Rgb(0xce, 0xb6, 0xfb), // accent-300
    speaker_agent: Color::Rgb(0xc9, 0xc5, 0xd2), // neutral-300
    gauge_fill: Color::Rgb(0xa0, 0x81, 0xd5),  // accent-500
    gauge_track: Color::Rgb(0x60, 0x5a, 0x6c),
    glyph_done: Color::Rgb(0x7f, 0x64, 0xab),    // accent-700
    glyph_running: Color::Rgb(0xbe, 0x9d, 0xf7), // accent-400
    glyph_pending: Color::Rgb(0x5d, 0x57, 0x6a), // neutral-700
    hunk_header: Color::Rgb(0xa0, 0x81, 0xd5),   // accent-500
    reverse_bg: Color::Rgb(0xbe, 0x9d, 0xf7),    // accent-400
    reverse_ink: Color::Rgb(0x0c, 0x0a, 0x11),   // ground-0
    diff_box: Color::Rgb(0x3a, 0x36, 0x48),      // diff-ground
    add: Color::Rgb(0x5e, 0xd4, 0x76),
    add_row: Color::Rgb(0x3d, 0x4b, 0x42),
    add_code: Color::Rgb(0x9c, 0xea, 0xa7),
    del: Color::Rgb(0xf6, 0x6d, 0x67),
    del_row: Color::Rgb(0x4b, 0x3a, 0x42),
    del_code: Color::Rgb(0xff, 0xa8, 0xa0),
};

/// The design system's `.tui-light` scope — same layout, ramps flipped;
/// selection band darker than the page, not lighter (per the source's own
/// note: "on a light ground the selection band must be darker than the
/// page, not lighter").
pub(crate) const LIGHT: Palette = Palette {
    theme: Theme::Light,
    // The ladder inverts: the frame ground is the *lightest* surface and
    // every other band sinks below it. The one exception is `break_`, which
    // rises — a separator has to stay visible against the ground it parts.
    scrim: Color::Rgb(0xae, 0xaa, 0xb7),
    recess: Color::Rgb(0xbb, 0xb6, 0xc5),
    break_: Color::Rgb(0xf0, 0xed, 0xf5),
    ground: Color::Rgb(0xfb, 0xf9, 0xfe),
    bar_bottom: Color::Rgb(0xce, 0xc9, 0xd8),
    bar: Color::Rgb(0xe4, 0xe1, 0xeb),
    panel_title: Color::Rgb(0xce, 0xc6, 0xdf),
    line: Color::Rgb(0xc9, 0xc5, 0xd2), // neutral-300 (legacy)
    text: Color::Rgb(0x17, 0x15, 0x1c),
    body: Color::Rgb(0x32, 0x2e, 0x39),
    code: Color::Rgb(0x22, 0x1f, 0x28),
    context: Color::Rgb(0x61, 0x5c, 0x6c),
    value: Color::Rgb(0x43, 0x3f, 0x4b),
    label: Color::Rgb(0x52, 0x4d, 0x5b),
    dim: Color::Rgb(0x61, 0x5c, 0x6c),
    quiet: Color::Rgb(0x43, 0x3f, 0x4b),
    mark: Color::Rgb(0x6d, 0x41, 0xa9),      // accent-800
    mark_idle: Color::Rgb(0x96, 0x90, 0xa3),
    band: Color::Rgb(0xc6, 0xb1, 0xef),      // band-light
    accent_text: Color::Rgb(0x56, 0x2c, 0x8b), // accent-900
    speaker_you: Color::Rgb(0x5c, 0x30, 0x93),
    speaker_agent: Color::Rgb(0x43, 0x3f, 0x4b),
    gauge_fill: Color::Rgb(0x6d, 0x41, 0xa9), // accent-800
    gauge_track: Color::Rgb(0x93, 0x8d, 0xa0),
    glyph_done: Color::Rgb(0x56, 0x2c, 0x8b),    // accent-900
    glyph_running: Color::Rgb(0x6d, 0x41, 0xa9), // accent-800
    glyph_pending: Color::Rgb(0x96, 0x90, 0xa3),
    hunk_header: Color::Rgb(0x6d, 0x41, 0xa9), // accent-800
    reverse_bg: Color::Rgb(0x6d, 0x41, 0xa9),  // accent-800
    reverse_ink: Color::Rgb(0xfb, 0xf9, 0xfe),
    diff_box: Color::Rgb(0xec, 0xe8, 0xf3),
    add: Color::Rgb(0x00, 0x73, 0x24),
    add_row: Color::Rgb(0xd9, 0xe4, 0xd9),
    add_code: Color::Rgb(0x00, 0x49, 0x14),
    del: Color::Rgb(0xb0, 0x0a, 0x1d),
    del_row: Color::Rgb(0xe9, 0xd7, 0xda),
    del_code: Color::Rgb(0x69, 0x04, 0x0d),
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
