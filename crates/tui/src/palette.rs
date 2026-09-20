//! Themeable color palette for `ui.rs` — ported from the Mjolnir Design
//! System (`claude.ai/design`, synced 2026-09-07 from the bound copy in the
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
//! step's lightness removes a boundary."
//!
//! The seven roles that read those steps are [`Palette::scrim`],
//! [`Palette::recess`], [`Palette::break_`], [`Palette::ground`],
//! [`Palette::bar_bottom`], [`Palette::bar`] and [`Palette::panel_title`].
//! **The two themes do not order them the same way**, which is the thing to
//! know before changing one. In [`DARK`] that list *is* the ladder, darkest
//! to lightest — it reads as `--color-ground-0…6` in order. In [`LIGHT`]
//! the ground is the lightest surface and every other band sinks below it,
//! so the ladder is a different sequence entirely: `ground`, `bar_bottom`,
//! `bar`, `break_`, `recess`, `panel_title`, `scrim` — the order of
//! `--color-ground-light-0…6`, which Turn 15 renumbered strictly by
//! lightness. A reading of the light theme as "the dark list reversed" is
//! wrong and was the shape of two of the defects Turn 14 fixed.
//!
//! That is also why the previous local contrast deviation is gone. Eleven
//! fields here used to sit ahead of the tokens, hand-nudged for WCAG
//! headroom because the hand-tuned palette kept failing at the dim steps.
//! The upstream ramps are now *generated* — one hue (300°), lightness
//! climbing in even OKLCH steps, chroma falling as lightness rises — which
//! is the systematic fix that deviation was standing in for. Both ladders
//! are strictly monotonic with seven distinct rungs, and the light one's
//! narrowest rung (1.037:1, bar to break) is still wider than the dark
//! one's (1.011:1, scrim to recess); `dim` holds 5.14:1 on the light
//! theme's recessed field, the darkest band inside a frame and the binding
//! one there. Every value below is the token's own, so this file and the
//! token layer agree again.
//!
//! Turn 15 made that light ladder *shallower* on purpose. Nothing in the
//! previous one failed a contrast floor; it read as harsh beside the dark
//! theme anyway, because it spanned 19:1 from ink to ground where the dark
//! theme separates bands by a step and lets the ends stay soft. The light
//! rungs now sit within 10% of each other and hierarchy is carried by the
//! step between them rather than by the distance to the ends — so a "this
//! looks low-contrast, nudge it" edit here is undoing a decision, not
//! fixing an oversight.
//!
//! There is deliberately no `line` field. `--tui-line` still exists
//! upstream but is marked legacy there and scoped to "annotation around a
//! frame"; its last consumer here was the transcript scrollbar, which the
//! design lists under "Deliberately absent" and which has now been removed.
//! `cells.css` states the rule this follows: "If a token here is not applied
//! through a `var()` somewhere, delete it rather than document it."
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

/// The two palettes, **generated** from `.claude/design/tokens/` into
/// [`crate::tokens`] and re-exported here so every call site keeps reading
/// `palette::DARK`.
///
/// They used to be written out by hand in this file, with a `// neutral-200`
/// comment beside each value as the only thing linking them to the design
/// system. That is a transcription, and a transcription drifts. The review
/// loop's stage 3 regenerates the file and fails if the result differs, so
/// the app's palette and the imported design cannot disagree — the first
/// generation reproduced all eighty-four hand-written values exactly, which
/// is the evidence that the transcription had been kept honest until now and
/// no reason to keep doing it by hand.
pub(crate) use crate::tokens::{DARK, LIGHT};

