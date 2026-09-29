//! The colour palette for `ui/`, one field per token in the design's
//! `tokens/colors.css` (`docs/design/`). Colour rules: AGENTS.md "Design
//! System".
//!
//! `DARK` (`:root`) and `LIGHT` (`.tui-light`) are chosen once by
//! `Theme::from_config` and passed explicitly. Never a global or `OnceLock`:
//! tests run in parallel in one process and a shared theme would leak.
//!
//! Bands are separated by ground tone only; the light theme inverts the
//! ladder, so "raised" is a step in either direction.
//!
//! No `chrome`/`dot` (the mock's title bar) and no `syn`/`call` (reserved,
//! unused by the design): the generator lists them as uncarried.

use ratatui::style::Color;

use crate::tokens::{
    GAUGE_DARK, GAUGE_LIGHT, GAUGE_SEGMENTS, HIGHLIGHT_DARK, HIGHLIGHT_LIGHT, MARK_COLS, MARK_DARK,
    MARK_LIGHT, MARK_ROWS,
};

/// The two palettes, generated into [`crate::tokens`]; review stage 4 fails
/// on any drift from the design.
pub(crate) use crate::tokens::{DARK, LIGHT};

/// One theme's colours, one field per `tokens/colors.css` token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    /// The theme this palette belongs to, so holders need no separate `Theme`.
    pub theme: Theme,
    /// `--win`: the conversation and window ground.
    pub win: Color,
    /// `--tint`: the echoed prompt, the review's file tree, an idle
    /// selection; one step off the ground.
    pub tint: Color,
    /// `--panel`: a question's band.
    pub panel: Color,
    /// `--field`: the input field and any list's current row.
    pub field: Color,
    /// `--select`: the band naming the selection above the comment field;
    /// the only ground with a hint of the accent.
    pub select: Color,
    /// `--track`: an empty context-bar segment.
    pub track: Color,
    /// `--label`: primary text (prose, a running step, the current row).
    pub label: Color,
    /// `--label2`: secondary text (echoed prompt, done step, fact, key verb,
    /// footer).
    pub label2: Color,
    /// `--label3`: tertiary text (line numbers, folders, pending step, fold,
    /// an action not ready).
    pub label3: Color,
    /// `--accent`: blue, the developer's only: `›`, `▎`, `◆`, `✓`, and the
    /// ready action's glyph.
    pub accent: Color,
    /// `--fill`: the accent as a fill (the mark, the context bar); never
    /// text.
    pub fill: Color,
    /// `--onfill`: ink on `fill`; unused by any frame.
    pub onfill: Color,
    /// `--amber`: running, and nothing else.
    pub amber: Color,
    /// `--add`: a `+` sign, an added file's marker, a `+11` count.
    pub add: Color,
    /// `--addcode`: code on an added row.
    pub addcode: Color,
    /// `--addrow`: an added row's ground.
    pub addrow: Color,
    /// `--del`: a `−` sign and a `−2` count.
    pub del: Color,
    /// `--delcode`: code on a removed row.
    pub delcode: Color,
    /// `--delrow`: a removed row's ground.
    pub delrow: Color,
}

impl Palette {
    /// The brand mark's cells for this theme, `[row][col]` of (upper half,
    /// lower half) — see `tokens::MARK_CELL`.
    pub fn mark(&self) -> &'static [[(Color, Color); MARK_COLS]; MARK_ROWS] {
        match self.theme {
            Theme::Dark => &MARK_DARK,
            Theme::Light => &MARK_LIGHT,
        }
    }

    /// The working line's highlight at `distance` cells from its centre;
    /// every cell past the table's end takes its last tone.
    pub fn highlight(&self, distance: usize) -> Color {
        let table: &[Color] = match self.theme {
            Theme::Dark => &HIGHLIGHT_DARK,
            Theme::Light => &HIGHLIGHT_LIGHT,
        };
        table[distance.min(table.len() - 1)]
    }

    /// The context bar's segments, left to right, with `filled` lit (clamped
    /// to `GAUGE_SEGMENTS`); the lit run ramps to `fill` at its leading edge.
    pub fn gauge(&self, filled: usize) -> &'static [Color; GAUGE_SEGMENTS] {
        let table = match self.theme {
            Theme::Dark => &GAUGE_DARK,
            Theme::Light => &GAUGE_LIGHT,
        };
        &table[filled.min(GAUGE_SEGMENTS)]
    }
}

/// Which of the two fixed palettes a session renders with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Light text on a dark ground; the default.
    #[default]
    Dark,
    /// Dark text on a light ground, the ground ladder inverted.
    Light,
}

impl Theme {
    /// Parses `tui.yaml`'s `theme`; anything but `light` (case-insensitive,
    /// trimmed), or none, is dark.
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

    /// Guards a regenerated palette against losing a step and merging two
    /// bands on screen.
    #[test]
    fn the_ground_ladder_steps_one_way_in_dark_and_the_other_in_light() {
        for (pal, rising) in [(&DARK, true), (&LIGHT, false)] {
            let ladder = [pal.win, pal.tint, pal.panel, pal.field];
            for pair in ladder.windows(2) {
                let (lower, upper) = (luma(pair[0]), luma(pair[1]));
                assert!(
                    if rising { upper > lower } else { upper < lower },
                    "{:?}: {:?} -> {:?}",
                    pal.theme,
                    pair[0],
                    pair[1]
                );
            }
        }
    }

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
    fn the_highlight_fades_from_label_toward_label2() {
        for pal in [&DARK, &LIGHT] {
            assert_eq!(pal.highlight(0), pal.label);
            let (near, far) = (pal.highlight(1), pal.highlight(2));
            let between = |c: Color| {
                let (l, l2, c) = (luma(pal.label), luma(pal.label2), luma(c));
                l.min(l2) < c && c < l.max(l2)
            };
            assert!(between(near) && between(far), "{:?}", pal.theme);
            assert_eq!(pal.highlight(9), far, "the far tone holds");
        }
    }

    #[test]
    fn the_gauge_is_empty_at_zero_and_full_at_ten() {
        for pal in [&DARK, &LIGHT] {
            assert!(pal.gauge(0).iter().all(|c| *c == pal.track));
            assert_eq!(pal.gauge(10)[9], pal.fill, "the leading edge is full fill");
            assert_eq!(
                pal.gauge(4)[4],
                pal.track,
                "the fifth segment of four is empty"
            );
            assert_eq!(pal.gauge(4)[3], pal.fill);
        }
    }
}
