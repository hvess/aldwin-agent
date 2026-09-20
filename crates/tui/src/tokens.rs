//! The design system, in Rust. **Generated — do not edit.**
//!
//! Emitted by `mjolnir-review tokens --write` from
//! `.claude/design/tokens/`. The review loop's stage 3 regenerates
//! this file and fails if the result differs, so the app's palette
//! and the imported design cannot drift apart.
//!
//! Roles the terminal cannot express are listed in
//! `crates/review/src/tokens.rs` with the reason; there are 3.

use ratatui::style::Color;

use crate::palette::Palette;
use crate::Theme;

pub(crate) const DARK: Palette = Palette {
    theme: Theme::Dark,
    accent_text: Color::Rgb(0xdf, 0xd1, 0xfb),
    add: Color::Rgb(0x5e, 0xd4, 0x76),
    add_code: Color::Rgb(0x9c, 0xea, 0xa7),
    add_row: Color::Rgb(0x3d, 0x4b, 0x42),
    band: Color::Rgb(0x60, 0x47, 0x88),
    bar: Color::Rgb(0x47, 0x42, 0x51),
    bar_bottom: Color::Rgb(0x36, 0x31, 0x3f),
    body: Color::Rgb(0xe3, 0xdf, 0xeb),
    break_: Color::Rgb(0x1e, 0x1a, 0x26),
    code: Color::Rgb(0xec, 0xe9, 0xf3),
    context: Color::Rgb(0x9a, 0x95, 0xa4),
    del: Color::Rgb(0xf6, 0x6d, 0x67),
    del_code: Color::Rgb(0xff, 0xa8, 0xa0),
    del_row: Color::Rgb(0x4b, 0x3a, 0x42),
    diff_box: Color::Rgb(0x3a, 0x36, 0x48),
    dim: Color::Rgb(0x9a, 0x95, 0xa4),
    gauge_fill: Color::Rgb(0xa0, 0x81, 0xd5),
    gauge_track: Color::Rgb(0x60, 0x5a, 0x6c),
    glyph_done: Color::Rgb(0x7f, 0x64, 0xab),
    glyph_pending: Color::Rgb(0x5d, 0x57, 0x6a),
    glyph_running: Color::Rgb(0xbe, 0x9d, 0xf7),
    ground: Color::Rgb(0x27, 0x23, 0x2f),
    hunk_header: Color::Rgb(0xa0, 0x81, 0xd5),
    label: Color::Rgb(0xb1, 0xad, 0xbb),
    mark: Color::Rgb(0xbe, 0x9d, 0xf7),
    mark_idle: Color::Rgb(0x5d, 0x57, 0x6a),
    panel_title: Color::Rgb(0x5d, 0x57, 0x6b),
    quiet: Color::Rgb(0xc9, 0xc5, 0xd2),
    recess: Color::Rgb(0x0f, 0x0b, 0x15),
    reverse_bg: Color::Rgb(0xbe, 0x9d, 0xf7),
    reverse_ink: Color::Rgb(0x0c, 0x0a, 0x11),
    scrim: Color::Rgb(0x0c, 0x0a, 0x11),
    speaker_agent: Color::Rgb(0xc9, 0xc5, 0xd2),
    speaker_you: Color::Rgb(0xce, 0xb6, 0xfb),
    step_done: Color::Rgb(0x7f, 0x64, 0xab),
    syn_call: Color::Rgb(0x8f, 0xb8, 0xf8),
    syn_keyword: Color::Rgb(0xc9, 0xa2, 0xf7),
    syn_number: Color::Rgb(0xe8, 0xc1, 0x84),
    syn_string: Color::Rgb(0x9c, 0xea, 0xa7),
    syn_type: Color::Rgb(0x6f, 0xcf, 0xd9),
    text: Color::Rgb(0xf4, 0xf2, 0xf9),
    value: Color::Rgb(0xc9, 0xc5, 0xd2),
};

