use mjolnir_permissions::PromptPayload;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, PermState};
use crate::highlight;
use crate::log::{LogEntry, ToolActivityStatus};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const BRIGHT: Color = Color::White;
// A dedicated LightGreen was tried first for user/assistant separation
// (see the git history) but read as too loud against real terminal color
// schemes, per explicit developer feedback — swapped for a muted gray text
// color plus a subtle background tint, which separates user input from
// both assistant text (BRIGHT, no bg) and dim metadata without fighting
// the terminal's own palette. Fixed RGB rather than a named ANSI color so
// the "subtle" tint doesn't get reinterpreted by whatever the terminal
// theme maps that ANSI slot to.
const USER_FG: Color = Color::Rgb(190, 190, 195);
const USER_BG: Color = Color::Rgb(40, 40, 46);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let input_height = input_area_height(&app.input);
    let [log_area, status_area, input_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1), Constraint::Length(input_height)]).areas(area);

    let log_inner_height = log_area.height as usize;
    app.scroll.set_viewport_height(log_inner_height, app.total_lines());

    draw_log(frame, log_area, app);
    draw_status(frame, status_area, app);
    draw_input(frame, input_area, app);
}

fn input_area_height(input: &str) -> u16 {
    // +2 for the border; at least 3 total so a single-line draft still gets
    // a visible box, matching "multi-line textarea" without it collapsing
    // to a single row when empty.
    let lines = input.matches('\n').count() as u16 + 1;
    (lines + 2).max(3)
}

