//! The full-window review (ADR 0009 §4): the design's `ReviewHeader`,
//! `FileTree`, `DiffRow` and `CommentField`, laid out as frames G–I:
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
//! No stroke anywhere: every boundary is a change of ground.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::chrome::{self, Action, Composer};
use super::grid::{
    elide, justified, Ctx, BODY_X, GUTTER_LN, MARGIN_X, MARK_COL, PANE_GAP, SIGN_COL, TREE_W,
};
use super::question;
use super::wrap::wrap_line;
use crate::app::App;
use crate::draft::expand_tabs;
use crate::log::{plural, LogEntry};
use crate::palette::Palette;
use crate::review::{DiffRow, Pane, Review, Tree};

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let pal = app.theme.palette();
    let Some(review) = app.review() else { return };
    let (title, summary) = (review.title.clone(), review_summary(app));

    // Bottom band height: the discard question, the comment field or the
    // field; must match the draw below.
    let typed = !app.draft.text().trim().is_empty();
    let action_width = action_for(review, typed).map_or(0, |a| a.width());
    let field_rows: u16 = match (&review.confirm, review.commenting()) {
        (Some(list), _) => question::panel_rows(&review.discard_question(), Some(list), area.width),
        (None, true) => 2,
        (None, false) => Composer::new(app.draft.text(), area.width, action_width).height(),
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
    // Frame G: the summary is on the prose column.
    let prose = Ctx::new(pal, area.width).body().width as usize;
    let summary_line = Line::from(vec![
        Span::raw(" ".repeat(BODY_X)),
        Span::styled(elide(&summary, prose), Style::default().fg(pal.label2)),
    ]);
    frame.render_widget(Paragraph::new(summary_line), summary_row);

    let tree_w = (TREE_W as u16).min(body.width / 2);
    let [tree, _, diff, _] = Layout::horizontal([
        Constraint::Length(tree_w),
        Constraint::Length(PANE_GAP as u16),
        Constraint::Min(1),
        Constraint::Length(MARGIN_X as u16),
    ])
    .areas(body);
    let tree = draw_tree(frame, tree, review, pal);
    let (pane, saw_bottom) = draw_diff(frame, diff, review, pal);
    if let Some(review) = app.review_mut() {
        review.scroll = pane.top;
        review.pane = Some(pane);
        review.tree = Some(tree);
        if saw_bottom {
            review.mark_read();
        }
    }
    let Some(review) = app.review() else { return };

    match (&review.confirm, review.commenting()) {
        (Some(list), _) => question::draw_panel(
            frame,
            field_area,
            &review.discard_question(),
            Some(list),
            pal,
        ),
        (None, true) => {
            let (lines, location) = review.selection_label().unwrap_or_default();
            chrome::draw_comment_field(
                frame,
                field_area,
                app,
                &lines,
                &location,
                review.comment.text(),
                review.comment.cursor(),
            );
        }
        (None, false) => {
            let action = action_for(review, typed);
            let composer = Composer::new(app.draft.text(), area.width, action_width);
            chrome::draw_field(frame, field_area, app, &composer, action);
        }
    }
    chrome::draw_footer(frame, footer, app);
}

/// The field's action, naming what `⌃↩` will do: send the comments (a typed
/// draft counts as one), else approve once every file is read. When not
/// ready its words say what it waits for, so colour is never the only
/// signal (HIG, "Accessibility"). None while the comments are with the
/// agent: the footer says the turn is working.
fn action_for(review: &Review, typed: bool) -> Option<Action> {
    if review.waiting() {
        return None;
    }
    let comments = review.comment_count() + usize::from(typed);
    let (label, ready) = if comments > 0 {
        (format!("Send {}", plural(comments, "Comment")), true)
    } else if review.all_read() {
        ("Approve".to_string(), true)
    } else {
        (
            format!("Approve after reading {}", plural(review.unread(), "file")),
            false,
        )
    };
    Some(Action {
        label,
        key: "⌃↩",
        ready,
    })
}

/// The last non-blank line of the agent's last prose.
fn review_summary(app: &App) -> String {
    app.log
        .iter()
        .rev()
        .find_map(|e| match e {
            LogEntry::AssistantText { text } => text.lines().rev().find(|l| !l.trim().is_empty()),
            _ => None,
        })
        .map(|line| line.trim().to_string())
        .unwrap_or_default()
}

/// The file tree on `--tint`: a progress dot per file, then folders and
/// files with a read `✓` or current `›`. Returns the `Tree` that clicks are
/// mapped against.
fn draw_tree(frame: &mut Frame, area: Rect, review: &Review, pal: &Palette) -> Tree {
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
    // Parallel to `lines`: the file each row shows.
    let mut files: Vec<Option<usize>> = vec![None];
    let mut dots = vec![Span::styled(" ".repeat(MARGIN_X), on_tint)];
    for file in review.files() {
        let (glyph, fg) = if file.read {
            ("●", pal.accent)
        } else {
            ("○", pal.label3)
        };
        dots.push(Span::styled(glyph, Style::default().fg(fg).bg(pal.tint)));
    }
    lines.push(fill(dots, pal.tint));
    lines.push(fill(vec![], pal.tint));
    files.extend([None, None]);

    let mut last_dir: Option<String> = None;
    for (i, file) in review.files().iter().enumerate() {
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
            files.push(None);
            last_dir = Some(dir);
        }
        files.push(Some(i));
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
    files.truncate(area.height as usize);
    Tree {
        x: area.x,
        y: area.y,
        width: area.width,
        files,
    }
}