// What the light half of that generated file is, and why it looks the way
// it does — kept because it is history the generator cannot carry:
//
// The design system's `.tui-light` scope — same roles, same hue, ramps
// flipped; selection band darker than the page, not lighter (per the
// source's own note: "on a light ground the selection band must be darker
// than the page, not lighter").
//
// Turn 15 regenerated every value here for the second time. The change is
// *depth*, not hue: the theme now spans `#241f2b` ink to a `#f7f5fa`
// ground where Turn 14 ran `#0e0c12` to `#faf7ff` with a `#a39fac` desk,
// and the seven ground rungs sit inside 10% of each other. See this
// module's doc comment for why that shallowness is the decision rather
// than an oversight.
//
// The light values also stopped being literals upstream. Turn 14 wrote
// thirty hexes into `.tui-light` on the argument that the light accents
// had outgrown `--color-accent-900`; Turn 15 replaced that with light
// ramps of their own — `--color-ground-light-*`, `--color-ink-light-*`,
// `--color-accent-light-*`, `--color-neutral-light-*` and
// `--color-diff-light-*` — so every field below can name its rung the way
// the [`DARK`] fields do.
//
// Both palettes are generated now; see the re-export above.

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
    /// `--tui-step-done` — a *settled first-run step's* `●`, which is not the
    /// same role as [`Palette::glyph_done`] even though the two carry the
    /// same value in [`DARK`].
    ///
    /// They part company in [`LIGHT`], and the reason is worth keeping: both
    /// glyphs have to recede from the accent mark, but they recede in
    /// opposite directions. A finished tool call sits in a dense run of tool
    /// rows and falls back by going *lighter* than the mark
    /// (`--color-accent-light-300`, `#7a58ae`); a settled step sits alone
    /// beside the answer it produced, which has to stay readable, so it
    /// falls back by going *darker* (`--color-accent-light-600`, `#5f3aa0`).
    /// Spelling both as one field would force one of the two to be wrong.
    ///
    /// Turn 15 is where the values finally match that sentence. Turn 14 put
    /// both glyphs *above* the mark — `step_done` was `#6941a1` against a
    /// `#4b1f7e` mark — so "recedes by going darker" was prose the palette
    /// contradicted. The mark is now `#6b3fb0` and `step_done` sits a rung
    /// below it.
    pub step_done: Color,
    pub hunk_header: Color,
    /// `--tui-syn-keyword` — a keyword, a storage modifier, a language
    /// constant.
    ///
    /// These five `syn_*` roles are the one place the system carries more
    /// than one hue on purpose: keyword keeps the system's own 300°, string
    /// *is* the diff green so a literal and an added line agree, and the
    /// other three take 265°, 195° and 75°. Two rules keep that from
    /// becoming a second theme, and both are the design system's:
    ///
    /// * **No syntax role may outrank the accent mark.** They sit at one
    ///   lightness per theme, at the palette's own chroma. A keyword is a
    ///   colour a line carries, not a mark the eye is meant to jump to.
    /// * **Five roles, no more.** Everything else a highlighter would happily
    ///   colour — identifiers, parameters, operators, punctuation, macros
    ///   that are not call names — stays [`Palette::code`], and a comment
    ///   drops to [`Palette::dim`]. An identifier is not a category.
    ///
    /// `highlight.rs` builds a syntect theme from these rather than loading
    /// one, so a fenced block cannot introduce a colour the system never
    /// chose; `every_highlighted_colour_is_one_of_the_seven_roles` pins it.
    pub syn_keyword: Color,
    /// `--tui-syn-call` — the *name* in a call or definition, not the call
    /// expression around it. See [`Palette::syn_keyword`] for the rules.
    pub syn_call: Color,
    /// `--tui-syn-type` — a named type, class, struct, enum or trait. See
    /// [`Palette::syn_keyword`].
    pub syn_type: Color,
    /// `--tui-syn-string` — a string literal, and whatever a syntax nests
    /// inside one (escapes, interpolation placeholders), so a quoted run
    /// reads as one thing. See [`Palette::syn_keyword`].
    pub syn_string: Color,
    /// `--tui-syn-number` — a numeric literal. See [`Palette::syn_keyword`].
    pub syn_number: Color,
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