fn draw_log(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = intro_lines(&app.status.model_name, area.width);
    // Separates the banner from the first real entry, same as the
    // inter-entry separator below — skipped when the log is still empty so
    // a fresh session doesn't end in a trailing blank line.
    if !app.log.is_empty() {
        lines.push(Line::default());
    }
    for (i, entry) in app.log.iter().enumerate() {
        // Blank line between entries — not just at the user/assistant
        // boundary, since every entry kind benefits from more breathing
        // room, per explicit feedback that the log felt visually cramped.
        if i > 0 {
            lines.push(Line::default());
        }
        lines.extend(render_entry(entry, area.width));
    }
    if app.thinking {
        lines.push(Line::from(Span::styled("thinking…", Style::default().fg(DIM))));
    }

    let visible: Vec<Line> = lines.into_iter().skip(app.scroll.offset).collect();
    let paragraph = Paragraph::new(Text::from(visible)).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// The welcome banner shown above the conversation log on every draw: the
/// "MJOLNIR" wordmark, then a Mjolnir (Thor's hammer) mark beside the
/// tagline/version line — a mascot pivot away from mjolnir.md's
/// originally-decided little owl, per explicit developer direction toward
/// something more "aggressive/directive" (see mjolnir.md's Mascot section
/// for the superseded rationale, and this function's own history for the
/// several prior hammer designs: two procedural crosshatch-weave textures,
/// then this literal photo trace rendered as Braille dots). `MJOLNIR_ART`
/// is a literal trace of a real reference photo (thresholded, trimmed,
/// resized preserving aspect, Gaussian-blurred before thresholding so fine
/// knotwork linework survives as continuous strokes instead of fragmenting
/// into speckle, read back one source pixel per Braille dot — 2×4 real
/// sub-character dots per cell) rather than a hand-drawn or
/// procedurally-generated shape — per explicit developer feedback that
/// procedural attempts "weren't a true representation" of the reference
/// images supplied. `WORDMARK_ART` is FIGlet's "Whimsy" font (`-k` kerning
/// layout — plain smushing ran the letters together), found by rendering
/// "MJOLNIR" through the ~370-font xero/figlet-fonts collection and
/// grepping for a fragment the developer pasted as their preferred
/// reference, after two earlier wordmark attempts (hand-drawn angular
/// block letters, then FIGlet's "Colossal") — the developer wanted a real
/// existing font, not another from-scratch design, and Whimsy specifically
/// once they saw it. See this crate's git history for the generating
/// scripts; neither is kept in the repo since they're one-time art
/// pipelines, not runtime code. Always exactly `log::INTRO_LINE_COUNT`
/// lines — that constant is a plain `usize` (not derived from this
/// function) so `App::total_lines` can stay ratatui-free per
/// `log::line_count`'s doc comment; keep the two in sync by hand if either
/// array or the border changes shape. Styled uniformly ACCENT+BOLD — a
/// traced silhouette has no shading gradient to speak of, so per-glyph
/// styling would be pointless; ACCENT is still the one deliberate
/// expansion of accent beyond "card border and focused input only" (see
/// the Palette Progress note in mjolnir-tui.md).
const WORDMARK_ART: [&str; 10] = [
    "               d8,          d8b             d8,        ",
    "              `8P           88P            `8P         ",
    "                           d88                         ",
    "  88bd8b,d88b d88   d8888b 888    88bd88b   88b 88bd88b",
    "  88P'`?8P'?8b?88  d8P' ?88?88    88P' ?8b  88P 88P'  `",
    " d88  d88  88P 88b 88b  d88 88b  d88   88P d88 d88     ",
    "d88' d88'  88b `88b`?8888P'  88bd88'   88bd88'd88'     ",
    "                )88                                    ",
    "               ,88P                                    ",
    "            `?888P                                     ",
];

const MJOLNIR_ART: [&str; 16] = [
    "⠀⠀⠀⠀⠀⠀⣠⡶⠒⣺⣿⣿⣉⣏⣉⣿⣿⣗⠒⣦⡄⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⣿⠇⡾⢋⡭⣍⠻⣿⠟⡩⢭⡙⣷⢸⣿⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⣿⡀⢷⡘⠒⣨⡿⢡⢾⡀⠚⢁⡟⢠⣼⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠘⠳⣄⠉⠛⢉⣴⢿⣦⡙⠛⠋⡠⠞⠁⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠈⡆⢀⣤⣙⡿⢋⣤⡀⢸⠁⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⡇⠘⣿⠋⣤⠙⣿⠃⢸⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⡇⢸⣿⡶⠉⢴⣿⡇⢸⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⢰⠃⣼⢿⣐⠿⢀⡿⣧⠸⡄⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⢀⡞⢰⣇⠚⣡⣶⣍⠃⣸⡄⢳⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⣀⣀⣀⣀⣀⣠⡾⠤⠾⠿⠛⠛⠛⠛⠛⠿⠷⠤⢷⣤⣄⣀⣀⣤⣄⠀",
    "⢸⠁⣴⣤⣤⠆⠀⠲⣶⣶⠟⢛⣉⣙⠛⢿⣿⡶⢂⣠⣶⣶⢶⣶⣶⠀⡇",
    "⣸⢀⣿⣿⣇⠸⠟⣷⢸⠃⣼⠋⣭⡍⢳⡈⣿⡇⣾⡿⠛⣛⣛⠛⢿⡄⣧",
    "⡟⠘⢛⣉⣙⠓⢚⣡⣾⡀⣿⡘⠿⠿⠿⢠⣿⣷⣈⠓⠛⣋⣽⣿⣦⠀⢸",
    "⠛⠤⠤⠤⢭⣉⡙⠻⠿⣷⣌⠛⠶⠶⠾⠟⣋⣴⡿⠟⢛⣉⣩⠭⠤⠤⠞",
    "⠀⠀⠀⠀⠀⠀⠈⠙⠒⠤⣉⠛⢷⣶⡶⠟⣉⡤⠖⠋⠉⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠙⠢⣄⡴⠋⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
];

/// Every `MJOLNIR_ART` row is exactly this many chars (not trimmed of
/// trailing blank Braille cells), so the info column in `intro_lines`
/// starts at the same screen column on every row regardless of how much
/// art content that particular row has. `WORDMARK_ART` doesn't need this
/// — nothing sits beside it — so its rows aren't held to a matching
/// invariant.
const MJOLNIR_ART_WIDTH: usize = 27;

fn intro_lines(model_name: &str, width: u16) -> Vec<Line<'static>> {
    debug_assert!(
        MJOLNIR_ART.iter().all(|row| row.chars().count() == MJOLNIR_ART_WIDTH),
        "MJOLNIR_ART rows must stay fixed-width or the info column drifts off-alignment — see every_mjolnir_art_row_is_exactly_mjolnir_art_width_chars"
    );
    let frame = Style::default().fg(ACCENT);
    let wordmark_style = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let art_style = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let tagline = Style::default().fg(BRIGHT).add_modifier(Modifier::ITALIC);
    let meta = Style::default().fg(DIM);

    // Beside the art, not below it — per explicit developer direction.
    // Vertically centered against the art block's height.
    let info: [(String, Style); 3] = [
        ("a tool for thought.".to_string(), tagline),
        (String::new(), meta),
        (format!("v{} ({}) · {model_name}", env!("CARGO_PKG_VERSION"), env!("MJOLNIR_GIT_HASH")), meta),
    ];
    let info_offset = (MJOLNIR_ART.len().saturating_sub(info.len())) / 2;

    let mut content: Vec<Line<'static>> = WORDMARK_ART.iter().map(|row| Line::from(Span::styled(*row, wordmark_style))).collect();
    content.push(Line::default());
    content.extend(MJOLNIR_ART.iter().enumerate().map(|(i, art_row)| {
        let mut spans = vec![Span::styled(*art_row, art_style)];
        if let Some(row_i) = i.checked_sub(info_offset) {
            if let Some((text, style)) = info.get(row_i) {
                spans.push(Span::raw("   "));
                spans.push(Span::styled(text.clone(), *style));
            }
        }
        Line::from(spans)
    }));
    bordered(width, content, frame)
}

