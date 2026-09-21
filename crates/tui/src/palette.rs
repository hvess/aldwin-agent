//! Themeable color palette for `ui.rs` — ported from the Aldwin Design
//! System (`claude.ai/design`, synced 2026-09-21 from the bound copy in the
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
//! # What the system is now
//!
//! Near-neutral warm grey for grounds and ink, **one** brand colour —
//! lantern gold, `#e9c46a` — four status hues, and two syntax hues. It
//! replaced the single-hue (300°) OKLCH system on 2026-09-21; every value
//! changed, and so did the argument. That system got its coherence from
//! everything sharing a hue. This one gets it from almost nothing having
//! one: the greys carry "only a hint of warmth and no brown", so the gold is
//! the only thing in a resting frame the eye reads as a colour at all.
//!
//! Three rules follow, and each is the design's own sentence:
//!
//! * **Gold is spent on one thing per band.** The open step's `▌`, the
//!   selected row's band and `▌`, the prompt `▸` and caret, a running
//!   spinner, the `you` label — "and never on a fill larger than the
//!   wordmark". A finished `●` is *not* gold: [`Palette::done`] is a
//!   neutral, "readable, below the mark". Under the old system that glyph
//!   was an accent step, so this is the rule most likely to be undone by
//!   habit.
//! * **A status hue appears only when something is a status**, each paired
//!   with its glyph — [`Palette::ok`] `✓`, [`Palette::err`] `✗`,
//!   [`Palette::warn`] `!`, [`Palette::info`] `·` — "and nothing else in a
//!   frame borrows them". A diff sign is a status, which is why `add` and
//!   `ok` are one sage and `del` and `err` one rose.
//! * **No status hue appears inside a code block.** See
//!   [`Palette::syn_keyword`].
//!
//! # The ground ladder carries every boundary
//!
//! Nothing inside a frame is stroked, so a band's *tone* is the only thing
//! separating it from its neighbour and the seven ground steps are
//! load-bearing structure rather than decoration.
//!
//! The ladder is named from the frame ground outward: `--color-ground-0` is
//! the ground, `up-1…3` step toward the raised bands, `down-1…3` toward the
//! sunk ones. Darkest to lightest, the roles read [`Palette::scrim`],
//! [`Palette::recess`], [`Palette::break_`], [`Palette::ground`],
//! [`Palette::bar_bottom`], [`Palette::bar`], [`Palette::panel_title`].
//!
//! **Both themes now run the same way round**, which they did not before:
//! a raised band is *lighter* than the ground in light as well as in dark,
//! where the previous light theme sank every band below a near-white page.
//! The single exception is light's [`Palette::panel_title`], which
//! `palette.css` calls "the one raised band darker than ground, because it
//! is a title" — it sits between `break_` and `recess` there. So the light
//! ladder is the dark one with exactly one rung moved, and a reading of it
//! as "the dark list reversed" is wrong in six places rather than one.
//!
//! The steps are narrow on purpose — the dark ladder's tightest pair is
//! 1.014:1, scrim to recess — and hierarchy is carried by the step between
//! rungs rather than by the distance to the ends. A "this looks
//! low-contrast, nudge it" edit here is undoing a decision, not fixing an
//! oversight; and it cannot be made here anyway, because this file's values
//! are generated.
//!
//! There is deliberately no `border` field. `--tui-border` exists upstream
//! but `semantic.css` scopes it to "the one quiet border, outside frames",
//! and a terminal has no outside. The generator lists it as uncarried so a
//! stroke colour is never within reach of code that must not draw one.
//!
//! # Why there is no alpha blending here at all
//!
//! Two things used to be composited by hand because ratatui's `Color` has no
//! runtime alpha, and the design has since resolved both into opaque roles:
//!
//! * A diff row's fill was an `rgba()` tint pre-blended over `diff_box`. It
//!   is now `--tui-add-row` / `--tui-del-row`, the tokens' own solid hexes.
//!   The `-bg` tints have no terminal rendering and are not carried.
//! * The transcript behind an open panel was faded at `opacity:.45`. It is
//!   now recoloured to [`Palette::scrim_text`], [`Palette::scrim_quiet`] and
//!   [`Palette::scrim_mark`] — `semantic.css`: "a recolour, never alpha". See
//!   [`Palette::scrimmed`].

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
    /// `--tui-bar` — chrome surfaces: the top bar and an overlay panel's
    /// body. Two steps above the ground.
    pub bar: Color,
    /// `--tui-bar-bottom` — the bottom band (composer + status line), one
    /// step above the ground in both themes.
    pub bar_bottom: Color,
    /// `--tui-recess` — "a field the user reads from", sunk two steps below
    /// the ground: the review file pane and the command list.
    ///
    /// Quoted code is *not* here. Spans, blocks and the inline diff all sit
    /// on [`Palette::diff_box`], which is a raised tone — the design moved
    /// them off the recess because "the recess swallows the ramp".
    pub recess: Color,
    /// `--tui-break` — the one-row band that separates transcript turns,
    /// one step *below* the ground: "the one row that sinks below the
    /// transcript". A full row of a different ground, never a rule — in a
    /// terminal that is a single `Style::bg` on a one-row rect, so nothing
    /// here needs approximating.
    ///
    /// Named with a trailing underscore only because `break` is a Rust
    /// keyword; it mirrors `--tui-break` one-to-one like every other field.
    pub break_: Color,
    /// `--tui-panel-title` — an overlay panel's or review's title row. A
    /// *lift*, not a well. The top of the dark ladder; in light it is the
    /// one raised band darker than the ground — see this module's doc
    /// comment.
    pub panel_title: Color,
    /// `--tui-desk` — outside the terminal window. Unused by a real
    /// terminal, which has no outside, but carried so the palette stays
    /// one-to-one with the token layer and the snapshot fixtures can render
    /// a framed scene.
    pub desk: Color,
    /// `--tui-scrim` — "the dimmed layer behind an overlay". The same rung
    /// as [`Palette::desk`] today, and a separate role because the two mean
    /// different things: one is where the window is not, the other is what
    /// a transcript sinks toward while a panel holds the floor.
    pub scrim: Color,
    /// `--tui-scrim-text` — prose and paths in a dimmed transcript. See
    /// [`Palette::scrimmed`].
    pub scrim_text: Color,
    /// `--tui-scrim-quiet` — tool names and facts in a dimmed transcript.
    pub scrim_quiet: Color,
    /// `--tui-scrim-mark` — glyphs and speaker labels in a dimmed transcript.
    pub scrim_mark: Color,
    /// `--tui-text` — primary text: paths that change, the current row,
    /// the composer draft, the "you" turn's content.
    pub text: Color,
    /// `--tui-body` — agent prose, ordinary secondary content.
    pub body: Color,
    /// `--tui-code` — code text (fenced blocks, inline `` `code` ``).
    pub code: Color,
    /// `--tui-context` — unchanged lines in a diff.
    pub context: Color,
    /// `--tui-value` — a right-flush value.
    pub value: Color,
    /// `--tui-label` — the 8-cell label column: `in`/`writes`/`network`,
    /// `provider`/`access`, a code block's language caption.
    pub label: Color,
    /// `--tui-dim` — metadata, timestamps, config paths, right-flush result
    /// summaries.
    pub dim: Color,
    /// `--tui-quiet` — tool names, stdout, an option's purpose, a key hint's
    /// verb. One step *brighter* than [`Palette::label`]: the name reads as
    /// "quieter than prose", not "quieter than everything".
    pub quiet: Color,
    /// `--tui-mark` — the gold `▌`, `▸` and caret. Session identity is *not*
    /// marked with this — the top bar carries no mark — so it is reserved
    /// for the open step, the selected row, the prompt and the caret.
    pub mark: Color,
    /// `--tui-mark-idle` — `▌` on a selectable row that is not selected.
    pub mark_idle: Color,
    /// `--tui-band` — the selected row's fill, a gold tint. Always paired
    /// with `mark`, never one without the other.
    pub band: Color,
    /// `--tui-accent-text` — text on the selected row. Primary ink, not
    /// gold: the band and the mark carry the selection, and gold text on a
    /// gold tint would spend the accent twice in one row.
    pub accent_text: Color,
    /// `--tui-speaker-you` — the `you` turn label.
    pub speaker_you: Color,
    /// `--tui-speaker-agent` — the `aldwin` turn label.
    pub speaker_agent: Color,
    /// `--tui-gauge-fill` — the context gauge under 80%. A neutral: a gauge
    /// that is merely filling is not news.
    pub gauge_fill: Color,
    /// `--tui-gauge-fill-hot` — the context gauge at 80% and above, where
    /// it turns gold because it has become the thing to look at.
    pub gauge_fill_hot: Color,
    pub gauge_track: Color,
    /// `--tui-done` — a settled `●`: a finished tool call, an answered
    /// first-run step, the idle status. "Readable, below the mark."
    ///
    /// One role where there used to be two. `glyph_done` and `step_done`
    /// existed separately because on the old light ground they had to recede
    /// from a violet mark in opposite directions; with a neutral `●` and a
    /// gold mark there is nothing to recede from, and the split went with
    /// the hue that needed it.
    pub done: Color,
    /// `--tui-glyph-running` — the spinner (`◐◓◑◒`).
    pub glyph_running: Color,
    /// `--tui-glyph-pending` — `○`: a step still to come, a hunk not yet
    /// reached, the waiting status.
    pub glyph_pending: Color,
    /// `--tui-hunk-header` — the `@@` row. Metadata grey now; it was an
    /// accent step, and a hunk header is not a mark.
    pub hunk_header: Color,
    /// `--tui-ok` — `✓`. The same sage as [`Palette::add`]: a diff sign is a
    /// status.
    pub ok: Color,
    /// `--tui-err` — `✗`. The same rose as [`Palette::del`].
    pub err: Color,
    /// `--tui-warn` — `!`, a call that was denied.
    pub warn: Color,
    /// `--tui-info` — `·`, the info line's pointer. The only place the sky
    /// hue appears outside a code block.
    pub info: Color,
    /// `--tui-syn-keyword` — iris: a keyword, a storage modifier, `self`.
    ///
    /// The syntax ramp is **two hues and no more** — iris here and sky on
    /// [`Palette::syn_call`]. It was five. `palette.css` states what
    /// happened to the rest: "Strings are the quiet neutral; types, numbers
    /// and every other identifier are the code tone; comments are metadata."
    /// So [`Palette::syn_string`] is a role with no hue of its own, and a
    /// type or a number is simply [`Palette::code`].
    ///
    /// The rule that bounds it: **no status hue appears inside a code
    /// block**, "so rose and ember never appear inside a code block and code
    /// can never be mistaken for an error or a diff". The old ramp broke
    /// exactly that — its string role *was* the diff green.
    ///
    /// `highlight.rs` builds a syntect theme from these rather than loading
    /// one, so a fenced block cannot introduce a colour the system never
    /// chose; `every_highlighted_colour_is_one_of_the_five_roles` pins it.
    pub syn_keyword: Color,
    /// `--tui-syn-call` — sky: the *name* in a call or definition, not the
    /// call expression around it. See [`Palette::syn_keyword`].
    pub syn_call: Color,
    /// `--tui-syn-string` — a string literal, and whatever a syntax nests
    /// inside one, in the quiet neutral. See [`Palette::syn_keyword`].
    pub syn_string: Color,
    /// `--tui-reverse-bg` / `--tui-reverse-ink` — the gold fill and the ink
    /// on it. `semantic.css` scopes this to "id chips in documentation";
    /// nothing inside a frame uses it since the wordmark was cut, and it is
    /// carried only so the palette stays one-to-one with the token layer.
    pub reverse_bg: Color,
    pub reverse_ink: Color,
    /// `--tui-diff-box` — the surface quoted code sits on: inline spans,
    /// fenced blocks, the inline transcript diff. The same rung as
    /// [`Palette::bar`], so quoted code reads as lifted rather than sunk.
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

