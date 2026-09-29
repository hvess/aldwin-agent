//! The footer's working line (frames B–D, `W1`, `W2`): the running `●`
//! blinking in the mark column, the phrase typing in with a highlight
//! running along it, and the turn's time after it. Every cell is a glyph
//! and a foreground colour, redrawn each tick.

use std::time::Duration;

use ratatui::style::{Color, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use super::grid::{elide, MARK_COL};
use crate::activity::WorkingLine;
use crate::motion::{ticks, Motion};
use crate::palette::Palette;

/// How long the running `●` and its `○` each show.
const BLINK: u64 = ticks(Duration::from_millis(500));

/// Characters a new phrase types in per tick.
const TYPED_PER_TICK: usize = 3;

/// Cells the highlight's centre travels beyond each end of the phrase, so
/// it enters and leaves rather than jumping.
const RUN_OUT: usize = 4;

/// The mark column: amber `●` and `○` in turn (a still `●` under reduced
/// motion), or a still `label3` `○` once stalled.
pub(super) fn mark(line: &WorkingLine, tick: u64, motion: Motion, pal: &Palette) -> Span<'static> {
    let (glyph, colour) = if line.stalled {
        ("○", pal.label3)
    } else if motion == Motion::Reduced || (tick / BLINK).is_multiple_of(2) {
        ("●", pal.amber)
    } else {
        ("○", pal.amber)
    };
    Span::styled(format!("{glyph:<MARK_COL$}"), Style::default().fg(colour))
}

/// The phrase and the timer, fitted to `room` cells: the phrase is
/// shortened, the timer never. Under reduced motion the phrase is whole from
/// its first tick, in `label` up to the call's target.
pub(super) fn words(
    line: &WorkingLine,
    motion: Motion,
    pal: &Palette,
    room: usize,
) -> Vec<Span<'static>> {
    let timer = format!("  {}m {:02}s", line.seconds / 60, line.seconds % 60);
    let phrase = elide(&line.words, room.saturating_sub(timer.width()));
    let all = phrase.chars().count();
    let mut spans = match (line.stalled, motion) {
        (true, _) => runs(&phrase, all, line.code_at, pal, |_| pal.label2),
        (false, Motion::Reduced) => runs(&phrase, all, line.code_at, pal, |_| pal.label),
        (false, Motion::Full) => animated(&phrase, line.age, line.code_at, pal),
    };
    spans.push(Span::styled(timer, Style::default().fg(pal.label3)));
    spans
}

/// `phrase` `age` ticks after it began: typing in, then the highlight
/// sweeping it on a loop.
fn animated(phrase: &str, age: u64, code_at: Option<usize>, pal: &Palette) -> Vec<Span<'static>> {
    let len = phrase.chars().count();
    let typing = len.div_ceil(TYPED_PER_TICK) as u64;
    // The highlight's centre, offset by `RUN_OUT` so it stays unsigned.
    let sweep = (age >= typing).then(|| ((age - typing) % (len + 2 * RUN_OUT) as u64) as usize);
    let shown = len.min((age as usize + 1).saturating_mul(TYPED_PER_TICK));
    runs(phrase, shown, code_at, pal, |i| {
        sweep.map_or(pal.label, |s| pal.highlight((i + RUN_OUT).abs_diff(s)))
    })
}