/// Wraps `content` in a border that spans the full render width (`width`,
/// the log area's actual `Rect::width` — art alone can't know this, so it's
/// threaded in from `draw_log` at render time), left-aligning each line
/// with a small fixed margin rather than centering — per explicit
/// developer direction that the banner should read left-to-right (art,
/// then wordmark/info beside it), not sit centered in the middle of a wide
/// terminal. `content_width` sums `Span::content` char counts, which only
/// holds up for single-width glyphs — true of every char used here
/// (box-drawing and Braille dot patterns are Unicode East Asian Width
/// "Narrow"/"Neutral") but would need adjustment for wide (CJK/emoji) text.
fn bordered(width: u16, content: Vec<Line<'static>>, border_style: Style) -> Vec<Line<'static>> {
    const LEFT_MARGIN: usize = 2;
    let inner_width = (width as usize).saturating_sub(2);
    let mut out = Vec::with_capacity(content.len() + 2);
    out.push(Line::from(Span::styled(format!("┌{}┐", "─".repeat(inner_width)), border_style)));
    for line in content {
        let content_width: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        let avail = inner_width.saturating_sub(LEFT_MARGIN + 1); // trailing space before the right border
        let right_pad = avail.saturating_sub(content_width);
        let mut spans = vec![Span::styled(format!("│{}", " ".repeat(LEFT_MARGIN)), border_style)];
        spans.extend(line.spans);
        spans.push(Span::styled(format!("{} │", " ".repeat(right_pad)), border_style));
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(format!("└{}┘", "─".repeat(inner_width)), border_style)));
    out
}

fn render_entry(entry: &LogEntry, width: u16) -> Vec<Line<'static>> {
    match entry {
        // Palette per mjolnir-tui.md: bright = assistant, muted gray +
        // subtle background = user — these must not share a style, or the
        // two speakers become indistinguishable in the log. A slash command
        // is user input that never reaches the model (see mjolnir-cli's
        // interceptor) — dim marks it as directed at the harness itself,
        // not conversation, the same way tool metadata and notices are dim
        // (and it skips the background tint, since it isn't a chat message).
        LogEntry::UserMessage { text } => {
            let style = if is_command(text) { Style::default().fg(DIM) } else { Style::default().fg(USER_FG).bg(USER_BG) };
            // Padded to the full render width so the background tint reads
            // as a chat bubble even for a short message, not just a tinted
            // "> " prefix — per explicit developer feedback. Only exact for
            // a source line that fits on one screen row: a line long enough
            // to wrap under Paragraph's own Wrap{trim:false} gets this
            // padding appended past the wrap point, not per wrapped row.
            text.lines()
                .map(|l| {
                    let content = format!("> {l}");
                    let pad = (width as usize).saturating_sub(content.chars().count());
                    Line::from(Span::styled(format!("{content}{}", " ".repeat(pad)), style))
                })
                .collect()
        }
        LogEntry::AssistantText { text } => render_assistant_text(text),
        LogEntry::ToolActivity { calls, .. } => calls
            .iter()
            .map(|c| {
                let label = if c.name.is_empty() { c.call_id.clone() } else { format!("{} ({})", c.name, c.call_id) };
                let text = match &c.status {
                    ToolActivityStatus::Running => format!("  [running] {label}"),
                    ToolActivityStatus::Completed { is_error, summary } => {
                        let tag = if *is_error { "error" } else { "done" };
                        format!("  [{tag}] {label}: {summary}")
                    }
                };
                Line::from(Span::styled(text, Style::default().fg(DIM)))
            })
            .collect(),
        LogEntry::RetryAttempt { info } => {
            let status = info.status.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());
            vec![Line::from(Span::styled(
                format!("  [retry {}] {} {status}: {}", info.attempt, info.provider, info.message),
                Style::default().fg(DIM),
            ))]
        }
        LogEntry::ApprovalCard { diff, resolution, .. } => render_card(
            "Approve this edit?",
            diff,
            "[y] approve   [n] deny   [Ctrl+C] deny",
            resolution.map(|approved| if approved { "approved".to_string() } else { "denied".to_string() }),
        ),
        LogEntry::PermissionPrompt { payload, resolution, .. } => render_prompt_card(payload, resolution.as_deref()),
        LogEntry::TurnEnded { reason } => {
            use crate::log::TurnEndReasonKind;
            let text = match reason {
                TurnEndReasonKind::EndTurn => "— turn ended —".to_string(),
                TurnEndReasonKind::Cancelled => "— turn cancelled —".to_string(),
                TurnEndReasonKind::Error(message) => format!("— turn ended in error: {message} —"),
            };
            vec![Line::from(Span::styled(text, Style::default().fg(DIM)))]
        }
        LogEntry::Error { message } => vec![Line::from(Span::styled(format!("error: {message}"), Style::default().fg(Color::Red)))],
        LogEntry::Notice { message } => vec![Line::from(Span::styled(format!("— {message} —"), Style::default().fg(DIM)))],
    }
}

/// One piece of assistant text — either prose or a fenced code block.
enum Segment {
    Prose(String),
    Code { lang: String, body: String },
}