impl Palette {
    /// What `fg` becomes in a transcript that has sunk behind an open panel.
    ///
    /// `semantic.css` is explicit that this is "a recolour, never alpha", and
    /// gives three destinations: prose and paths, tool names and facts,
    /// glyphs and labels. Ink that carries *content* — prose, a path, code —
    /// keeps the brighter [`Palette::scrim_text`]; everything else recedes to
    /// [`Palette::scrim_mark`].
    ///
    /// `scrim_quiet` and `scrim_mark` are one value in both themes, which is
    /// what makes a lookup by colour sound here: several live roles share a
    /// hex (`quiet`, `value` and `speaker_agent` are all neutral-300), and if
    /// the two quiet destinations ever part company this has to start asking
    /// what a cell *is* rather than what colour it carries. The test
    /// `the_two_quiet_scrim_roles_coincide` exists to say so out loud when
    /// that day comes.
    pub(crate) fn scrimmed(&self, fg: Color) -> Color {
        let content = [self.text, self.body, self.code, self.accent_text, self.add_code, self.del_code];
        match fg {
            Color::Rgb(..) if content.contains(&fg) => self.scrim_text,
            Color::Rgb(..) => self.scrim_mark,
            // `Color::Reset` and the indexed colours are whatever the
            // terminal itself paints; there is no role to move them to.
            other => other,
        }
    }
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