/// The diff pane: a header of path and `+N −N`, then the rows. Returns the
/// `Pane` that mouse clicks are mapped against (ADR 0010), and whether the
/// last row was shown whole (which marks the file read).
fn draw_diff(frame: &mut Frame, area: Rect, review: &Review, pal: &Palette) -> (Pane, bool) {
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
    let height = (area.height as usize).saturating_sub(3);
    let selection = review.selection();
    // A comment is drawn once, on the last line of its range (frame I).
    let comment_for = |line: usize| {
        file.comments
            .iter()
            .find(|c| c.lines.1 == line)
            .map(|c| c.text.clone())
    };
    let drawn: Vec<Vec<Line<'static>>> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let selected = selection.is_some_and(|(a, b)| a <= i && i <= b);
            diff_row(row, selected, comment_for, ctx)
        })
        .collect();

    // The largest scroll offset that still fills the pane.
    let mut last_top = rows.len();
    let mut below = 0;
    while last_top > 0 && below + drawn[last_top - 1].len() <= height {
        last_top -= 1;
        below += drawn[last_top].len();
    }
    let top = review.scroll.min(last_top);

    let mut shown = Vec::new();
    let mut whole = None;
    for (i, row_lines) in drawn.iter().enumerate().skip(top) {
        let room = height - shown.len();
        if room == 0 {
            break;
        }
        if row_lines.len() <= room {
            whole = Some(i);
        }
        for line in row_lines.iter().take(room) {
            lines.push(line.clone());
            shown.push(i);
        }
    }
    let saw_bottom = rows.is_empty() || whole == Some(rows.len() - 1);
    let bottom = whole.unwrap_or(top);
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
    let pane = Pane {
        x: area.x,
        y: area.y + 3,
        width: area.width,
        lines: shown,
        top,
        bottom,
        last_top,
    };
    (pane, saw_bottom)
}

/// One diff row's screen rows: line number, sign and code on the row's own
/// ground. A selected row takes the accent `▎` in the gutter's first cell.
///
/// Long code wraps onto continuation rows with a blank gutter, against the
/// design (baseline `long-diff-lines-wrap`; the app follows the HIG). A
/// comment with no room on the last row gets a row of its own.
fn diff_row(
    row: &DiffRow,
    selected: bool,
    comment_for: impl Fn(usize) -> Option<String>,
    ctx: Ctx,
) -> Vec<Line<'static>> {
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
            expand_tabs(text),
            pal.label2,
            line.to_string(),
        ),
        DiffRow::Add { line, text } => (
            pal.addrow,
            "+",
            pal.add,
            expand_tabs(text),
            pal.addcode,
            line.to_string(),
        ),
        DiffRow::Del { text, .. } => (
            pal.delrow,
            "−",
            pal.del,
            expand_tabs(text),
            pal.delcode,
            String::new(),
        ),
    };
    let on_bg = |fg: Color| Style::default().fg(fg).bg(bg);
    // Continuation rows pass `("", "")`: only the selection edge shows.
    let gutter = |number: &str, sign: &str| {
        let mut spans = Vec::with_capacity(3);
        let digits = if selected {
            spans.push(Span::styled("▎", on_bg(pal.accent)));
            GUTTER_LN - 1
        } else {
            GUTTER_LN
        };
        spans.push(Span::styled(
            format!("{number:>digits$}"),
            on_bg(pal.label3),
        ));
        spans.push(Span::styled(
            format!("{sign:^width$}", width = SIGN_COL),
            on_bg(sign_fg),
        ));
        spans
    };
    let code_fg = if selected { pal.label } else { code_fg };
    let room = width.saturating_sub(GUTTER_LN + SIGN_COL).max(1);
    let chunks: Vec<String> = if matches!(row, DiffRow::Fold { .. }) {
        vec![elide(&code, room)]
    } else {
        wrap_line(Line::from(code), room)
            .into_iter()
            .map(|line| line.spans.into_iter().map(|s| s.content).collect())
            .collect()
    };

    let fill = |mut spans: Vec<Span<'static>>| {
        let used: usize = spans.iter().map(|s| s.content.width()).sum();
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used)),
            Style::default().bg(bg),
        ));
        Line::from(spans)
    };
    let mut lines: Vec<Line<'static>> = chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut spans = if i == 0 {
                gutter(&number, sign)
            } else {
                gutter("", "")
            };
            spans.push(Span::styled(chunk, on_bg(code_fg)));
            spans
        })
        .map(fill)
        .collect();

    let comment = row
        .anchor()
        .and_then(&comment_for)
        .filter(|_| !matches!(row, DiffRow::Fold { .. } | DiffRow::Del { .. }));
    if let Some(comment) = comment {
        let last = lines.last_mut().expect("a row draws at least one line");
        let code_w: usize = last.spans[..last.spans.len() - 1]
            .iter()
            .map(|s| s.content.width())
            .sum();
        let free = width.saturating_sub(code_w);
        // Frame I keeps one blank cell after a comment: the text is
        // shortened to leave it, never the pad dropped to fit the text.
        let tag = |room: usize| {
            format!(
                "{} ",
                elide(&format!("◆ {comment}"), room.saturating_sub(1))
            )
        };
        // Room for `◆`, a character and the gap before it, else a new row.
        if free >= 2 + 3 {
            last.spans.pop();
            let tag = tag(free - 2);
            last.spans.push(Span::styled(
                " ".repeat(free - tag.width()),
                Style::default().bg(bg),
            ));
            last.spans.push(Span::styled(tag, on_bg(pal.accent)));
        } else {
            let mut spans = gutter("", "");
            let used: usize = spans.iter().map(|s| s.content.width()).sum();
            let tag = tag(width.saturating_sub(used));
            spans.push(Span::styled(
                " ".repeat(width.saturating_sub(used + tag.width())),
                Style::default().bg(bg),
            ));
            spans.push(Span::styled(tag, on_bg(pal.accent)));
            lines.push(Line::from(spans));
        }
    }
    lines
}
