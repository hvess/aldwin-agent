//! The full-window review (ADR 0009 §4) — the design's `ReviewHeader`,
//! `FileTree`, `DiffRow` and `CommentField`, laid out as the four `G`–`I`
//! frames draw them:
//!
//! ```text
//! blank
//! title (600)                         Nothing is saved until you approve
//! summary
//! blank
//! ┆ tree 28ch on --tint ┆ 4ch ┆ diff pane …                   ┆ 3ch
//! blank
//! field  (or the two-row comment field)
//! blank
//! footer
//! blank
//! ```
//!
//! No stroke anywhere: the tree is its ground, the current row is
//! `--field`, a selection is the accent `▎` edge and brighter text, and
//! added and removed rows keep their own grounds.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::chrome::{self, Action, Composer};
use super::grid::{
    elide, justified, Ctx, BODY_X, GUTTER_LN, MARGIN_X, MARK_COL, PANE_GAP, SIGN_COL, TREE_W,
};
use super::question;
use crate::app::{App, Asking};
use crate::list::{List, ListRow};
use crate::review::{DiffRow, Pane, Review};
use aldwin_core::Question;

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let pal = app.theme.palette();
    let Some(review) = app.review() else { return };
    let (title, summary) = (review_title(app), review_summary(app));

    // The bottom band: the field, or the comment field, or the discard
    // question.
    let commenting = review.comment.is_some();
    let confirming = review.confirm_discard;
    let discard_question = confirming.then(|| discard_asking(review));
    let field_rows: u16 = match (&discard_question, commenting) {
        (Some(asking), _) => question::panel_rows(asking, area.width),
        (None, true) => 2,
        (None, false) => Composer::new(&app.input, area.width, action_for(review).width()).height(),
    };
    let [_, title_row, summary_row, _, body, _, field_area, _, footer, _] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(field_rows),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    // Header.
    let width = area.width as usize;
    let right = "Nothing is saved until you approve";
    let title_line = justified(
        vec![
            Span::raw(" ".repeat(BODY_X)),
            Span::styled(
                elide(
                    &title,
                    width.saturating_sub(BODY_X + MARGIN_X + right.width() + 2),
                ),
                Style::default().fg(pal.label).add_modifier(Modifier::BOLD),
            ),
        ],
        vec![
            Span::styled(right, Style::default().fg(pal.label2)),
            Span::raw(" ".repeat(MARGIN_X)),
        ],
        width,
    );
    frame.render_widget(Paragraph::new(title_line), title_row);
    // Frame G: the summary is `padding: 0 5ch`, the prose column.
    let prose = Ctx::new(pal, area.width).body().width as usize;
    let summary_line = Line::from(vec![
        Span::raw(" ".repeat(BODY_X)),
        Span::styled(elide(&summary, prose), Style::default().fg(pal.label2)),
    ]);
    frame.render_widget(Paragraph::new(summary_line), summary_row);

    // Two panes.
    let tree_w = (TREE_W as u16).min(body.width / 2);
    let [tree, _, diff, _] = Layout::horizontal([
        Constraint::Length(tree_w),
        Constraint::Length(PANE_GAP as u16),
        Constraint::Min(1),
        Constraint::Length(MARGIN_X as u16),
    ])
    .areas(body);
    draw_tree(frame, tree, review, pal);
    let (pane, saw_bottom) = draw_diff(frame, diff, review, pal);
    if let Some(review) = app.review_mut() {
        review.scroll = pane.top;
        review.pane = Some(pane);
        if saw_bottom {
            review.mark_read();
        }
    }
    let Some(review) = app.review() else { return };

    // The bottom band.
    match (discard_question, commenting) {
        (Some(asking), _) => question::draw_panel(frame, field_area, &asking, pal),
        (None, true) => {
            let draft = review.comment.clone().unwrap_or_default();
            let (lines, location) = review.selection_label().unwrap_or_default();
            chrome::draw_comment_field(
                frame,
                field_area,
                app,
                &lines,
                &location,
                &draft.text,
                draft.cursor,
            );
        }
        (None, false) => {
            let action = action_for(review);
            let composer = Composer::new(&app.input, area.width, action.width());
            chrome::draw_field(frame, field_area, app, &composer, Some(action));
        }
    }
    chrome::draw_footer(frame, footer, app);
}

/// `Approve  ⌃↩` — grey until every file is read — or `Send N Comments ⌃↩`
/// in accent once there is a comment to send.
fn action_for(review: &Review) -> Action {
    let comments = review.comment_count();
    if comments > 0 {
        Action {
            label: format!(
                "Send {comments} {}",
                if comments == 1 { "Comment" } else { "Comments" }
            ),
            key: "⌃↩",
            ready: true,
        }
    } else {
        Action {
            label: "Approve".into(),
            key: "⌃↩",
            ready: review.all_read(),
        }
    }
}