    /// Parses `tui.yaml`'s `theme` field (`aldwin_config::TuiConfig::theme`)
    /// — case-insensitive `"light"` selects `Light`; `None`, `"dark"`, or
    /// anything unrecognized selects `Dark`, the long-standing default. An
    /// unrecognized value doesn't refuse to start: consistent with this
    /// being a purely cosmetic setting, a typo shouldn't block the session
    /// the way a malformed permissions or provider file does. `pub`, not
    /// `pub(crate)` — `aldwin-cli`'s bootstrap calls this to resolve
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
    /// Both ladders climb, darkest first. They are the same list with one
    /// rung moved: light's `panel_title` is "the one raised band darker than
    /// ground, because it is a title" and sits between `recess` and `break_`
    /// instead of at the top. Everything else holds its place, which is the
    /// change from the previous light theme — that one sank every band
    /// below a near-white page and so ran the opposite way to dark.
    #[test]
    fn both_ground_ladders_are_strictly_ordered_and_have_no_repeated_rung() {
        let dark = [DARK.scrim, DARK.recess, DARK.break_, DARK.ground, DARK.bar_bottom, DARK.bar, DARK.panel_title];
        let light =
            [LIGHT.scrim, LIGHT.recess, LIGHT.panel_title, LIGHT.break_, LIGHT.ground, LIGHT.bar_bottom, LIGHT.bar];

        for (name, ladder) in [("dark", dark), ("light", light)] {
            for pair in ladder.windows(2) {
                assert!(luma(pair[0]) < luma(pair[1]), "the {name} ladder climbs: {:?} then {:?}", pair[0], pair[1]);
            }
        }
        // Seven distinct rungs needs no separate assertion: a strict
        // ordering by luma already rules out two bands sharing a tone.
    }