pub(crate) const LIGHT: Palette = Palette {
    theme: Theme::Light,
    accent_text: Color::Rgb(0x4d, 0x2a, 0x80),
    add: Color::Rgb(0x12, 0x63, 0x25),
    add_code: Color::Rgb(0x0d, 0x4d, 0x18),
    add_row: Color::Rgb(0xd3, 0xea, 0xd6),
    band: Color::Rgb(0xd8, 0xcb, 0xf0),
    bar: Color::Rgb(0xe8, 0xe4, 0xee),
    bar_bottom: Color::Rgb(0xef, 0xec, 0xf4),
    body: Color::Rgb(0x35, 0x30, 0x3e),
    break_: Color::Rgb(0xe4, 0xe0, 0xec),
    code: Color::Rgb(0x2a, 0x24, 0x33),
    context: Color::Rgb(0x5c, 0x55, 0x68),
    del: Color::Rgb(0xb0, 0x12, 0x2e),
    del_code: Color::Rgb(0x6b, 0x00, 0x16),
    del_row: Color::Rgb(0xf4, 0xd2, 0xda),
    diff_box: Color::Rgb(0xe9, 0xe5, 0xf0),
    dim: Color::Rgb(0x5c, 0x55, 0x68),
    gauge_fill: Color::Rgb(0x7d, 0x56, 0xb8),
    gauge_track: Color::Rgb(0xb8, 0xb2, 0xc2),
    glyph_done: Color::Rgb(0x7a, 0x58, 0xae),
    glyph_pending: Color::Rgb(0x9a, 0x93, 0xa5),
    glyph_running: Color::Rgb(0x6b, 0x3f, 0xb0),
    ground: Color::Rgb(0xf7, 0xf5, 0xfa),
    hunk_header: Color::Rgb(0x7d, 0x56, 0xb8),
    label: Color::Rgb(0x51, 0x4a, 0x5c),
    mark: Color::Rgb(0x6b, 0x3f, 0xb0),
    mark_idle: Color::Rgb(0x9a, 0x93, 0xa5),
    panel_title: Color::Rgb(0xd4, 0xce, 0xe0),
    quiet: Color::Rgb(0x42, 0x3c, 0x4c),
    recess: Color::Rgb(0xde, 0xd9, 0xe6),
    reverse_bg: Color::Rgb(0x4d, 0x2a, 0x80),
    reverse_ink: Color::Rgb(0xf7, 0xf5, 0xfa),
    scrim: Color::Rgb(0xcf, 0xca, 0xd9),
    speaker_agent: Color::Rgb(0x42, 0x3c, 0x4c),
    speaker_you: Color::Rgb(0x5d, 0x34, 0x99),
    step_done: Color::Rgb(0x5f, 0x3a, 0xa0),
    syn_call: Color::Rgb(0x1f, 0x56, 0xc4),
    syn_keyword: Color::Rgb(0x7b, 0x2f, 0xc9),
    syn_number: Color::Rgb(0x8a, 0x53, 0x00),
    syn_string: Color::Rgb(0x0f, 0x6b, 0x23),
    syn_type: Color::Rgb(0x0a, 0x5c, 0x6d),
    text: Color::Rgb(0x24, 0x1f, 0x2b),
    value: Color::Rgb(0x42, 0x3c, 0x4c),
};

// ---- The grid, from tokens/cells.css --------------------------------
//
// Cell counts. Body text lands at MARGIN_X + LABEL_COL_WIDTH +
// LABEL_GUTTER as a consequence of the three, which is why cells.css
// declares no --body-col and nothing here restates one.
//
// Only tokens the app consumes are emitted. cells.css declares more
// — panel and bar heights among them — and generating a constant
// nothing reads would be this file asserting a layout rule rather
// than carrying a value. Whether the app *should* consume one of
// them is stage 5's question, not stage 3's.

pub(crate) const MARGIN_X: usize = 3;
pub(crate) const LABEL_COL_WIDTH: usize = 8;
pub(crate) const LABEL_GUTTER: usize = 2;
pub(crate) const GROUP_GAP: usize = 6;
pub(crate) const OPTION_LABEL_COL: usize = 16;
pub(crate) const STEP_MARK_COL: usize = 10;
pub(crate) const STEP_CONTENT_COL: usize = 29;
pub(crate) const GUTTER_LINE_NO_INLINE: usize = 5;
pub(crate) const DIFF_SIGN_COL: usize = 2;