/// The last thing you asked for, as the review's title — sentence case, no
/// trailing stop.
fn review_title(app: &App) -> String {
    let text = app.log.iter().rev().find_map(|e| match e {
        crate::log::LogEntry::UserMessage { text } if !text.trim_start().starts_with('/') => {
            Some(text.lines().next().unwrap_or("").trim().to_string())
        }
        _ => None,
    });
    text.unwrap_or_else(|| "Changes".into())
        .trim_end_matches(['.', '!'])
        .to_string()
}

/// The agent's last sentence before the review opened.
fn review_summary(app: &App) -> String {
    let mut seen_ready = false;
    for e in app.log.iter().rev() {
        if let crate::log::LogEntry::AssistantText { text } = e {
            if !seen_ready && text.starts_with("Ready for you to review") {
                seen_ready = true;
                continue;
            }
            if let Some(line) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                return line.trim().to_string();
            }
        }
    }
    String::new()
}

fn discard_asking(review: &Review) -> Asking {
    let n = review.files.len();
    let files = format!("Discard {n} {}", if n == 1 { "file" } else { "files" });
    Asking {
        question: Question {
            question: "Discard these changes?".into(),
            detail: "Nothing has been written. The agent is told.".into(),
            options: vec!["Keep reviewing".into(), files.clone()],
        },
        list: List::new(vec![ListRow::new("Keep reviewing"), ListRow::new(files)]),
        asker: crate::app::Asker::Session,
    }
}

/// The file tree on `--tint`: a blank row, the progress dots at the
/// margin, a blank row, then folders in `label3` and files with their read
/// `✓` or current `›` centred in a 3-cell column.
fn draw_tree(frame: &mut Frame, area: Rect, review: &Review, pal: &crate::palette::Palette) {
    let on_tint = Style::default().bg(pal.tint);
    frame.render_widget(Block::new().style(on_tint), area);
    let width = area.width as usize;
    let fill = |spans: Vec<Span<'static>>, bg| {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        let mut spans = spans;
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used)),
            Style::default().bg(bg),
        ));
        Line::from(spans)
    };

    let mut lines: Vec<Line<'static>> = vec![fill(vec![], pal.tint)];
    let mut dots = vec![Span::styled(" ".repeat(MARGIN_X), on_tint)];
    for file in &review.files {
        let (glyph, fg) = if file.read {
            ("●", pal.accent)
        } else {
            ("○", pal.label3)
        };
        dots.push(Span::styled(glyph, Style::default().fg(fg).bg(pal.tint)));
    }
    lines.push(fill(dots, pal.tint));
    lines.push(fill(vec![], pal.tint));

    let mut last_dir: Option<String> = None;
    for (i, file) in review.files.iter().enumerate() {
        let (dir, name) = match file.path.rsplit_once('/') {
            Some((d, n)) => (d.to_string(), n.to_string()),
            None => (String::new(), file.path.clone()),
        };
        if last_dir.as_deref() != Some(dir.as_str()) && !dir.is_empty() {
            lines.push(fill(
                vec![
                    Span::styled(" ".repeat(MARGIN_X), on_tint),
                    Span::styled(
                        elide(&dir, width.saturating_sub(MARGIN_X + 1)),
                        Style::default().fg(pal.label3).bg(pal.tint),
                    ),
                ],
                pal.tint,
            ));
            last_dir = Some(dir);
        }
        let current = i == review.current;
        let bg = if current { pal.field } else { pal.tint };
        let (glyph, glyph_fg) = if current {
            ("›", pal.accent)
        } else if file.read {
            ("✓", pal.label3)
        } else {
            (" ", pal.label3)
        };
        let name_fg = if current { pal.label } else { pal.label2 };
        let mut spans = vec![
            Span::styled(format!(" {glyph} "), Style::default().fg(glyph_fg).bg(bg)),
            Span::styled(" ".repeat(MARK_COL), Style::default().bg(bg)),
            Span::styled(
                elide(&name, width.saturating_sub(MARGIN_X + MARK_COL + 8)),
                Style::default().fg(name_fg).bg(bg),
            ),
        ];
        if file.added {
            spans.push(Span::styled(" +", Style::default().fg(pal.add).bg(bg)));
        }
        let comments = file.comments.len();
        if comments > 0 {
            let used: usize = spans.iter().map(|s| s.content.width()).sum();
            let tag = format!("◆ {comments}");
            let gap = width
                .saturating_sub(used)
                .saturating_sub(tag.width() + MARK_COL);
            spans.push(Span::styled(" ".repeat(gap), Style::default().bg(bg)));
            spans.push(Span::styled(tag, Style::default().fg(pal.accent).bg(bg)));
        }
        lines.push(fill(spans, bg));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)).style(on_tint), area);
}

