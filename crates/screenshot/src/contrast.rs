//! Contrast, computed once, for every adjacency a frame actually paints.
//!
//! Three separate judges on `run-1789850385` computed WCAG ratios by hand
//! from sampled pixels, and three of the conformance catalogue's Class C
//! items are contrast findings. None of that needed a model: the declared
//! grid holds both colours, and the arithmetic is four lines.
//!
//! # The floors, and where each one comes from
//!
//! Nothing here is a number this harness invented. The design system states
//! three, in three different registers, and this module refuses to smooth
//! over the fact that they disagree about scope:
//!
//! * **Ink on its own ground: 3.3:1.** `HANDOFF.md:109` calibrates `--t-dim`
//!   — the dimmest ink role in the system — at 3.3:1 against the recessed
//!   field, "the darkest band and the binding one in this theme". That is the
//!   design deliberately setting its own worst case, so ink below it is ink
//!   the design never sanctioned.
//! * **Ground against adjacent ground: 1.15:1.** `HANDOFF.md:74` — "every
//!   adjacency in the five screens is now at least 1.15:1". Since Turn 13
//!   nothing inside a frame is stroked, so a boundary *is* its tonal step;
//!   below this there is no boundary and nothing to fall back on.
//! * **4.67:1** appears in `SYNC.md`'s Turn 15 note as "the project's
//!   minimum" while rejecting an added `+` at 4.51:1. It is **not** used as a
//!   floor here, because `HANDOFF.md:109` calibrates an ink role at 3.3:1 in
//!   the same breath — the two cannot both be global. Applying 4.67 would
//!   fire on every `--tui-dim` span in every frame, which is the design's own
//!   deliberate choice reported as a defect. The disagreement is recorded as
//!   conformance Class C item 10 and wants settling upstream; until it is,
//!   this gate holds the floor the design actually calibrated.
//!
//! # A mark is not text, and the design says so
//!
//! `HANDOFF.md:109` sets the idle `▌` deliberately low — "it holds ~1.6:1
//! there and ~2.4:1 on the bar" — because a mark that is not the selected one
//! is meant to recede. Holding it to the ink floor would report the design's
//! own calibration as a defect, so [`MARK_FLOOR`] is the design's own number
//! for it and applies to the mark and glyph roles.
//!
//! # Why a *dimmed* span is measured but never failed
//!
//! The transcript behind a permission panel is blended toward the ground, and
//! measuring it is how conformance Class A item 38 was found. Failing it is a
//! different matter, and the arithmetic settles it: at the design's own
//! stated 35% the dimmed body measures **2.706:1** dark and **1.975:1**
//! light, while the app's 45% measures 3.566:1 and 2.489:1. The design's
//! stated opacity is *further* below the design's stated ink floor than the
//! app is. No opacity satisfies both.
//!
//! So the design has no answer here, and per `.claude/skills/screenshot` a
//! rendered thing the design has no answer for is a stop-and-ask, not
//! something for a gate to invent a number about. Dimmed spans are carried in
//! `facts.adjacencies` with their ratios, where a reader and a judge both see
//! them, and they are not failed. What that costs is stated plainly rather
//! than hidden: a regression in the dim would not be caught here.

use crate::vt::Color;

/// `HANDOFF.md:109` — the dimmest ink the design calibrates.
pub const INK_FLOOR: f64 = 3.3;
/// `HANDOFF.md:74` — the narrowest ground-to-ground step the design claims.
pub const GROUND_FLOOR: f64 = 1.15;
/// `HANDOFF.md:109` — the idle mark "holds ~1.6:1" on the recessed field,
/// which is the design stating its own floor for a glyph meant to recede.
pub const MARK_FLOOR: f64 = 1.6;

/// Roles that paint a **mark** rather than text, and so answer to
/// [`MARK_FLOOR`] instead of [`INK_FLOOR`].
///
/// Not a judgement call: every one of these is named in the design's glyph
/// table or its gauge, and none of them ever carries a word.
pub const MARKS: [&str; 7] = ["mark", "mark-idle", "glyph-running", "glyph-done", "glyph-pending", "gauge-fill", "gauge-track"];

