//! Themeable colour palette for `ui/` — the Aldwin Design System
//! (`claude.ai/design`, project `b9de8837-…`, imported 2026-09-23; see
//! `.claude/design/IMPORT.md`) rather than hand-picked. Field names mirror
//! the design's own tokens (`tokens/colors.css`) one-to-one, so a value
//! here can be checked directly against that source instead of against
//! another layer of local naming. Two fixed instances — `DARK` (`:root`)
//! and `LIGHT` (`.tui-light`) — selected once at startup via
//! `Theme::from_config` and carried explicitly from there. Deliberately
//! *not* a global/`OnceLock`: this crate's `cargo test` runs many tests in
//! parallel inside one process, and a shared mutable "current theme" would
//! make one test's theme choice leak into another's.
//!
//! # What the system is
//!
//! Neutrals at OKLCH hue 260 with chroma under 0.01, and **one accent, blue
//! at hue 255: blue means you.** Your prompt, your selection, your comments,
//! your next action, and nothing else. Amber means running. Green and red
//! appear only in a diff. Three text tones. It replaced the lantern-gold
//! system on 2026-09-23 — every value changed, and so did the argument:
//! that system spent its one colour on "what is open"; this one spends it on
//! *the developer*, and the running state gets a colour of its own.
//!
//! Four rules follow, each the design's own sentence:
//!
//! * **Blue is the developer's.** The `›` of the prompt and of the current
//!   row, the `▎` of a selection, a `◆` comment, `✓` on a step the agent
//!   finished for them, and the key glyph of the action that is ready. The
//!   agent's own activity is never blue.
//! * **Amber means running**, and nothing else is amber: the `●` of the
//!   running plan step and of `Working…` in the footer.
//! * **Green and red appear only in diffs** — a failure is a sentence in
//!   `label`, not a red row (ADR 0009 §5).
//! * **Three text tones.** `label` for what is current, `label2` for what
//!   is said around it, `label3` for what is pending or structural — line
//!   numbers, folders, an unread step.
//!
//! # Grounds, not borders
//!
//! Nothing inside a window is stroked. Bands are distinct grounds and a
//! band's *tone* is the only thing separating it from its neighbour:
//! [`Palette::win`] for the conversation, [`Palette::tint`] for the echoed
//! prompt and the file tree, [`Palette::panel`] for a question,
//! [`Palette::field`] for the input and the current row, [`Palette::select`]
//! for the selection label. The light theme inverts the ladder — grounds
//! step darker as they rise — so a "raised" band is a step in either
//! direction, never a lighter one by assumption.
//!
//! There is deliberately no `chrome` or `dot` field: they paint the mock's
//! macOS title bar, and a terminal's title bar is the terminal's. And no
//! `syn` or `call`: the design reserves them and applies them nowhere, so
//! the generator lists them as uncarried rather than hand code a hue it
//! must not use.

use ratatui::style::Color;

/// The two palettes, **generated** from `.claude/design/tokens/` into
/// [`crate::tokens`] and re-exported here so every call site keeps reading
/// `palette::DARK`. The review loop's stage 3 regenerates the file and
/// fails if the result differs, so the app's palette and the imported
/// design cannot disagree.
pub(crate) use crate::tokens::{DARK, LIGHT};

/// One themeable surface, matching `tokens/colors.css` one-to-one (see this
/// module's doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// Which half of the system this palette is — carried on the palette
    /// itself so anything already holding one can ask, without a second
    /// `Theme` threaded down beside it purely to answer the same question.
    pub theme: Theme,
    /// `--win` — the conversation ground, and the window's.
    pub win: Color,
    /// `--tint` — the echoed prompt, the review's file tree, an idle
    /// selection. One step off the ground.
    pub tint: Color,
    /// `--panel` — a question's band.
    pub panel: Color,
    /// `--field` — the input field, and the current row of any list.
    pub field: Color,
    /// `--select` — the band above the comment field naming what is
    /// selected. The one ground that carries a hint of the accent.
    pub select: Color,
    /// `--track` — an empty context-bar segment.
    pub track: Color,
    /// `--label` — primary text: prose, a running step, the current row.
    pub label: Color,
    /// `--label2` — secondary: the echoed prompt, a done step, a fact, a
    /// key's verb, the footer.
    pub label2: Color,
    /// `--label3` — tertiary: line numbers, folders, a pending step, a fold,
    /// an action that is not ready.
    pub label3: Color,
    /// `--accent` — blue means you: `›`, `▎`, `◆`, `✓`, and the ready
    /// action's glyph.
    pub accent: Color,
    /// `--fill` — the accent as a fill: the mark and the context bar. Never
    /// text.
    pub fill: Color,
    /// `--onfill` — ink on `fill`. Carried for completeness; no frame puts
    /// text on a fill.
    pub onfill: Color,
    /// `--amber` — running, and nothing else.
    pub amber: Color,
    /// `--add` — a `+` sign, an added file's marker, a `+11` count.
    pub add: Color,
    /// `--addcode` — code on an added row.
    pub addcode: Color,
    /// `--addrow` — an added row's ground.
    pub addrow: Color,
    /// `--del` — a `−` sign and a `−2` count.
    pub del: Color,
    /// `--delcode` — code on a removed row.
    pub delcode: Color,
    /// `--delrow` — a removed row's ground.
    pub delrow: Color,
}