/// The diff pane: a blank row, the path in weight 600 with `+11 −2` flush
/// right, a blank row, then the rows. Returns where the rows landed — the
/// scroll it settled on, and the rect a click is measured against — and
/// whether the last row was on screen.
fn draw_diff(
    frame: &mut Frame,
    area: Rect,
    review: &Review,
    pal: &crate::palette::Palette,
) -> (Pane, bool) {
    let file = review.file();
    let width = area.width as usize;
    let ctx = Ctx::new(pal, area.width);

    let header = justified(
        vec![Span::styled(
            elide(&file.path, width.saturating_sub(12)),
            Style::default().fg(pal.label).add_modifier(Modifier::BOLD),
        )],
        vec![
            Span::styled(
                format!("+{}", file.added_lines),
                Style::default().fg(pal.add),
            ),
            Span::raw(" "),
            Span::styled(
                format!("−{}", file.removed_lines),
                Style::default().fg(pal.del),
            ),
        ],
        width,
    );
    let mut lines: Vec<Line<'static>> = vec![Line::default(), header, Line::default()];

    let rows = file.rows();
    let pane = (area.height as usize).saturating_sub(3);
    let top = review.scroll.min(rows.len().saturating_sub(pane));
    let selection = review.selection();
    // A comment rides at the end of the *last* line of its range, once —
    // frame `I` puts `◆ Use config` on 145 of a 144–145 comment.
    let comment_for = |line: usize| {
        file.comments
            .iter()
            .find(|c| c.lines.1 == line)
            .map(|c| c.text.clone())
    };

    for (i, row) in rows.iter().enumerate().skip(top).take(pane) {
        let selected = selection.is_some_and(|(a, b)| a <= i && i <= b);
        lines.push(diff_row(row, selected, comment_for, ctx));
    }
    let saw_bottom = top + pane >= rows.len();
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
    let rows_at = Pane {
        x: area.x,
        y: area.y + 3,
        width: area.width,
        height: pane as u16,
        top,
    };
    (rows_at, saw_bottom)
}

/// One diff row: the 5-cell line number, the 2-cell sign, the code — on the
/// row's own ground. Every number is `label3`, as frames G–I draw them.
/// Selected rows take the accent `▎` in the first cell, a 4-cell number and
/// `label` code. A comment rides at the end in accent, `◆ text`.
fn diff_row(
    row: &DiffRow,
    selected: bool,
    comment_for: impl Fn(usize) -> Option<String>,
    ctx: Ctx,
) -> Line<'static> {
    let pal = ctx.pal;
    let width = ctx.width as usize;
    let (bg, sign, sign_fg, code, code_fg, number) = match row {
        DiffRow::Fold { len, .. } => (
            pal.win,
            "",
            pal.label3,
            format!("⋯  {len} lines"),
            pal.label3,
            String::new(),
        ),
        DiffRow::Context { line, text } => (
            pal.win,
            "",
            pal.label3,
            text.clone(),
            pal.label2,
            line.to_string(),
        ),
        DiffRow::Add { line, text } => (
            pal.addrow,
            "+",
            pal.add,
            text.clone(),
            pal.addcode,
            line.to_string(),
        ),
        DiffRow::Del { text, .. } => (
            pal.delrow,
            "−",
            pal.del,
            text.clone(),
            pal.delcode,
            String::new(),
        ),
    };
    let number_fg = pal.label3;
    let mut spans: Vec<Span<'static>> = Vec::new();
    if selected {
        spans.push(Span::styled("▎", Style::default().fg(pal.accent).bg(bg)));
        spans.push(Span::styled(
            format!("{number:>width$}", width = GUTTER_LN - 1),
            Style::default().fg(number_fg).bg(bg),
        ));
    } else {
        spans.push(Span::styled(
            format!("{number:>width$}", width = GUTTER_LN),
            Style::default().fg(number_fg).bg(bg),
        ));
    }
    spans.push(Span::styled(
        format!("{sign:^width$}", width = SIGN_COL),
        Style::default().fg(sign_fg).bg(bg),
    ));
    let comment = row
        .anchor()
        .and_then(&comment_for)
        .filter(|_| !matches!(row, DiffRow::Fold { .. } | DiffRow::Del { .. }));
    let code_fg = if selected { pal.label } else { code_fg };
    // "The code is never broken up": the code takes what it needs first,
    // and the comment rides in what is left — elided, or dropped when there
    // is not even room for its glyph.
    let room = width.saturating_sub(GUTTER_LN + SIGN_COL);
    spans.push(Span::styled(
        elide(&code.replace('\t', "    "), room),
        Style::default().fg(code_fg).bg(bg),
    ));
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let free = width.saturating_sub(used);
    match comment.filter(|_| free >= 2 + 3) {
        Some(c) => {
            let tag = elide(&format!("◆ {c} "), free - 2);
            spans.push(Span::styled(
                " ".repeat(free - tag.width()),
                Style::default().bg(bg),
            ));
            spans.push(Span::styled(tag, Style::default().fg(pal.accent).bg(bg)));
        }
        None => spans.push(Span::styled(" ".repeat(free), Style::default().bg(bg))),
    }
    Line::from(spans)
}