/// Splits on ` ``` ` fences (optionally followed by a language tag on the
/// opening fence). An unterminated fence — the closing ` ``` ` hasn't
/// streamed in yet — still renders as code up to the end of the buffer
/// rather than falling back to prose, since re-rendering happens on every
/// delta and the fence will close on a later redraw.
fn split_code_fences(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut prose = String::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        match line.trim_start().strip_prefix("```") {
            Some(lang) => {
                if !prose.is_empty() {
                    segments.push(Segment::Prose(std::mem::take(&mut prose)));
                }
                let mut body = String::new();
                for line in lines.by_ref() {
                    if line.trim() == "```" {
                        break;
                    }
                    body.push_str(line);
                    body.push('\n');
                }
                segments.push(Segment::Code { lang: lang.trim().to_string(), body });
            }
            None => {
                prose.push_str(line);
                prose.push('\n');
            }
        }
    }
    if !prose.is_empty() {
        segments.push(Segment::Prose(prose));
    }
    segments
}

fn render_assistant_text(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for segment in split_code_fences(text) {
        match segment {
            Segment::Prose(s) => {
                lines.extend(s.lines().map(render_markdown_line));
            }
            Segment::Code { lang, body } => {
                let label = if lang.is_empty() { "code".to_string() } else { lang.clone() };
                lines.push(Line::from(Span::styled(format!("┌─ {label}"), Style::default().fg(DIM))));
                for code_line in highlight::highlight_lines(&lang, &body) {
                    let mut spans = vec![Span::styled("│ ", Style::default().fg(DIM))];
                    spans.extend(code_line);
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(Span::styled("└─", Style::default().fg(DIM))));
            }
        }
    }
    // A single marker on the very first rendered line — prose or a code
    // fence's header, whichever comes first — scans as "here's where the
    // assistant's turn starts" without repeating on every line.
    if let Some(first) = lines.first_mut() {
        first.spans.insert(0, Span::styled("● ", Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)));
    }
    lines
}

/// Renders one prose line (never a fenced-code line — those are already
/// pulled out by `split_code_fences`) of LLM-authored markdown. Hand-rolled
/// rather than pulling in a CommonMark crate: `log::line_count`'s scroll-math
/// invariant depends on exactly one rendered `Line` per source line, and a
/// real block-level parser normalizes blank lines and reflows paragraphs,
/// breaking that guarantee. Per-line block-prefix detection (heading, list,
/// blockquote, rule) plus a recursive-descent inline pass covers what LLMs
/// actually emit without touching line count. Styling is modifiers only
/// (bold/italic/underline/reversed/crossed-out) — mjolnir-tui.md reserves
/// the one accent color for the approval card and focused input.
fn render_markdown_line(line: &str) -> Line<'static> {
    let base = Style::default().fg(BRIGHT);
    let trimmed_start = line.trim_start();
    let indent = &line[..line.len() - trimmed_start.len()];

    if is_hr(trimmed_start) {
        return Line::from(Span::styled("─".repeat(20), Style::default().fg(DIM)));
    }
    if let Some((level, rest)) = parse_heading(trimmed_start) {
        let style = if level <= 2 { base.add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { base.add_modifier(Modifier::BOLD) };
        return Line::from(parse_inline(rest, style));
    }
    if let Some(rest) = trimmed_start.strip_prefix('>') {
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        let mut spans = vec![Span::styled(format!("{indent}▎ "), Style::default().fg(DIM))];
        spans.extend(parse_inline(rest, base.add_modifier(Modifier::ITALIC)));
        return Line::from(spans);
    }
    if let Some(rest) = parse_bullet(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}• "), base)];
        spans.extend(parse_inline(rest, base));
        return Line::from(spans);
    }
    if let Some((marker, rest)) = parse_ordered(trimmed_start) {
        let mut spans = vec![Span::styled(format!("{indent}{marker} "), base)];
        spans.extend(parse_inline(rest, base));
        return Line::from(spans);
    }
    Line::from(parse_inline(line, base))
}

/// Recursive-descent inline pass: `**bold**`, `*italic*`/`_italic_`,
/// `` `code` ``, `~~strike~~`, `[text](url)`. Delimiters nest via recursion
/// (e.g. `**bold *and italic***`) rather than a flat token stream, which
/// keeps this a single small function instead of a tokenizer + AST.
fn parse_inline(text: &str, base: Style) -> Vec<Span<'static>> {
    fn flush(buf: &mut String, style: Style, spans: &mut Vec<Span<'static>>) {
        if !buf.is_empty() {
            spans.push(Span::styled(std::mem::take(buf), style));
        }
    }

    let mut spans = Vec::new();
    let mut buf = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('`') {
            if let Some(end) = stripped.find('`') {
                flush(&mut buf, base, &mut spans);
                spans.push(Span::styled(stripped[..end].to_string(), base.add_modifier(Modifier::REVERSED)));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("**") {
            if let Some(end) = stripped.find("**") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::BOLD)));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if let Some(stripped) = rest.strip_prefix("~~") {
            if let Some(end) = stripped.find("~~") {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::CROSSED_OUT)));
                rest = &stripped[end + 2..];
                continue;
            }
        } else if rest.starts_with('*') || rest.starts_with('_') {
            let delim = &rest[..1];
            let stripped = &rest[1..];
            if let Some(end) = stripped.find(delim) {
                flush(&mut buf, base, &mut spans);
                spans.extend(parse_inline(&stripped[..end], base.add_modifier(Modifier::ITALIC)));
                rest = &stripped[end + 1..];
                continue;
            }
        } else if rest.starts_with('[') {
            if let Some((label, url, remainder)) = parse_link(rest) {
                flush(&mut buf, base, &mut spans);
                spans.push(Span::styled(label.to_string(), base.add_modifier(Modifier::UNDERLINED)));
                if !url.is_empty() && url != label {
                    spans.push(Span::styled(format!(" ({url})"), Style::default().fg(DIM)));
                }
                rest = remainder;
                continue;
            }
        }

        let ch_len = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        buf.push_str(&rest[..ch_len]);
        rest = &rest[ch_len..];
    }
    flush(&mut buf, base, &mut spans);
    spans
}