/// The floor a span answers to, or `None` when the design states none for it.
///
/// `ink` is the role description as [`crate::facts`] writes it — a role name,
/// a `/`-joined list when one value carries several roles, or `dim(...)` for
/// a blend.
pub fn floor_for(ink: &str) -> Option<f64> {
    if ink.starts_with("dim(") {
        return None;
    }
    let names = ink.trim_start_matches("--tui-");
    if names.split('/').any(|n| MARKS.contains(&n)) {
        return Some(MARK_FLOOR);
    }
    Some(INK_FLOOR)
}

fn channel(c: u8) -> f64 {
    let v = c as f64 / 255.0;
    if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

pub fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

pub fn ratio(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    ((hi + 0.05) / (lo + 0.05) * 1000.0).round() / 1000.0
}

/// The ratio between two declared colours, or `None` when either is not a
/// concrete RGB value — an ANSI index or the terminal's default is already a
/// `colour` gate violation and reporting a ratio for it would imply the cell
/// was legitimate.
pub fn of(fg: Color, bg: Color) -> Option<f64> {
    match (fg, bg) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => Some(ratio((r1, g1, b1), (r2, g2, b2))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_on_white_is_twenty_one_to_one() {
        assert_eq!(ratio((0, 0, 0), (255, 255, 255)), 21.0);
    }

    #[test]
    fn a_colour_against_itself_is_one_to_one() {
        assert_eq!(ratio((0x47, 0x42, 0x51), (0x47, 0x42, 0x51)), 1.0);
    }

    /// The three values the conformance catalogue's Class C item 10 states,
    /// so a change to this arithmetic shows up as a failing test rather than
    /// as a quietly different catalogue.
    #[test]
    fn matches_the_catalogues_published_measurements() {
        // hunk header on the quoted-code field, dark
        assert_eq!(ratio((0xa0, 0x81, 0xd5), (0x3a, 0x36, 0x48)), 3.662);
        // a context row's code on the same field
        assert_eq!(ratio((0x9a, 0x95, 0xa4), (0x3a, 0x36, 0x48)), 4.002);
        // a line number on an added row
        assert_eq!(ratio((0xb1, 0xad, 0xbb), (0x3d, 0x4b, 0x42)), 4.188);
    }

    /// Class C item 2's light-ladder measurement, which is a ground-to-ground
    /// step rather than ink on a ground.
    #[test]
    fn the_light_ladders_narrowest_rung() {
        assert_eq!(ratio((0xf7, 0xf5, 0xfa), (0xef, 0xec, 0xf4)), 1.079);
    }

    #[test]
    fn a_mark_answers_to_the_marks_floor_and_prose_to_inks() {
        assert_eq!(floor_for("--tui-mark-idle"), Some(MARK_FLOOR));
        // One value carrying several roles still counts as a mark if any of
        // them is one — `#5d576a` is both `glyph-pending` and `mark-idle`.
        assert_eq!(floor_for("--tui-glyph-pending/mark-idle"), Some(MARK_FLOOR));
        assert_eq!(floor_for("--tui-body"), Some(INK_FLOOR));
        assert_eq!(floor_for("--tui-dim"), Some(INK_FLOOR));
    }

    #[test]
    fn a_dimmed_span_has_no_floor_to_answer_to() {
        assert_eq!(floor_for("dim(--tui-body, 45%)"), None);
    }

    /// The measurement that decides it: the design's own 35% is further below
    /// the design's own ink floor than the app's 45% is. Pinned here because
    /// the conformance catalogue's Class A item 38 was written the other way
    /// round by three judges who measured the app and not the reference.
    #[test]
    fn the_designs_stated_dim_is_darker_than_the_apps() {
        let blend = |ink: (u8, u8, u8), ground: (u8, u8, u8), alpha: f64| -> (u8, u8, u8) {
            let mix = |i: u8, g: u8| (i as f64 * alpha + g as f64 * (1.0 - alpha)).round() as u8;
            (mix(ink.0, ground.0), mix(ink.1, ground.1), mix(ink.2, ground.2))
        };
        let (body, ground) = ((0xe3, 0xdf, 0xeb), (0x27, 0x23, 0x2f));
        let stated = ratio(blend(body, ground, 0.35), ground);
        let shipped = ratio(blend(body, ground, 0.45), ground);
        assert_eq!(stated, 2.706);
        assert_eq!(shipped, 3.566);
        assert!(stated < shipped, "the reference's own number is the darker of the two");
    }
}
