use amundsen_permissions::PromptPayload;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, PermState};
use crate::log::{LogEntry, ToolActivityStatus};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const BRIGHT: Color = Color::White;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let input_height = input_area_height(&app.input);
    let [log_area, status_area, input_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1), Constraint::Length(input_height)]).areas(area);

    let log_inner_height = log_area.height as usize;
    app.scroll.set_viewport_height(log_inner_height, app.log.len());

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
    let mut lines: Vec<Line> = Vec::new();
    for entry in &app.log {
        lines.extend(render_entry(entry));
    }
    if app.thinking {
        lines.push(Line::from(Span::styled("thinking…", Style::default().fg(DIM))));
    }

    let visible: Vec<Line> = lines.into_iter().skip(app.scroll.offset).collect();
    let paragraph = Paragraph::new(Text::from(visible)).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_entry(entry: &LogEntry) -> Vec<Line<'static>> {
    match entry {
        // Palette per amundsen-tui.md: bright = assistant, normal = user —
        // these must not share a style, or the two speakers become
        // indistinguishable in the log. A slash command is user input that
        // never reaches the model (see amundsen-cli's interceptor) — dim
        // marks it as directed at the harness itself, not conversation,
        // the same way tool metadata and notices are dim.
        LogEntry::UserMessage { text } => {
            let style = if is_command(text) { Style::default().fg(DIM) } else { Style::default() };
            text.lines().map(|l| Line::from(Span::styled(format!("> {l}"), style))).collect()
        }
        LogEntry::AssistantText { text } => {
            text.lines().map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(BRIGHT).add_modifier(Modifier::BOLD)))).collect()
        }
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

/// Mirrors amundsen-cli's own `/`-prefix check (`text.trim_start().strip_prefix('/')`
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
    use amundsen_config::Config;
    use amundsen_permissions::Engine;
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

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Both lines render "hi" at column 2 (after the "> " / no prefix);
        // find each by its line content and compare the cell style at the
        // first character — they must not be identical, or the two
        // speakers are indistinguishable in the log.
        let user_cell = &buffer[(2, 0)]; // "> hi"
        let assistant_cell = &buffer[(0, 1)]; // "hi"
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

        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer().clone();

        let plain_cell = &buffer[(2, 0)]; // "> hi"
        let command_cell = &buffer[(2, 1)]; // "> /exit"
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
}