fn parse_link(text: &str) -> Option<(&str, &str, &str)> {
    let after_bracket = &text[1..];
    let close = after_bracket.find(']')?;
    let label = &after_bracket[..close];
    let after_label = &after_bracket[close + 1..];
    let after_paren = after_label.strip_prefix('(')?;
    let close_paren = after_paren.find(')')?;
    let url = &after_paren[..close_paren];
    let remainder = &after_paren[close_paren + 1..];
    Some((label, url, remainder))
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    line[hashes..].strip_prefix(' ').map(|rest| (hashes as u8, rest))
}

fn parse_bullet(line: &str) -> Option<&str> {
    let marker = line.chars().next()?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }
    line[marker.len_utf8()..].strip_prefix(' ')
}

fn parse_ordered(line: &str) -> Option<(String, &str)> {
    let digits_end = line.find(|c: char| !c.is_ascii_digit()).unwrap_or(0);
    if digits_end == 0 {
        return None;
    }
    let (digits, rest) = line.split_at(digits_end);
    let sep = rest.chars().next()?;
    if sep != '.' && sep != ')' {
        return None;
    }
    let rest = rest[sep.len_utf8()..].strip_prefix(' ')?;
    Some((format!("{digits}{sep}"), rest))
}

/// A line of 3+ `-`, `*`, or `_` (ignoring interior spaces, so `- - -`
/// counts) and nothing else — CommonMark's thematic break.
fn is_hr(line: &str) -> bool {
    let stripped: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    stripped.len() >= 3 && (stripped.chars().all(|c| c == '-') || stripped.chars().all(|c| c == '*') || stripped.chars().all(|c| c == '_'))
}

/// Mirrors mjolnir-cli's own `/`-prefix check (`text.trim_start().strip_prefix('/')`
/// in `slash.rs`) — this crate can't depend on that one to reuse it
/// directly (cli depends on tui, not the other way around), so the rule is
/// duplicated; keep the two in sync if it ever changes.
fn is_command(text: &str) -> bool {
    text.trim_start().starts_with('/')
}

fn render_prompt_card(payload: &PromptPayload, resolution: Option<&str>) -> Vec<Line<'static>> {
    let (title, keys) = match payload {
        PromptPayload::Tool { kind, target } => {
            (format!("Allow {kind}: {target}?"), "[o]nce [s]ession [p]roject [a]lways   Shift = deny at the same tier   Ctrl+C = deny once".to_string())
        }
        PromptPayload::ContextFile { path } => (format!("Inject context file {}?", path.display()), "[s]ession [p]roject [n]o   Ctrl+C = no".to_string()),
        PromptPayload::Edit { kind } => (format!("Edit approval for {kind}"), String::new()),
    };
    render_card(&title, "", &keys, resolution.map(str::to_string))
}

