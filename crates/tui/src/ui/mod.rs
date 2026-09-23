//! The ratatui frontend: `App`'s state turned into a frame, once per
//! redraw.
//!
//! This module owns the two screens' *band layout* and nothing else.
//! Everything it composes lives in a submodule with one job:
//!
//! | module | job |
//! |--------|-----|
//! | [`grid`] | the design system's cell grid, and the `Ctx` (`palette` + column width) every builder takes |
//! | [`wrap`] | word-wrapping one logical line, before anything is inset or filled |
//! | [`row`] | the single filled-row primitive every band is built from |
//! | [`markdown`] | LLM-authored markdown: fences, block prefixes, tables, inline delimiters |
//! | [`launch`] | the brand mark and the launch card |
//! | [`transcript`] | the conversation log, one entry at a time |
//! | [`chrome`] | the field, the comment field, the footer and its context bar |
//! | [`question`] | the question panel and the command list — one list control |
//! | [`review`] | the full-window review: tree, diff, and the field's review shape |
//!
//! The one discipline that spans all of them: a row is wrapped exactly
//! once, by [`wrap`] or by [`row::Row`], *before* it is inset or filled —
//! never afterwards by a `Paragraph`'s own `Wrap`. See aldwin-tui.md's
//! Progress notes for the two bugs that discipline exists to prevent.

pub(crate) mod chrome;
mod grid;
mod launch;
mod markdown;
mod question;
pub(crate) mod review;
mod row;
mod transcript;
mod wrap;

#[cfg(test)]
mod tests;

use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::Frame;

use crate::app::{App, Mode};
use grid::Ctx;

pub(crate) use transcript::Transcript;

/// `padding: 24px 0` on the body — one blank row at the top of the window
/// and one at the bottom, the frame's own.
const BODY_PAD_ROWS: u16 = 1;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let pal = app.theme.palette();
    let area = frame.area();
    // The window ground, drawn first and under everything else.
    frame.render_widget(Block::default().style(Style::default().bg(pal.win)), area);

    if matches!(app.mode, Mode::Review(_)) {
        review::draw(frame, area, app);
        return;
    }

    // The bottom band: what holds it decides its height, measured once.
    let bottom = chrome::Bottom::measure(app, area.width);
    let [top_pad, body, bottom_area] =
        Layout::vertical([Constraint::Length(BODY_PAD_ROWS), Constraint::Min(1), Constraint::Length(bottom.height())]).areas(area);
    let _ = top_pad;

    app.render_width = body.width;
    if app.log.is_empty() {
        // The launch card, two blank rows under the top padding.
        launch::draw(frame, body, app);
    } else {
        let ctx = Ctx::new(pal, body.width);
        let visible = app.transcript_view(body.height as usize);
        transcript::draw_log(frame, body, visible, ctx);
    }

    bottom.draw(frame, bottom_area, app);
}