    /// The chrome bands sit the same way round in both themes: both rise
    /// off the ground, the top bar a step further than the composer. Pinned
    /// because the previous light theme had them *sinking*, and the one
    /// before that had them the wrong way round entirely — this pair has
    /// been inverted twice, so it is worth a test that reads as a sentence.
    #[test]
    fn the_top_bar_is_further_from_the_ground_than_the_composer_in_both_themes() {
        for (name, pal) in [("dark", DARK), ("light", LIGHT)] {
            assert!(luma(pal.bar_bottom) > luma(pal.ground), "{name}: the composer rises off the ground");
            assert!(luma(pal.bar) > luma(pal.bar_bottom), "{name}: the top bar rises further");
        }
    }

    /// A settled `●` is a neutral, not an accent step — "readable, below the
    /// mark". Under the single-hue system it was accent-700, so the habit to
    /// guard against is reaching for the brand colour to mean "finished".
    /// Gold is spent on what is *open*.
    #[test]
    fn a_settled_glyph_is_a_neutral_and_never_the_mark() {
        for (name, pal) in [("dark", DARK), ("light", LIGHT)] {
            assert_ne!(pal.done, pal.mark, "{name}: done is not gold");
            assert_eq!(pal.done, pal.label, "{name}: done is the label neutral");
            assert_eq!(pal.glyph_running, pal.mark, "{name}: what is running *is* gold");
        }
    }

    /// A diff sign is a status, so the pairs share a hue — and a code block
    /// may carry neither. See [`Palette::syn_keyword`].
    #[test]
    fn diff_signs_are_statuses_and_no_status_hue_is_a_syntax_role() {
        for (name, pal) in [("dark", DARK), ("light", LIGHT)] {
            assert_eq!(pal.add, pal.ok, "{name}: added is the ok sage");
            assert_eq!(pal.del, pal.err, "{name}: removed is the err rose");
            for syn in [pal.syn_keyword, pal.syn_call, pal.syn_string] {
                for (status, what) in [(pal.ok, "ok"), (pal.err, "err"), (pal.warn, "warn")] {
                    assert_ne!(syn, status, "{name}: a syntax role carries the {what} hue");
                }
            }
        }
    }

    /// [`Palette::scrimmed`] looks a cell up by colour, which is sound only
    /// while the two quiet destinations are one value. If the design ever
    /// parts them, this fails and says what has to change.
    #[test]
    fn the_two_quiet_scrim_roles_coincide() {
        for (name, pal) in [("dark", DARK), ("light", LIGHT)] {
            assert_eq!(
                pal.scrim_quiet, pal.scrim_mark,
                "{name}: scrimmed() maps by colour and cannot tell a tool name from a glyph; \
                 give it the cell's role before letting these differ"
            );
        }
    }

    #[test]
    fn a_scrimmed_transcript_keeps_content_brighter_than_everything_else() {
        for pal in [DARK, LIGHT] {
            for content in [pal.text, pal.body, pal.code] {
                assert_eq!(pal.scrimmed(content), pal.scrim_text);
            }
            for other in [pal.quiet, pal.dim, pal.mark, pal.done, pal.speaker_you, pal.add, pal.err] {
                assert_eq!(pal.scrimmed(other), pal.scrim_mark);
            }
            assert_eq!(pal.scrimmed(Color::Reset), Color::Reset, "the terminal's own colour has no role to move to");
        }
    }
}