fn render_card(title: &str, body: &str, keys: &str, resolution: Option<String>) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(format!("┌─ {title}"), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))];
    for l in body.lines() {
        lines.push(Line::from(Span::styled(format!("│ {l}"), Style::default().fg(BRIGHT))));
    }
    match resolution {
        Some(r) => lines.push(Line::from(Span::styled(format!("└─ resolved: {r}"), Style::default().fg(ACCENT)))),
        None => lines.push(Line::from(Span::styled(format!("└─ {keys}"), Style::default().fg(ACCENT)))),
    }
    lines
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let s = &app.status;
    let turn_step = match (s.turn, s.step) {
        (Some(t), Some(st)) => format!("T{t} S{st}"),
        (Some(t), None) => format!("T{t}"),
        _ => "-".to_string(),
    };
    let perm = |label: &str, state: PermState| format!("{label}:{}", if state == PermState::Allowed { "allow" } else { "deny" });
    let tools = if s.running_tools.is_empty() { String::new() } else { format!(" | tools: {}", s.running_tools.join(" ")) };

    let text = format!(
        "{}  {turn_step}  {} {} {}{tools}  |  Ctrl+C: cancel/quit",
        s.model_name,
        perm("read", s.read),
        perm("shell", s.shell),
        perm("edit", s.edit),
    );
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, Style::default().fg(DIM)))), area);
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).border_style(if app.pending_approval.is_some() || app.pending_prompt.is_some() {
        Style::default().fg(DIM)
    } else {
        Style::default().fg(ACCENT)
    });
    let paragraph = Paragraph::new(app.input.as_str()).block(block).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use mjolnir_config::Config;
    use mjolnir_permissions::Engine;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::Arc;

    fn app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
        App::new("claude-sonnet-5".into(), Arc::new(Engine::new(config)))
    }

    fn rendered(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("")
    }

    /// Regression test for the bug the user actually hit: scroll math
    /// compared `viewport_height` (rendered rows) against `app.log.len()`
    /// (entry count) instead of `app.total_lines()` (rendered rows), so
    /// `max_offset` stayed 0 for any conversation with fewer entries than
    /// the viewport had rows — which is most of them, since a handful of
    /// multi-line entries routinely outgrows a terminal's row count. The
    /// old `ScrollState`-only unit tests couldn't catch this: they fed
    /// `total_len` in whatever unit the test author chose, never
    /// exercising the actual `App`/`ui::draw` wiring that picks that unit.
    /// This one does, with a small viewport that can't possibly show all
    /// ten 5-line entries at once.
    #[test]
    fn auto_follow_shows_the_tail_of_a_long_conversation_in_a_small_viewport() {
        let mut app = app();
        for i in 0..10 {
            app.log.push(LogEntry::AssistantText { text: format!("entry-{i}\nline2\nline3\nline4\nline5") });
        }

        // Log area gets roughly height-2 rows (status bar + input box eat
        // the rest) — nowhere near the ~59 rows ten 5-line entries plus
        // nine separators need.
        let out = rendered(&mut app, 80, 12);

        assert!(out.contains("entry-9"), "the latest entry must be visible under auto-follow");
        assert!(!out.contains("entry-0"), "the earliest entry must have scrolled out of view");
    }

    #[test]
    fn status_bar_shows_model_name_and_permission_summary() {
        let mut app = app();
        let out = rendered(&mut app, 80, 20);
        assert!(out.contains("claude-sonnet-5"));
        assert!(out.contains("read:deny"));
        assert!(out.contains("shell:deny"));
        assert!(out.contains("edit:deny"));
    }

    #[test]
    fn user_message_appears_in_the_rendered_log() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hello world".into() });
        let out = rendered(&mut app, 80, 20);
        assert!(out.contains("hello world"));
    }

    #[test]
    fn thinking_indicator_renders_only_while_active() {
        let mut app = app();
        app.thinking = true;
        assert!(rendered(&mut app, 80, 20).contains("thinking…"));
        app.thinking = false;
        assert!(!rendered(&mut app, 80, 20).contains("thinking…"));
    }

    #[test]
    fn approval_card_shows_labeled_keys_and_the_diff() {
        let mut app = app();
        app.log.push(LogEntry::ApprovalCard { call_id: "c1".into(), diff: "-old\n+new".into(), resolution: None });
        let out = rendered(&mut app, 80, 20);
        assert!(out.contains("approve"));
        assert!(out.contains("deny"));
        assert!(out.contains("old"));
        assert!(out.contains("new"));
    }

    #[test]
    fn user_and_assistant_messages_are_visually_distinct() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        app.log.push(LogEntry::AssistantText { text: "hi".into() });

        // Tall enough that the banner plus both entries fit without
        // triggering auto-follow scroll — otherwise the offset below
        // (which assumes the viewport shows everything from row 0) would
        // be reading the wrong rows entirely.
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // The welcome banner (log::INTRO_LINE_COUNT rows) plus its own
        // separator come first, then the same "row 1 is the blank
        // separator between entries, row 2 is the second entry" shape as
        // before, just offset past the banner.
        let base = intro_offset();
        let user_cell = &buffer[(2, base)]; // "> hi"
        let assistant_cell = &buffer[(2, base + 2)]; // "● hi"
        assert_ne!(
            (user_cell.fg, user_cell.modifier),
            (assistant_cell.fg, assistant_cell.modifier),
            "user and assistant text must use different styles"
        );
    }

    #[test]
    fn a_slash_command_renders_differently_from_a_plain_user_message() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        app.log.push(LogEntry::UserMessage { text: "/exit".into() });

        // See the sizing comment on user_and_assistant_messages_are_visually_distinct above.
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let base = intro_offset();
        let plain_cell = &buffer[(2, base)]; // "> hi"
        let command_cell = &buffer[(2, base + 2)]; // "> /exit" — the next row is the blank separator line
        assert_ne!(
            (plain_cell.fg, plain_cell.modifier),
            (command_cell.fg, command_cell.modifier),
            "a slash command must not use the same style as a plain user message"
        );
    }

    #[test]
    fn input_text_is_rendered_in_the_input_box() {
        let mut app = app();
        app.input = "draft text".into();
        let out = rendered(&mut app, 80, 20);
        assert!(out.contains("draft text"));
    }

    /// Rows the welcome banner always occupies before the first real log
    /// entry: `log::INTRO_LINE_COUNT` art/text rows plus the one separator
    /// `draw_log` inserts between the banner and the log (present here
    /// since every caller pushes at least one entry before measuring).
    fn intro_offset() -> u16 {
        (crate::log::INTRO_LINE_COUNT + 1) as u16
    }

    /// The row-index assumptions the two style-comparison tests above make
    /// (the row right after the banner is blank, the second entry lands two
    /// rows after that) only hold because `draw_log` inserts exactly one
    /// blank line between entries — pin that down directly so a change to
    /// the spacing logic fails loudly here instead of silently making those
    /// tests compare the wrong cells.
    #[test]
    fn a_blank_line_separates_consecutive_log_entries() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "first".into() });
        app.log.push(LogEntry::UserMessage { text: "second".into() });

        // See the sizing comment on user_and_assistant_messages_are_visually_distinct above.
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let row = intro_offset() + 1;
        let row_text: String = (0..80).map(|x| buffer[(x, row)].symbol().to_string()).collect();
        assert_eq!(row_text.trim(), "", "the row after the first entry must be the blank separator between the two entries");
    }

    #[test]
    fn assistant_text_gets_a_marker_that_user_text_does_not() {
        let mut assistant_app = app();
        assistant_app.log.push(LogEntry::AssistantText { text: "hi".into() });
        assert!(rendered(&mut assistant_app, 80, 20).contains('●'), "assistant text should start with a marker");

        let mut user_app = app();
        user_app.log.push(LogEntry::UserMessage { text: "hi".into() });
        assert!(!rendered(&mut user_app, 80, 20).contains('●'), "user text should not get the assistant marker");
    }

    #[test]
    fn fenced_code_block_is_stripped_of_its_fences_and_syntax_highlighted() {
        let mut app = app();
        app.log.push(LogEntry::AssistantText { text: "here:\n```rust\nfn main() {}\n```\ndone".into() });

        // See the sizing comment on user_and_assistant_messages_are_visually_distinct above.
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let out: String = buffer.content.iter().map(|c| c.symbol()).collect::<Vec<_>>().join("");

        assert!(!out.contains("```"), "the literal fence markers must not reach the screen");
        assert!(out.contains("rust"), "the language tag should appear in the block's header");
        assert!(out.contains("fn main"), "the code itself must still be shown");

        // At least two distinct foreground colors within the code line —
        // proof it went through the highlighter, not just plain dim text.
        // No blank-line separator here beyond the banner's own: draw_log
        // only inserts one between entries, and this is all one
        // AssistantText entry. Restricted to the line's own width so
        // unstyled padding cells past the printed text can't manufacture a
        // spurious second color.
        let code_row = intro_offset() + 2; // "● here:" / "┌─ rust" / "│ fn main() {}"
        let colors: std::collections::HashSet<Color> = (0..20).map(|x| buffer[(x, code_row)].fg).collect();
        assert!(colors.len() > 1, "expected the highlighted code line to use more than one color, got {colors:?}");
    }

    #[test]
    fn every_mjolnir_art_row_is_exactly_mjolnir_art_width_chars() {
        for (i, row) in MJOLNIR_ART.iter().enumerate() {
            assert_eq!(row.chars().count(), MJOLNIR_ART_WIDTH, "row {i} isn't fixed-width — the info column beside the art would drift off-alignment");
        }
    }

    #[test]
    fn the_wordmark_renders_above_the_hammer_art_with_info_beside_the_hammer() {
        let mut app = app();
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing its top rows out of view.
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // WORDMARK_ART starts right after the top border (screen row 0).
        let wordmark_row = 1u16;
        let wordmark_row_text: String = (0..100).map(|x| buffer[(x, wordmark_row)].symbol().to_string()).collect();
        assert!(wordmark_row_text.contains(WORDMARK_ART[0].trim()), "expected the wordmark's first row right after the top border, got: {wordmark_row_text:?}");

        // The tagline sits beside the hammer art (vertically centered
        // against MJOLNIR_ART's 16 rows, 3-line info block, offset
        // (16-3)/2 = 6), well below the wordmark block + its separator.
        let hammer_start = 1 + WORDMARK_ART.len() as u16 + 1;
        let tagline_row = hammer_start + ((MJOLNIR_ART.len() - 3) / 2) as u16;
        let tagline_row_text: String = (0..100).map(|x| buffer[(x, tagline_row)].symbol().to_string()).collect();
        assert!(tagline_row_text.contains("a tool for thought."), "expected the tagline beside the hammer art, got: {tagline_row_text:?}");
    }

    #[test]
    fn intro_banner_shows_the_active_model_and_is_exactly_intro_line_count_rows() {
        assert_eq!(intro_lines("claude-sonnet-5", 80).len(), crate::log::INTRO_LINE_COUNT, "ui::intro_lines must stay in sync with log::INTRO_LINE_COUNT");
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing its top rows out of view — see the sizing comment on
        // user_and_assistant_messages_are_visually_distinct.
        let out = rendered(&mut app(), 80, 40);
        assert!(out.contains("claude-sonnet-5"), "the active model should appear in the welcome banner");
        assert!(out.contains(WORDMARK_ART[3].trim()), "the wordmark should appear in the welcome banner");
        assert!(out.contains(env!("MJOLNIR_GIT_HASH")), "the build's git commit should appear in the welcome banner, distinct from the static crate version");
        assert!(out.contains(MJOLNIR_ART[0]), "the traced Mjolnir art should appear in the welcome banner");
    }

    #[test]
    fn a_fresh_session_shows_the_banner_before_any_log_entries() {
        let mut app = app();
        assert!(app.log.is_empty());
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing the wordmark (near the top) out of view.
        let out = rendered(&mut app, 80, 40);
        assert!(out.contains(WORDMARK_ART[3].trim()));
    }

    #[test]
    fn plain_user_messages_get_a_muted_background_but_slash_commands_do_not() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        app.log.push(LogEntry::UserMessage { text: "/exit".into() });

        // See the sizing comment on user_and_assistant_messages_are_visually_distinct above.
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let base = intro_offset();
        let plain_cell = &buffer[(2, base)]; // "> hi"
        let command_cell = &buffer[(2, base + 2)]; // "> /exit"
        assert_eq!(plain_cell.bg, USER_BG, "a plain user message should carry the subtle background tint");
        assert_ne!(command_cell.bg, USER_BG, "a slash command must not carry the chat-message background tint");
    }

    #[test]
    fn a_short_user_message_gets_the_background_tint_all_the_way_to_the_right_edge() {
        let mut app = app();
        app.log.push(LogEntry::UserMessage { text: "hi".into() });
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let row = intro_offset();
        let far_right_cell = &buffer[(79, row)]; // well past "> hi"
        assert_eq!(far_right_cell.bg, USER_BG, "the background tint should fill the full row width, not just trail the text");
    }

    #[test]
    fn the_welcome_banner_is_framed_by_a_border_spanning_the_full_render_width() {
        let mut app = app();
        // Tall enough that the whole banner fits without auto-follow scroll
        // pushing its top rows out of view — see the sizing comment on
        // user_and_assistant_messages_are_visually_distinct.
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        assert_eq!(buffer[(0, 0)].symbol(), "┌", "top-left corner of the banner's border");
        assert_eq!(buffer[(79, 0)].symbol(), "┐", "top-right corner should reach the full render width");
        let bottom = crate::log::INTRO_LINE_COUNT as u16 - 1;
        assert_eq!(buffer[(0, bottom)].symbol(), "└", "bottom-left corner of the banner's border");
        assert_eq!(buffer[(79, bottom)].symbol(), "┘", "bottom-right corner should reach the full render width");
    }

    #[test]
    fn bold_markdown_strips_asterisks_and_sets_the_bold_modifier() {
        let spans = parse_inline("say **hello** now", Style::default().fg(BRIGHT));
        let bold = spans.iter().find(|s| s.content.as_ref() == "hello").expect("bold span present");
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        assert!(spans.iter().all(|s| !s.content.contains('*')), "literal asterisks must not reach the screen");
    }

    #[test]
    fn italic_markdown_sets_the_italic_modifier() {
        let spans = parse_inline("that is *neat* stuff", Style::default().fg(BRIGHT));
        let italic = spans.iter().find(|s| s.content.as_ref() == "neat").expect("italic span present");
        assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn inline_code_strips_backticks_and_uses_reversed_video() {
        let spans = parse_inline("run `cargo test` first", Style::default().fg(BRIGHT));
        let code = spans.iter().find(|s| s.content.as_ref() == "cargo test").expect("code span present");
        assert!(code.style.add_modifier.contains(Modifier::REVERSED));
        assert!(spans.iter().all(|s| !s.content.contains('`')), "literal backticks must not reach the screen");
    }

    #[test]
    fn a_heading_line_drops_the_hashes_and_renders_bold() {
        let line = render_markdown_line("## Section Title");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Section Title");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_bullet_line_replaces_the_dash_with_a_bullet_marker() {
        let line = render_markdown_line("- first item");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "• first item");
    }

    #[test]
    fn markdown_in_the_full_log_renders_without_literal_markup_characters() {
        let mut app = app();
        app.log.push(LogEntry::AssistantText { text: "**bold** and `code` and *italic*".into() });
        let out = rendered(&mut app, 80, 20);
        assert!(!out.contains('*'), "literal asterisks must not reach the screen: {out:?}");
        assert!(!out.contains('`'), "literal backticks must not reach the screen: {out:?}");
        assert!(out.contains("bold"));
        assert!(out.contains("code"));
        assert!(out.contains("italic"));
    }
}