/// Which fixed `Palette` a session renders with — selected once at startup
/// (`Theme::from_config`, `App::theme`), switchable live via `/theme`; see
/// this module's doc comment for why this isn't a runtime-global.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
/// open — the reference's `opacity:.45` on the conversation column in both
/// panel scenes (`5a` and `5c`) of `Agent TUI v2.dc.html`. A terminal cell
/// has no alpha channel, so the effect is composited here instead (see
/// `fade`): the same arithmetic the browser does, done ahead of time.
///
/// Was `.35` until Turn 14 raised it. The dimmed transcript is the only
/// thing behind a panel and it has to stay readable enough to be worth
/// leaving on screen.
pub(crate) const PANEL_TRANSCRIPT_OPACITY: f32 = 0.45;

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
        assert!(luma(LIGHT.ground) > luma(DARK.ground));
        assert!(luma(LIGHT.text) < luma(DARK.text));
    }

    fn luma(c: Color) -> u32 {
        match c {
            Color::Rgb(r, g, b) => r as u32 * 3 + g as u32 * 6 + b as u32,
            other => panic!("expected an Rgb color, got {other:?}"),
        }
    }

    /// The ground ladder is the *only* thing separating one band from its
    /// neighbour — nothing inside a frame is stroked — so its seven rungs
    /// have to be seven distinct, strictly ordered steps in each theme. A
    /// duplicate or an out-of-order pair silently deletes a boundary, which
    /// is a defect no rendering test catches: the frame still draws, it just
    /// stops having an edge where it needs one.
    ///
    /// The two orders differ, and that is the point of testing both. The
    /// dark ladder is `--color-ground-0…6` climbing; the light one is
    /// `--color-ground-light-0…6` sinking from the frame ground, which is
    /// *not* the dark list reversed — `break_` is the fourth rung there and
    /// the second here. Reading it as if it were is what put `bar` above
    /// `bar_bottom` and `break_` above `ground` in the light theme before
    /// Turn 14.
    #[test]
    fn both_ground_ladders_are_strictly_ordered_and_have_no_repeated_rung() {
        let dark = [DARK.scrim, DARK.recess, DARK.break_, DARK.ground, DARK.bar_bottom, DARK.bar, DARK.panel_title];
        let light =
            [LIGHT.ground, LIGHT.bar_bottom, LIGHT.bar, LIGHT.break_, LIGHT.recess, LIGHT.panel_title, LIGHT.scrim];

        for pair in dark.windows(2) {
            assert!(luma(pair[0]) < luma(pair[1]), "the dark ladder climbs: {:?} then {:?}", pair[0], pair[1]);
        }
        for pair in light.windows(2) {
            assert!(luma(pair[0]) > luma(pair[1]), "the light ladder sinks from the ground: {:?} then {:?}", pair[0], pair[1]);
        }
        // Seven distinct rungs needs no separate assertion: a strict
        // ordering by luma already rules out two bands sharing a tone.
    }

    /// The light theme's chrome bands must sit the same way round as the
    /// dark theme's: the top bar is a step *further* from the transcript
    /// ground than the composer is. They were inverted in light until Turn
    /// 14, so the two themes disagreed about which bar was which.
    #[test]
    fn the_top_bar_is_further_from_the_ground_than_the_composer_in_both_themes() {
        assert!(luma(DARK.bar) > luma(DARK.bar_bottom), "dark: both rise, the top bar higher");
        assert!(luma(LIGHT.bar) < luma(LIGHT.bar_bottom), "light: both sink, the top bar lower");
    }

    /// `step_done` and `glyph_done` are one value in the dark theme and two
    /// in the light one, on purpose — see [`Palette::step_done`]. Pinned
    /// because the light pair looks like a copy-paste slip and the dark pair
    /// looks like a redundant field; each guards the other from being
    /// "tidied" away.
    ///
    /// The light assertion is that the two straddle the mark: the tool glyph
    /// recedes above it, the settled step below it. Turn 14 had both above,
    /// which agreed with neither the design system's prose nor its intent;
    /// Turn 15's `--color-accent-light-*` ramp put them where the prose says.
    #[test]
    fn the_two_done_glyph_roles_coincide_in_dark_and_straddle_the_mark_in_light() {
        assert_eq!(DARK.step_done, DARK.glyph_done, "both are accent-700 in the dark theme");
        assert_ne!(LIGHT.step_done, LIGHT.glyph_done, "they recede in opposite directions on a light ground");
        assert!(luma(LIGHT.glyph_done) > luma(LIGHT.mark), "a finished tool call recedes by going lighter than the mark");
        assert!(luma(LIGHT.step_done) < luma(LIGHT.mark), "a settled step recedes by going darker than the mark");
    }
}