/// The first `shown` characters of `phrase`, each in `tone(i)` up to
/// `code_at` and in `--code` from it, which the highlight does not touch
/// (frame `W2`). Runs of one colour share a span.
fn runs(
    phrase: &str,
    shown: usize,
    code_at: Option<usize>,
    pal: &Palette,
    tone: impl Fn(usize) -> Color,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, c) in phrase.chars().take(shown).enumerate() {
        let colour = match code_at {
            Some(at) if i >= at => pal.code,
            _ => tone(i),
        };
        match spans.last_mut() {
            Some(last) if last.style.fg == Some(colour) => last.content.to_mut().push(c),
            _ => spans.push(Span::styled(c.to_string(), Style::default().fg(colour))),
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::DARK;

    /// A line whose target begins at char 8, as `Reading router.rs`'s
    /// does in frame `W2`.
    fn line(words: &str, age: u64) -> WorkingLine {
        WorkingLine {
            words: words.into(),
            code_at: Some(8),
            age,
            stalled: false,
            seconds: 62,
        }
    }

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Each character's colour, the timer's included.
    fn colours(spans: &[Span]) -> Vec<Color> {
        spans
            .iter()
            .flat_map(|s| s.content.chars().map(move |_| s.style.fg.unwrap()))
            .collect()
    }

    /// Frame `W2`'s first four rows, 100ms apart.
    #[test]
    fn a_new_phrase_types_in_three_characters_a_tick() {
        for (age, typed) in [
            (0, "Rea"),
            (1, "Readin"),
            (2, "Reading r"),
            (3, "Reading rout"),
        ] {
            let spans = words(&line("Reading router.rs", age), Motion::Full, &DARK, 80);
            assert_eq!(text(&spans), format!("{typed}  1m 02s"));
            let typed = &colours(&spans)[..typed.len()];
            for (i, colour) in typed.iter().enumerate() {
                let wanted = if i < 8 { DARK.label } else { DARK.code };
                assert_eq!(*colour, wanted, "{i} at {age}");
            }
        }
    }

    /// Frame `W2`'s hold rows, typed by 600ms: at 600ms the centre is off
    /// the left edge, at 900ms one cell short of the `R`, at 1200ms on the
    /// first `a`.
    #[test]
    fn the_highlight_runs_in_from_the_left_once_typed() {
        let colours = |age| colours(&animated("Reading router.rs", age, Some(8), &DARK));
        let (near, far) = (DARK.highlight(1), DARK.highlight(2));
        assert!(colours(6)[..8].iter().all(|c| *c == far));
        assert_eq!(&colours(9)[..2], &[near, far]);
        assert_eq!(&colours(12)[..5], &[far, near, DARK.label, near, far]);
        for age in [6, 9, 12, 20] {
            assert!(
                colours(age)[8..].iter().all(|c| *c == DARK.code),
                "the target keeps its ink under the highlight at {age}"
            );
        }
    }

    #[test]
    fn the_dot_blinks_in_amber_until_the_line_stalls() {
        let working = line("Thinking", 0);
        let full = Motion::Full;
        assert_eq!(mark(&working, 0, full, &DARK).content, "● ");
        assert_eq!(mark(&working, BLINK, full, &DARK).content, "○ ");
        assert_eq!(
            mark(&working, BLINK, full, &DARK).style.fg,
            Some(DARK.amber)
        );
        let stalled = WorkingLine {
            stalled: true,
            ..working
        };
        assert_eq!(mark(&stalled, 0, full, &DARK).style.fg, Some(DARK.label3));
    }

    #[test]
    fn under_reduced_motion_only_the_timer_moves() {
        let reduced = Motion::Reduced;
        let working = line("Reading router.rs", 0);
        assert_eq!(mark(&working, BLINK, reduced, &DARK).content, "● ");
        let spans = words(&working, reduced, &DARK, 80);
        assert_eq!(text(&spans), "Reading router.rs  1m 02s");
        assert_eq!(spans[0].style.fg, Some(DARK.label));
        assert_eq!(spans[1].content, "router.rs");
        assert_eq!(spans[1].style.fg, Some(DARK.code));
    }

    /// Frame `W2`'s stalled row: the lead in `label2`, the target in `--code`.
    #[test]
    fn a_stalled_line_keeps_its_target_in_code() {
        let stalled = WorkingLine {
            stalled: true,
            code_at: Some(14),
            ..line("Still reading router.rs", 0)
        };
        let spans = words(&stalled, Motion::Full, &DARK, 80);
        assert_eq!(spans[0].content, "Still reading ");
        assert_eq!(spans[0].style.fg, Some(DARK.label2));
        assert_eq!(spans[1].content, "router.rs");
        assert_eq!(spans[1].style.fg, Some(DARK.code));
    }

    #[test]
    fn a_long_phrase_is_shortened_and_the_timer_kept() {
        let spans = words(
            &line("Running cargo test --workspace", 99),
            Motion::Full,
            &DARK,
            20,
        );
        assert_eq!(text(&spans), "Running car…  1m 02s");
    }
}