impl Palette {
    /// The brand mark's cells for this theme, `[row][col]` of (upper half,
    /// lower half) — see `tokens::MARK_CELL`.
    pub fn mark(&self) -> &'static [[(Color, Color); crate::tokens::MARK_COLS]; crate::tokens::MARK_ROWS] {
        match self.theme {
            Theme::Dark => &crate::tokens::MARK_DARK,
            Theme::Light => &crate::tokens::MARK_LIGHT,
        }
    }

    /// The context bar's ten segments for `filled` of them lit, left to
    /// right — the filled run ramping to `fill` at its leading edge.
    pub fn gauge(&self, filled: usize) -> &'static [Color; crate::tokens::GAUGE_SEGMENTS] {
        let table = match self.theme {
            Theme::Dark => &crate::tokens::GAUGE_DARK,
            Theme::Light => &crate::tokens::GAUGE_LIGHT,
        };
        &table[filled.min(crate::tokens::GAUGE_SEGMENTS)]
    }
}

/// Which of the two fixed palettes a session renders with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    /// `tui.yaml`'s `theme` value. Anything unrecognised — including
    /// nothing at all — is dark, in exactly one place.
    pub fn from_config(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("light") => Theme::Light,
            _ => Theme::Dark,
        }
    }

    pub(crate) fn palette(self) -> &'static Palette {
        match self {
            Theme::Dark => &DARK,
            Theme::Light => &LIGHT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(c: Color) -> u32 {
        match c {
            Color::Rgb(r, g, b) => u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114,
            other => panic!("palette values are Rgb, got {other:?}"),
        }
    }

    /// The design's own sentence: "The ladder inverts: grounds step darker
    /// as they rise." Dark's bands rise lighter from `win`; light's rise
    /// darker. Pinned so a regenerated palette that lost the step would
    /// fail here rather than merge two bands on screen.
    #[test]
    fn the_ground_ladder_steps_one_way_in_dark_and_the_other_in_light() {
        for (pal, rising) in [(&DARK, true), (&LIGHT, false)] {
            let ladder = [pal.win, pal.tint, pal.panel, pal.field];
            for pair in ladder.windows(2) {
                let (lower, upper) = (luma(pair[0]), luma(pair[1]));
                assert!(if rising { upper > lower } else { upper < lower }, "{:?}: {:?} -> {:?}", pal.theme, pair[0], pair[1]);
            }
        }
    }

    /// Three tones, in order, in both themes — the whole text hierarchy.
    #[test]
    fn the_three_text_tones_are_ordered() {
        assert!(luma(DARK.label) > luma(DARK.label2) && luma(DARK.label2) > luma(DARK.label3));
        assert!(luma(LIGHT.label) < luma(LIGHT.label2) && luma(LIGHT.label2) < luma(LIGHT.label3));
    }

    #[test]
    fn theme_from_config_falls_back_to_dark() {
        assert_eq!(Theme::from_config(Some("light")), Theme::Light);
        assert_eq!(Theme::from_config(Some(" LIGHT ")), Theme::Light);
        assert_eq!(Theme::from_config(Some("neon")), Theme::Dark);
        assert_eq!(Theme::from_config(None), Theme::Dark);
    }

    #[test]
    fn the_gauge_is_empty_at_zero_and_full_at_ten() {
        for pal in [&DARK, &LIGHT] {
            assert!(pal.gauge(0).iter().all(|c| *c == pal.track));
            assert_eq!(pal.gauge(10)[9], pal.fill, "the leading edge is full fill");
            assert_eq!(pal.gauge(4)[4], pal.track, "the fifth segment of four is empty");
            assert_eq!(pal.gauge(4)[3], pal.fill);
        }
    }
}
