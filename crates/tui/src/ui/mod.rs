//! The ratatui frontend: draws `App` as one frame per redraw. This module
//! owns only the two screens' band layout; each submodule has one job.
//!
//! A row is wrapped exactly once, by [`wrap`] or [`row::Row`], before it is
//! inset or filled — never by a `Paragraph`'s own `Wrap` (aldwin-tui.md
//! Progress notes record the two bugs this prevents).

pub(crate) mod chrome;
mod grid;
mod launch;
mod markdown;
mod question;
pub(crate) mod review;
mod row;
mod transcript;
mod working;
mod wrap;

#[cfg(test)]
mod tests;

use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::Frame;

use crate::app::{App, Mode};

pub(crate) use transcript::Transcript;

/// The frame's body `padding: 24px 0`: one blank row top and bottom.
const BODY_PAD_ROWS: u16 = 1;

/// Draws one frame of `app`: the full-window review when one is open,
/// otherwise the conversation.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let pal = app.theme.palette();
    let area = frame.area();
    // The window ground: drawn first, under everything.
    frame.render_widget(Block::default().style(Style::default().bg(pal.win)), area);

    if matches!(app.mode, Mode::Review(_)) {
        review::draw(frame, area, app);
        return;
    }

    // Measured once: its contents decide the bottom band's height.
    let bottom = chrome::Bottom::measure(app, area.width);
    let [_, body, bottom_area] = Layout::vertical([
        Constraint::Length(BODY_PAD_ROWS),
        Constraint::Min(1),
        Constraint::Length(bottom.height()),
    ])
    .areas(area);

    app.render_width = body.width;
    if app.log.is_empty() {
        launch::draw(frame, body, app);
    } else {
        let visible = app.transcript_view(body.height as usize);
        transcript::draw_log(frame, body, visible);
    }

    bottom.draw(frame, bottom_area, app);
}
