use std::collections::HashMap;

use aldwin_core::{Answer, Command, Event, LogRecord, PlanStep, Question, ReviewDecision, ReviewOutcome, TurnEndReason};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};

use crate::list::{List, ListOutcome, ListRow};
use crate::log::{LogEntry, WorkItem};
use crate::resume::SessionChoice;
use crate::review::{Review, ReviewOutcome as ReviewKey};
use crate::scroll::ScrollState;

/// How long a second Ctrl+C still counts as "again" for the exit escape
/// hatch in `App::cancel_or_quit`, in `App::tick`s — `run.rs` advances that
/// counter every 120ms, so ~2 seconds.
const DOUBLE_CTRL_C_TICKS: u64 = 16;

/// Rows one wheel notch moves the transcript — the same three a terminal's
/// own alternate-scroll translation sends as cursor keys.
const WHEEL_ROWS: usize = 3;

/// The design's option row for a question the developer would rather
/// answer in words. Appended by the `ask` tool; matched here by text.
const CHAT_ABOUT_THIS: &str = "Chat about this";

/// The four commands the `/` menu offers, in the order drawn — the
/// developer's list (see `crates/review/baseline.json`,
/// `frame-command-list-is-not-the-products`). Name, purpose, and what
/// picking it submits.
pub const COMMANDS: [(&str, &str, &str); 4] = [
    ("resume", "Pick up an earlier conversation", "/resume"),
    ("model", "Change the model", "/model"),
    ("quit", "Leave Aldwin", "/exit"),
    ("clear", "Start a fresh conversation in this project", "/clear"),
];

/// The display halves of one provider the catalogue offers, handed in by
/// aldwin-cli — an id, a purpose, and its models. This crate never sees an
/// endpoint or a key variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderChoice {
    pub id:      String,
    pub purpose: String,
    pub models:  Vec<ModelChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub id:      String,
    pub purpose: String,
    /// What the context bar divides by once this model is running.
    pub context: u32,
}

/// Who asked the question on screen, which decides where the answer goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asker {
    /// The agent, through `ask`. The answer is a `Command::Answer`.
    Agent { call_id: String },
    /// This screen, because no model is configured: which provider. The
    /// answer opens the model question.
    Provider { then: Option<String> },
    /// Which of that provider's models. The answer submits `/model p/m`,
    /// then the message that was waiting, if any.
    Model { provider: String, then: Option<String> },
    /// Bare `/resume`: which session. The answer submits `/resume <id>`.
    Session,
}

/// A question on screen: the design's `QuestionPanel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asking {
    pub question: Question,
    pub list:     List,
    pub asker:    Asker,
}

/// The `/` menu: the four commands, filtered by what is typed after the slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMenu {
    pub filter: String,
    pub list:   List,
}

impl CommandMenu {
    fn open() -> Self {
        let mut menu = Self { filter: String::new(), list: List::new(Vec::new()) };
        menu.refilter();
        menu
    }

    /// The commands whose name starts with the filter, in the fixed order.
    pub fn matching(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        COMMANDS.iter().copied().filter(|(name, _, _)| name.starts_with(self.filter.as_str())).collect()
    }

    fn refilter(&mut self) {
        let rows = self.matching().into_iter().map(|(name, purpose, _)| ListRow::with_detail(format!("/{name}"), purpose)).collect();
        let selected = self.list.selected;
        self.list = List::new(rows).opened_on(selected);
    }
}

/// What holds the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The conversation: transcript, field, footer.
    Conversation,
    /// A question in the bottom band, in place of the field.
    Question(Asking),
    /// The `/` menu above the field.
    Commands(CommandMenu),
    /// The full-window review.
    Review(Review),
}

/// Session facts the launch card and the footer draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInfo {
    /// The bare model id, or empty when nothing is configured yet.
    pub model_name:     String,
    pub version:        String,
    pub commit:         String,
    /// The project — the working directory's own name.
    pub project:        String,
    /// The git branch, when the directory is a checkout.
    pub branch:         Option<String>,
    /// The model's context window in tokens, when the catalogue knows it.
    pub context_window: Option<u32>,
    /// Tokens the last request carried — the prompt at the last step.
    pub context_used:   Option<u32>,
}

impl StatusInfo {
    /// The context bar's percentage, when both halves are known.
    pub fn context_percent(&self) -> Option<u8> {
        let (used, window) = (self.context_used?, self.context_window?);
        if window == 0 {
            return None;
        }
        Some(((u64::from(used) * 100 / u64::from(window)).min(100)) as u8)
    }
}

/// The working directory's own name — the launch card's `Project` fact.
pub(crate) fn project_name() -> String {
    std::env::current_dir().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default()
}

/// The checked-out branch, read from `.git/HEAD` rather than by running
/// git — this crate spawns nothing.
pub(crate) fn git_branch() -> Option<String> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let head = dir.join(".git").join("HEAD");
        if let Ok(text) = std::fs::read_to_string(&head) {
            let text = text.trim();
            return Some(text.strip_prefix("ref: refs/heads/").map_or_else(|| text.chars().take(8).collect(), str::to_string));
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Application state and the pure logic that mutates it. Rendering (`ui/`)
/// only ever reads from this; the terminal/event-loop glue (`run.rs`) only
/// ever calls `apply_event`/`handle_key` and does no interpretation of its
/// own — kept this way so both are unit-testable without a terminal.
pub struct App {
    pub log:            Vec<LogEntry>,
    pub mode:           Mode,
    pub thinking:       bool,
    pub scroll:         ScrollState,
    /// The log area's real render width, last set by `ui::draw`. Scroll
    /// navigation happens between draws with no render access of its own,
    /// so it reads this cached value.
    pub render_width:   u16,
    pub input:          String,
    pub cursor:         usize, // char index into `input`
    /// The field's real text-column width, cached by the field's own draw
    /// exactly as `render_width` is.
    pub composer_width: u16,
    /// First visual row of the draft the field is showing.
    pub composer_top:   usize,
    pub status:         StatusInfo,
    pub should_quit:    bool,
    /// True from `TurnStarted` until the matching `TurnEnded`.
    pub turn_active:    bool,
    /// True from a submitted message until the turn it asks for either
    /// starts or is answered without one ever starting (a locally-handled
    /// slash command). Only `cancel_or_quit` reads it.
    pub awaiting_turn:  bool,
    /// The next submission answers this `ask` call in words — the developer
    /// chose "Chat about this".
    pub answering:      Option<String>,
    /// Whether the work disclosures of the current turn are open. Space
    /// toggles it (`Space  Hide Details`).
    pub details_open:   bool,
    last_cancel_tick:   Option<u64>,
    pub tick:           u64,
    /// Populated on `ToolUseRequested` (the one event that carries the
    /// tool's name and input), consumed on `ToolDispatched`.
    pending_calls:      HashMap<String, (String, serde_json::Value)>,
    /// Commands `handle_key`/`apply_event` want sent — drained by the event
    /// loop after each call.
    pub outbox:         Vec<Command>,
    pub catalogue:      Vec<ProviderChoice>,
    pub current_provider: Option<String>,
    pub sessions:       Vec<SessionChoice>,
    pub theme:          crate::palette::Theme,
    transcript:         crate::ui::Transcript,
}

impl App {
    pub fn new(model_name: String) -> Self {
        Self {
            log: Vec::new(),
            mode: Mode::Conversation,
            thinking: false,
            scroll: ScrollState::default(),
            render_width: 80,
            input: String::new(),
            cursor: 0,
            composer_width: 74,
            composer_top: 0,
            status: StatusInfo {
                model_name,
                version: crate::version::VERSION.to_string(),
                commit: crate::version::GIT_HASH.to_string(),
                project: project_name(),
                branch: git_branch(),
                context_window: None,
                context_used: None,
            },
            should_quit: false,
            turn_active: false,
            awaiting_turn: false,
            answering: None,
            details_open: false,
            last_cancel_tick: None,
            tick: 0,
            pending_calls: HashMap::new(),
            outbox: Vec::new(),
            catalogue: Vec::new(),
            current_provider: None,
            sessions: Vec::new(),
            theme: crate::palette::Theme::default(),
            transcript: crate::ui::Transcript::default(),
        }
    }

    pub fn with_sessions(mut self, sessions: Vec<SessionChoice>) -> Self {
        self.sessions = sessions;
        self
    }

    /// The catalogue and which row the session runs on. `context_window`
    /// follows from the two when the row and model are known.
    pub fn with_catalogue(mut self, catalogue: Vec<ProviderChoice>, current_provider: Option<String>) -> Self {
        self.catalogue = catalogue;
        self.current_provider = current_provider;
        self.status.context_window = self.context_window_for(self.current_provider.as_deref(), &self.status.model_name);
        self
    }

    pub fn with_theme(mut self, theme: crate::palette::Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Test and preview seams for facts a real session reads from the
    /// machine.
    pub fn with_facts(mut self, project: &str, branch: Option<&str>) -> Self {
        self.status.project = project.into();
        self.status.branch = branch.map(str::to_string);
        self
    }

    fn context_window_for(&self, provider: Option<&str>, model: &str) -> Option<u32> {
        let p = self.catalogue.iter().find(|p| Some(p.id.as_str()) == provider)?;
        p.models.iter().find(|m| m.id == model).map(|m| m.context)
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Whether anything on screen reads `tick` — the running dot, and the
    /// caret, which blinks whenever a field is drawn (`--caret-period`).
    /// ratatui diffs cells, so a tick that moves nothing costs a draw and
    /// no bytes.
    pub(crate) fn is_animating(&self) -> bool {
        self.thinking || self.turn_active || self.awaiting_turn || matches!(self.mode, Mode::Conversation | Mode::Review(_) | Mode::Commands(_))
    }

    pub fn review(&self) -> Option<&Review> {
        match &self.mode {
            Mode::Review(r) => Some(r),
            _ => None,
        }
    }

    pub(crate) fn review_mut(&mut self) -> Option<&mut Review> {
        match &mut self.mode {
            Mode::Review(r) => Some(r),
            _ => None,
        }
    }

    /// The open review, for the snapshot harness to seed a state the keys
    /// alone cannot reach in a fixed number of presses.
    #[doc(hidden)]
    pub fn review_for_tests(&mut self) -> Option<&mut Review> {
        self.review_mut()
    }

    /// True while something other than the field takes keys.
    fn band_is_held(&self) -> bool {
        !matches!(self.mode, Mode::Conversation)
    }

    fn reset_conversation(&mut self) {
        self.log.clear();
        self.scroll = ScrollState::default();
        self.thinking = false;
        self.turn_active = false;
        self.awaiting_turn = false;
        self.details_open = false;
    }

    fn push(&mut self, entry: LogEntry) {
        self.log.push(entry);
        let total = self.total_lines();
        self.scroll.on_content_grew(total);
    }

    pub fn total_lines(&mut self) -> usize {
        self.sync_transcript();
        self.transcript.len()
    }

    #[cfg(test)]
    pub(crate) fn blocks_rebuilt(&self) -> usize {
        self.transcript.rebuilt()
    }

    pub(crate) fn transcript_view(&mut self, height: usize) -> Vec<ratatui::text::Line<'static>> {
        self.sync_transcript();
        let total = self.transcript.len();
        self.scroll.set_viewport_height(height, total);
        self.transcript.viewport(self.scroll.offset, height)
    }

    fn sync_transcript(&mut self) {
        let mut transcript = std::mem::take(&mut self.transcript);
        transcript.sync(self, self.render_width);
        self.transcript = transcript;
    }

    /// The `Work` entry of the current step — the last entry, if it is one.
    fn open_work(&mut self) -> Option<&mut Vec<WorkItem>> {
        match self.log.last_mut() {
            Some(LogEntry::Work { items, .. }) => Some(items),
            _ => None,
        }
    }

    fn record_call(&mut self, call_id: String, name: String, input: serde_json::Value) {
        let (verb, target) = WorkItem::describe(&name, &input);
        let item = WorkItem { call_id, verb, target, fact: None, failed: false };
        // `plan` and `ask` are not work the disclosure lists: the plan is
        // drawn as itself, and a question is its own row.
        if name == "plan" || name == "ask" {
            return;
        }
        let open = self.details_open;
        match self.open_work() {
            Some(items) => items.push(item),
            None => self.push(LogEntry::Work { items: vec![item], open }),
        }
    }

    fn finish_call(&mut self, call_id: &str, content: &str, is_error: bool) {
        // A call the disclosure lists: the newest `Work` entry that holds it.
        for entry in self.log.iter_mut().rev() {
            if let LogEntry::Work { items, .. } = entry {
                if let Some(item) = items.iter_mut().find(|i| i.call_id == call_id) {
                    item.failed = is_error;
                    item.fact = Some(WorkItem::fact_for(&item.verb, content, is_error));
                    return;
                }
            }
        }
        // Otherwise an `ask` answered: the newest unanswered question row
        // gets its answer. (`plan` results land nowhere; the plan is drawn
        // as itself.)
        if let Some(LogEntry::Question { answer, .. }) = self.log.iter_mut().rev().find(|e| matches!(e, LogEntry::Question { answer: None, .. })) {
            *answer = Some(content.trim_start_matches("The developer chose: ").trim_start_matches("The developer said: ").to_string());
        }
    }

    /// One loaded record, as the entry the live path would have produced.
    fn replay(&mut self, record: LogRecord) {
        match record {
            LogRecord::UserMessage { text, .. } => self.log.push(LogEntry::UserMessage { text }),
            LogRecord::AssistantMessage { text, .. } => {
                if let Some(LogEntry::AssistantText { text: buf }) = self.log.last_mut() {
                    buf.push('\n');
                    buf.push_str(&text);
                } else {
                    self.log.push(LogEntry::AssistantText { text });
                }
            }
            LogRecord::ToolUse { call, .. } => self.record_call(call.id, call.name, call.input),
            LogRecord::ToolResult { result, .. } => self.finish_call(&result.call_id, &result.content, result.is_error),
            LogRecord::TurnEnded { reason, .. } => self.push_turn_end(reason),
            LogRecord::Thinking { .. } | LogRecord::RedactedThinking { .. } => {}
            LogRecord::TurnStarted { .. } | LogRecord::StepBoundary { .. } => {}
        }
    }

    fn push_turn_end(&mut self, reason: TurnEndReason) {
        match reason {
            TurnEndReason::EndTurn => self.push(LogEntry::TurnBreak),
            TurnEndReason::Cancelled => {
                self.push(LogEntry::Failure { message: "Stopped.".into(), detail: None, open: false });
                self.push(LogEntry::TurnBreak);
            }
            TurnEndReason::Error(message) => {
                // The sentence names the kind of failure; the disclosure
                // holds all of it. Every error the loop reports reads
                // `kind: particulars`, and the particulars are a provider's
                // own body — JSON, as often as not — which is a detail to
                // open, not a sentence to read (ADR 0009 §5).
                let first = crate::log::first_line(&message, 72);
                let sentence = match first.split_once(": ") {
                    Some((kind, _)) if !kind.is_empty() => format!("The turn did not finish: {kind}."),
                    _ => format!("The turn did not finish: {first}"),
                };
                self.push(LogEntry::Failure { message: sentence, detail: Some(message), open: false });
                self.push(LogEntry::TurnBreak);
            }
        }
    }

    pub fn apply_event(&mut self, event: Event) {
        match event {
            Event::TurnStarted { .. } => {
                self.turn_active = true;
                self.awaiting_turn = false;
                self.details_open = false;
            }
            Event::TextDelta { text, .. } => {
                if let Some(LogEntry::AssistantText { text: buf }) = self.log.last_mut() {
                    buf.push_str(&text);
                } else {
                    self.push(LogEntry::AssistantText { text });
                }
            }
            Event::ThinkingStart { .. } | Event::ThinkingDelta { .. } => self.thinking = true,
            Event::ThinkingEnd { .. } => self.thinking = false,
            Event::ToolUseRequested { call, .. } => {
                self.pending_calls.insert(call.id, (call.name, call.input));
            }
            Event::ToolDispatched { call_id, .. } => {
                if let Some((name, input)) = self.pending_calls.remove(&call_id) {
                    self.record_call(call_id, name, input);
                }
            }
            Event::ToolCompleted { result, .. } => {
                // A call refused before dispatch never got `ToolDispatched`.
                if let Some((name, input)) = self.pending_calls.remove(&result.call_id) {
                    self.record_call(result.call_id.clone(), name, input);
                }
                self.finish_call(&result.call_id, &result.content, result.is_error);
            }
            Event::StepEnded { outcome, .. } => {
                self.status.context_used =
                    Some(outcome.usage.input_tokens + outcome.cache.cache_read_input_tokens + outcome.cache.cache_creation_input_tokens);
            }
            Event::RetryAttempt { info, .. } => self.push(LogEntry::retry(&info)),
            Event::TurnEnded { reason, .. } => {
                self.pending_calls.clear();
                self.turn_active = false;
                self.awaiting_turn = false;
                self.thinking = false;
                self.push_turn_end(reason);
            }
            // The developer's review comments, echoed the way a typed
            // message is — this is the one message the TUI did not send.
            Event::FollowUp { text, .. } => {
                self.awaiting_turn = true;
                self.push(LogEntry::UserMessage { text });
            }
            Event::PlanUpdated { steps, .. } => self.set_plan(steps),
            Event::QuestionAsked { call_id, question } => {
                self.push(LogEntry::Question { question: question.question.clone(), answer: None });
                let rows = question.options.iter().map(|o| ListRow::new(o.clone())).collect();
                self.mode = Mode::Question(Asking { question, list: List::new(rows), asker: Asker::Agent { call_id } });
            }
            Event::ReviewRequested { review_id, changeset } => {
                // The dispatcher never opens a review over nothing, and a
                // review over nothing has nothing to draw: answer it rather
                // than open it.
                if changeset.files.is_empty() {
                    self.outbox.push(Command::ReviewDecision { review_id, decision: ReviewDecision::Discard });
                    return;
                }
                self.push(LogEntry::AssistantText { text: format!("Ready for you to review: {}.", review_title(&changeset)) });
                self.mode = Mode::Review(Review::open(review_id, changeset));
            }
            Event::ReviewClosed { outcome } => {
                if matches!(self.mode, Mode::Review(_)) {
                    self.mode = Mode::Conversation;
                }
                // A comment goes back as the next message; the row the log
                // keeps is the saved or discarded one.
                if !matches!(outcome, ReviewOutcome::Commented { .. }) {
                    self.push(LogEntry::Review { outcome });
                }
            }
            Event::Notice { message } => {
                self.awaiting_turn = false;
                self.push(LogEntry::Notice { message });
            }
            Event::HistoryCleared => self.reset_conversation(),
            Event::HistoryLoaded { records } => {
                self.reset_conversation();
                for record in records {
                    self.replay(record);
                }
                self.sync_transcript();
            }
            Event::ThemeChanged { theme } => {
                self.awaiting_turn = false;
                self.theme = crate::palette::Theme::from_config(Some(&theme));
            }
            Event::ModelChanged { provider, model, context_window } => {
                self.awaiting_turn = false;
                self.status.context_window = context_window.or_else(|| self.context_window_for(provider.as_deref(), &model));
                self.status.model_name = model;
                self.current_provider = provider;
            }
        }
    }

    /// The plan is one entry per turn, replaced in place.
    fn set_plan(&mut self, steps: Vec<PlanStep>) {
        let since_turn = self.log.iter().rposition(|e| matches!(e, LogEntry::UserMessage { .. })).unwrap_or(0);
        if let Some(entry) = self.log[since_turn..].iter_mut().find(|e| matches!(e, LogEntry::Plan { .. })) {
            *entry = LogEntry::Plan { steps };
        } else {
            self.push(LogEntry::Plan { steps });
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match &mut self.mode {
            Mode::Review(_) => self.handle_review_key(key),
            Mode::Question(_) => self.handle_question_key(key),
            Mode::Commands(_) => self.handle_commands_key(key),
            Mode::Conversation => self.handle_conversation_key(key),
        }
    }

    fn handle_conversation_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, m) if m.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => self.insert_char('\n'),
            (KeyCode::Enter, _) => self.submit(),
            (KeyCode::Char('j'), m) if m.contains(KeyModifiers::CONTROL) => self.insert_char('\n'),
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => self.cancel_or_quit(),
            // `⎋  Stop` while working; nothing otherwise.
            (KeyCode::Esc, _) if self.turn_active || self.awaiting_turn => self.cancel_or_quit(),
            // `/` into an empty field opens the menu; anywhere else it types.
            (KeyCode::Char('/'), _) if self.input.is_empty() => self.mode = Mode::Commands(CommandMenu::open()),
            // `Space  Hide Details` — on an empty field only; otherwise it
            // is a space.
            (KeyCode::Char(' '), _) if self.input.is_empty() && self.has_work() => self.toggle_details(),
            (KeyCode::Backspace, _) => self.backspace(),
            (KeyCode::Delete, _) => self.delete_forward(),
            (KeyCode::Left, _) => self.cursor = self.cursor.saturating_sub(1),
            (KeyCode::Right, _) => self.cursor = (self.cursor + 1).min(self.input.chars().count()),
            (KeyCode::Home, _) => self.cursor = crate::draft::source_line(&self.input, self.cursor).0,
            (KeyCode::End, _) if self.input.is_empty() => {
                let total = self.total_lines();
                self.scroll.jump_to_bottom(total);
            }
            (KeyCode::End, _) => self.cursor = crate::draft::source_line(&self.input, self.cursor).1,
            (KeyCode::PageUp, _) => self.scroll.page_up(),
            (KeyCode::PageDown, _) => {
                let total = self.total_lines();
                self.scroll.page_down(total);
            }
            (KeyCode::Up, _) => {
                if !self.move_cursor_vertical(-1) {
                    self.scroll.line_up();
                }
            }
            (KeyCode::Down, _) => {
                if !self.move_cursor_vertical(1) {
                    let total = self.total_lines();
                    self.scroll.line_down(total);
                }
            }
            (KeyCode::Char(_), m) if m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) => {}
            (KeyCode::Char(c), _) => self.insert_char(c),
            _ => {}
        }
    }

    /// Whether the current turn has any work to show or hide.
    fn has_work(&self) -> bool {
        let since = self.log.iter().rposition(|e| matches!(e, LogEntry::UserMessage { .. })).unwrap_or(0);
        self.log[since..].iter().any(|e| matches!(e, LogEntry::Work { .. } | LogEntry::Failure { detail: Some(_), .. }))
    }

    fn toggle_details(&mut self) {
        self.details_open = !self.details_open;
        let open = self.details_open;
        let since = self.log.iter().rposition(|e| matches!(e, LogEntry::UserMessage { .. })).unwrap_or(0);
        for entry in &mut self.log[since..] {
            match entry {
                LogEntry::Work { open: o, .. } => *o = open,
                LogEntry::Failure { open: o, detail: Some(_), .. } => *o = open,
                _ => {}
            }
        }
    }

    fn handle_commands_key(&mut self, key: KeyEvent) {
        let Mode::Commands(menu) = &mut self.mode else { return };
        match key.code {
            // Letters filter by name; a digit picks the row, as in every
            // other list.
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && c.is_ascii_alphabetic() => {
                menu.filter.push(c.to_ascii_lowercase());
                menu.refilter();
                return;
            }
            KeyCode::Backspace => {
                if menu.filter.pop().is_none() {
                    self.mode = Mode::Conversation;
                    return;
                }
                menu.refilter();
                return;
            }
            _ => {}
        }
        match menu.list.handle_key(key.code, key.modifiers) {
            ListOutcome::Stay => {}
            ListOutcome::Close => self.mode = Mode::Conversation,
            ListOutcome::Chose(i) => {
                let Some((name, _, submits)) = menu.matching().get(i).copied() else { return };
                self.mode = Mode::Conversation;
                match name {
                    "resume" if !self.sessions.is_empty() => self.open_session_question(),
                    "model" if !self.catalogue.is_empty() => self.open_provider_question(None),
                    _ => self.submit_text(submits.to_string()),
                }
            }
        }
    }

    fn handle_question_key(&mut self, key: KeyEvent) {
        let Mode::Question(asking) = &mut self.mode else { return };
        let outcome = asking.list.handle_key(key.code, key.modifiers);
        let asking = asking.clone();
        match outcome {
            ListOutcome::Stay => {}
            // A question from the agent cannot be dismissed — the tool is
            // waiting — so `⎋` is "Chat about this".
            ListOutcome::Close => match asking.asker {
                Asker::Agent { call_id } => self.chat_about(call_id),
                _ => self.mode = Mode::Conversation,
            },
            ListOutcome::Chose(i) => self.answer(asking, i),
        }
    }

    fn chat_about(&mut self, call_id: String) {
        self.answering = Some(call_id);
        self.mode = Mode::Conversation;
    }

    fn answer(&mut self, asking: Asking, index: usize) {
        let chosen = asking.question.options.get(index).cloned().unwrap_or_default();
        self.mode = Mode::Conversation;
        match asking.asker {
            Asker::Agent { call_id } => {
                if chosen == CHAT_ABOUT_THIS {
                    self.chat_about(call_id);
                } else {
                    self.outbox.push(Command::Answer { call_id, answer: Answer::Chose { index } });
                }
            }
            Asker::Provider { then } => {
                let Some(provider) = self.catalogue.get(index).cloned() else { return };
                self.open_model_question(provider, then);
            }
            Asker::Model { provider, then } => {
                let model = self.catalogue.iter().find(|p| p.id == provider).and_then(|p| p.models.get(index)).map(|m| m.id.clone());
                if let Some(model) = model {
                    self.submit_text(format!("/model {provider}/{model}"));
                }
                if let Some(text) = then {
                    self.submit_text(text);
                }
            }
            Asker::Session => {
                if let Some(session) = self.sessions.get(index) {
                    self.submit_text(format!("/resume {}", session.id));
                }
            }
        }
    }

    fn open_provider_question(&mut self, then: Option<String>) {
        let rows = self.catalogue.iter().map(|p| ListRow::with_detail(p.id.clone(), p.purpose.clone())).collect();
        let current = self.current_provider.as_deref().and_then(|c| self.catalogue.iter().position(|p| p.id == c)).unwrap_or(0);
        let question = Question {
            question: "Where should the model run?".into(),
            detail:   "Each provider needs its key in the environment variable named beside it.".into(),
            options:  self.catalogue.iter().map(|p| p.id.clone()).collect(),
        };
        self.mode = Mode::Question(Asking { question, list: List::new(rows).opened_on(current), asker: Asker::Provider { then } });
    }

    fn open_model_question(&mut self, provider: ProviderChoice, then: Option<String>) {
        let rows = provider.models.iter().map(|m| ListRow::with_detail(m.id.clone(), m.purpose.clone())).collect();
        let current = provider.models.iter().position(|m| m.id == self.status.model_name).unwrap_or(0);
        let question = Question {
            question: format!("Which {} model?", provider.id),
            detail:   "Any model id the provider offers works; these are the known ones.".into(),
            options:  provider.models.iter().map(|m| m.id.clone()).collect(),
        };
        self.mode = Mode::Question(Asking { question, list: List::new(rows).opened_on(current), asker: Asker::Model { provider: provider.id, then } });
    }

    fn open_session_question(&mut self) {
        let rows = self
            .sessions
            .iter()
            .map(|s| ListRow::with_detail(s.title.clone(), format!("{} · {} {}", s.when, s.turns, if s.turns == 1 { "turn" } else { "turns" })))
            .collect();
        let question = Question {
            question: "Which conversation?".into(),
            detail:   "Newest first. The one you pick continues in its own file.".into(),
            options:  self.sessions.iter().map(|s| s.title.clone()).collect(),
        };
        self.mode = Mode::Question(Asking { question, list: List::new(rows), asker: Asker::Session });
    }

    fn handle_review_key(&mut self, key: KeyEvent) {
        // Ctrl+C in a review cancels the turn (which cancels the review).
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.cancel_or_quit();
            return;
        }
        let general = self.input.clone();
        let Some(review) = self.review_mut() else { return };
        let typing = review.comment.is_none() && !review.confirm_discard;
        // Text keys go to the review's own field unless a comment draft has
        // them; everything else is the review's.
        let is_text = matches!(key.code, KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) && c != ' ' && c != '?')
            || (matches!(key.code, KeyCode::Char(' ') | KeyCode::Char('?')) && !general.is_empty());
        if typing && (is_text || (key.code == KeyCode::Backspace && !general.is_empty())) {
            match key.code {
                KeyCode::Backspace => self.backspace(),
                KeyCode::Char(c) => self.insert_char(c),
                _ => {}
            }
            return;
        }
        match review.handle_key(key.code, key.modifiers, &general) {
            ReviewKey::Stay => {}
            ReviewKey::Decide(decision) => {
                let review_id = review.review_id.clone();
                if matches!(decision, ReviewDecision::Comment { .. }) {
                    self.input.clear();
                    self.cursor = 0;
                }
                self.outbox.push(Command::ReviewDecision { review_id, decision });
            }
        }
    }

    pub fn handle_mouse(&mut self, event: MouseEvent) {
        if self.band_is_held() {
            return;
        }
        match event.kind {
            MouseEventKind::ScrollUp => {
                for _ in 0..WHEEL_ROWS {
                    self.scroll.line_up();
                }
            }
            MouseEventKind::ScrollDown => {
                let total = self.total_lines();
                for _ in 0..WHEEL_ROWS {
                    self.scroll.line_down(total);
                }
            }
            _ => {}
        }
    }

    pub fn paste(&mut self, text: &str) {
        if self.band_is_held() {
            return;
        }
        self.insert_str(&crate::draft::sanitize(text));
    }

    fn move_cursor_vertical(&mut self, delta: isize) -> bool {
        let layout = crate::draft::Layout::new(&self.input, self.composer_width as usize);
        match layout.step_row(self.cursor, delta) {
            Some(cursor) => {
                self.cursor = cursor;
                true
            }
            None => false,
        }
    }

    fn insert_char(&mut self, c: char) {
        let byte_idx = self.byte_at(self.cursor);
        self.input.insert(byte_idx, c);
        self.cursor += 1;
    }

    fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let byte_idx = self.byte_at(self.cursor);
        self.input.insert_str(byte_idx, text);
        self.cursor += text.chars().count();
    }

    fn byte_at(&self, index: usize) -> usize {
        self.input.char_indices().nth(index).map_or(self.input.len(), |(i, _)| i)
    }

    fn backspace(&mut self) {
        let cursor = self.cursor.min(self.input.chars().count());
        if cursor == 0 {
            return;
        }
        self.cursor = cursor - 1;
        let byte_idx = self.byte_at(self.cursor);
        self.input.remove(byte_idx);
    }

    fn delete_forward(&mut self) {
        if let Some((byte_idx, _)) = self.input.char_indices().nth(self.cursor) {
            self.input.remove(byte_idx);
        }
    }

    fn submit(&mut self) {
        if self.input.trim().is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.input);
        self.cursor = 0;
        // "Chat about this": what was typed answers the question.
        if let Some(call_id) = self.answering.take() {
            self.push(LogEntry::UserMessage { text: text.clone() });
            self.outbox.push(Command::Answer { call_id, answer: Answer::Said { text } });
            return;
        }
        // No model yet: the first message is held while the two questions
        // are answered, then sent.
        if self.status.model_name.is_empty() && !self.catalogue.is_empty() && !text.trim_start().starts_with('/') {
            self.open_provider_question(Some(text));
            return;
        }
        let trimmed = text.trim();
        if trimmed == "/resume" && !self.sessions.is_empty() {
            self.open_session_question();
            return;
        }
        if trimmed == "/model" && !self.catalogue.is_empty() {
            self.open_provider_question(None);
            return;
        }
        self.submit_text(text);
    }

    /// The tail every submission shares, typed or picked.
    fn submit_text(&mut self, text: String) {
        self.awaiting_turn = true;
        self.push(LogEntry::UserMessage { text: text.clone() });
        self.outbox.push(Command::Submit { text });
    }

    /// Ctrl+C: cancel the running turn if there is one, otherwise exit. A
    /// second Ctrl+C within `DOUBLE_CTRL_C_TICKS` always exits.
    fn cancel_or_quit(&mut self) {
        let busy = self.turn_active || self.awaiting_turn;
        let repeat = self.last_cancel_tick.is_some_and(|t| self.tick.saturating_sub(t) <= DOUBLE_CTRL_C_TICKS);
        if busy && !repeat {
            self.last_cancel_tick = Some(self.tick);
            self.outbox.push(Command::Cancel);
            self.push(LogEntry::Notice { message: "Stopping. Press ⌃C again to leave Aldwin.".into() });
        } else {
            self.should_quit = true;
        }
    }
}

/// `rate limiting for the gateway` — the review's title is the first line
/// of the conversation's last request, lowercased, or the files when
/// there is none.
pub(crate) fn review_title(changeset: &aldwin_core::Changeset) -> String {
    let n = changeset.files.len();
    format!("{n} {}", if n == 1 { "file" } else { "files" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_core::{CacheStats, ChangedFile, Changeset, StepId, StepOutcome, StopReason, ToolCall, ToolResult, TurnId, UsageStats};
    use ratatui::crossterm::event::KeyEventState;

    fn app() -> App {
        App::new("claude-sonnet-5".into())
    }

    fn press(code: KeyCode) -> KeyEvent {
        press_mod(code, KeyModifiers::NONE)
    }

    fn press_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle_key(press(KeyCode::Char(c)));
        }
    }

    fn catalogue() -> Vec<ProviderChoice> {
        vec![
            ProviderChoice {
                id:      "anthropic".into(),
                purpose: "claude models".into(),
                models:  vec![ModelChoice { id: "claude-sonnet-5".into(), purpose: "balanced".into(), context: 1_000_000 }],
            },
            ProviderChoice {
                id:      "openai".into(),
                purpose: "gpt models".into(),
                models:  vec![ModelChoice { id: "gpt-5".into(), purpose: "balanced".into(), context: 400_000 }],
            },
        ]
    }

    #[test]
    fn typing_and_enter_submits_and_echoes() {
        let mut a = app();
        type_str(&mut a, "hello");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(a.outbox, vec![Command::Submit { text: "hello".into() }]);
        assert_eq!(a.log, vec![LogEntry::UserMessage { text: "hello".into() }]);
        assert!(a.awaiting_turn);
    }

    #[test]
    fn a_slash_in_an_empty_field_opens_the_menu_and_elsewhere_types() {
        let mut a = app();
        type_str(&mut a, "a/b");
        assert!(matches!(a.mode, Mode::Conversation));
        assert_eq!(a.input, "a/b");
        a.input.clear();
        a.cursor = 0;
        a.handle_key(press(KeyCode::Char('/')));
        let Mode::Commands(menu) = &a.mode else { panic!("the menu opens") };
        assert_eq!(menu.list.rows.len(), 4);
        assert_eq!(menu.list.rows[0].label, "/resume");
    }

    #[test]
    fn the_menu_filters_by_what_is_typed_and_backspace_past_the_slash_closes_it() {
        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        type_str(&mut a, "cl");
        let Mode::Commands(menu) = &a.mode else { panic!() };
        assert_eq!(menu.matching().len(), 1);
        assert_eq!(menu.matching()[0].0, "clear");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(a.outbox, vec![Command::Submit { text: "/clear".into() }]);

        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        a.handle_key(press(KeyCode::Backspace));
        assert!(matches!(a.mode, Mode::Conversation));
    }

    #[test]
    fn quit_from_the_menu_submits_exit() {
        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        a.handle_key(press(KeyCode::Char('3')));
        assert_eq!(a.outbox, vec![Command::Submit { text: "/exit".into() }]);
    }

    #[test]
    fn model_from_the_menu_asks_provider_then_model_and_submits_the_command() {
        let mut a = app().with_catalogue(catalogue(), Some("anthropic".into()));
        a.handle_key(press(KeyCode::Char('/')));
        a.handle_key(press(KeyCode::Char('2')));
        let Mode::Question(asking) = &a.mode else { panic!("provider question") };
        assert!(matches!(asking.asker, Asker::Provider { .. }));
        assert_eq!(asking.list.selected, 0, "opens on the current provider");
        a.handle_key(press(KeyCode::Char('2')));
        let Mode::Question(asking) = &a.mode else { panic!("model question") };
        assert!(matches!(&asking.asker, Asker::Model { provider, .. } if provider == "openai"));
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(a.outbox, vec![Command::Submit { text: "/model openai/gpt-5".into() }]);
    }

    /// No wizard: with nothing configured the first message is held, the
    /// two questions are asked, and then both the `/model` and the message
    /// go, in that order.
    #[test]
    fn with_no_model_the_first_message_waits_for_the_two_questions() {
        let mut a = App::new(String::new()).with_catalogue(catalogue(), None);
        type_str(&mut a, "add rate limiting");
        a.handle_key(press(KeyCode::Enter));
        assert!(matches!(&a.mode, Mode::Question(q) if matches!(q.asker, Asker::Provider { then: Some(_) })));
        assert!(a.outbox.is_empty());
        a.handle_key(press(KeyCode::Char('1')));
        a.handle_key(press(KeyCode::Char('1')));
        assert_eq!(a.outbox, vec![Command::Submit { text: "/model anthropic/claude-sonnet-5".into() }, Command::Submit { text: "add rate limiting".into() }]);
    }

    #[test]
    fn an_agent_question_takes_the_band_and_a_number_answers_it() {
        let mut a = app();
        a.apply_event(Event::QuestionAsked {
            call_id:  "q1".into(),
            question: Question { question: "Limit anonymous?".into(), detail: "why".into(), options: vec!["Yes".into(), "No".into(), CHAT_ABOUT_THIS.into()] },
        });
        assert!(matches!(a.mode, Mode::Question(_)));
        type_str(&mut a, "x");
        assert!(a.input.is_empty(), "no typing ahead while a question is open");
        a.handle_key(press(KeyCode::Char('2')));
        assert_eq!(a.outbox, vec![Command::Answer { call_id: "q1".into(), answer: Answer::Chose { index: 1 } }]);
        assert!(matches!(a.mode, Mode::Conversation));
    }

    #[test]
    fn chat_about_this_makes_the_next_message_the_answer() {
        let mut a = app();
        a.apply_event(Event::QuestionAsked {
            call_id:  "q1".into(),
            question: Question { question: "Q?".into(), detail: String::new(), options: vec!["Yes".into(), CHAT_ABOUT_THIS.into()] },
        });
        a.handle_key(press(KeyCode::Esc));
        assert_eq!(a.answering.as_deref(), Some("q1"));
        type_str(&mut a, "only for keyed requests");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(a.outbox, vec![Command::Answer { call_id: "q1".into(), answer: Answer::Said { text: "only for keyed requests".into() } }]);
        assert!(a.answering.is_none());
    }

    /// The bug this pins: `finish_call` used to stop at the first `Work`
    /// entry whether or not it held the call, so an `ask` answered after any
    /// other work in the turn never reached its question row.
    #[test]
    fn an_answered_question_gets_its_answer_even_after_other_work() {
        let mut a = app();
        a.log.push(LogEntry::UserMessage { text: "go".into() });
        a.log.push(LogEntry::Work { items: vec![], open: false });
        a.apply_event(Event::QuestionAsked {
            call_id:  "q1".into(),
            question: Question { question: "Q?".into(), detail: String::new(), options: vec!["Yes".into(), CHAT_ABOUT_THIS.into()] },
        });
        a.handle_key(press(KeyCode::Char('1')));
        a.apply_event(Event::ToolCompleted { turn_id: TurnId(1), step_id: StepId(1), result: ToolResult { call_id: "q1".into(), content: "The developer chose: Yes".into(), is_error: false } });
        assert!(matches!(a.log.last(), Some(LogEntry::Question { answer: Some(ans), .. }) if ans == "Yes"), "{:?}", a.log);
    }

    #[test]
    fn a_review_over_nothing_is_answered_rather_than_opened() {
        let mut a = app();
        a.apply_event(Event::ReviewRequested { review_id: "r".into(), changeset: Changeset::default() });
        assert!(matches!(a.mode, Mode::Conversation));
        assert_eq!(a.outbox, vec![Command::ReviewDecision { review_id: "r".into(), decision: ReviewDecision::Discard }]);
    }

    #[test]
    fn tool_calls_become_a_work_disclosure_with_facts() {
        let mut a = app();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        let call = ToolCall { id: "c1".into(), name: "read".into(), input: serde_json::json!({"path": "src/x.rs"}) };
        a.apply_event(Event::ToolUseRequested { turn_id: TurnId(1), step_id: StepId(1), call });
        a.apply_event(Event::ToolDispatched { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into() });
        a.apply_event(Event::ToolCompleted { turn_id: TurnId(1), step_id: StepId(1), result: ToolResult { call_id: "c1".into(), content: "a\nb\n".into(), is_error: false } });
        let Some(LogEntry::Work { items, open }) = a.log.last() else { panic!("{:?}", a.log) };
        assert!(!open);
        assert_eq!(items[0].verb, "Read");
        assert_eq!(items[0].target, "src/x.rs");
        assert_eq!(items[0].fact.as_deref(), Some("2 lines"));
    }

    #[test]
    fn space_on_an_empty_field_toggles_the_details_and_otherwise_types() {
        let mut a = app();
        a.log.push(LogEntry::UserMessage { text: "go".into() });
        a.log.push(LogEntry::Work { items: vec![], open: false });
        a.handle_key(press(KeyCode::Char(' ')));
        assert!(matches!(a.log.last(), Some(LogEntry::Work { open: true, .. })));
        assert!(a.input.is_empty());
        type_str(&mut a, "a ");
        assert_eq!(a.input, "a ");
    }

    #[test]
    fn the_plan_is_one_entry_per_turn_replaced_in_place() {
        let mut a = app();
        a.log.push(LogEntry::UserMessage { text: "go".into() });
        let step = |t: &str, s| PlanStep { text: t.into(), state: s };
        a.apply_event(Event::PlanUpdated { turn_id: TurnId(1), steps: vec![step("Count", aldwin_core::StepState::Running)] });
        a.apply_event(Event::PlanUpdated { turn_id: TurnId(1), steps: vec![step("Count", aldwin_core::StepState::Done), step("Check", aldwin_core::StepState::Running)] });
        assert_eq!(a.log.iter().filter(|e| matches!(e, LogEntry::Plan { .. })).count(), 1);
        let Some(LogEntry::Plan { steps }) = a.log.last() else { panic!() };
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn a_review_takes_the_screen_and_its_decision_is_sent_with_its_id() {
        let mut a = app();
        let changeset = Changeset { files: vec![ChangedFile { path: "f.rs".into(), before: Some("x\n".into()), after: "y\n".into() }] };
        a.apply_event(Event::ReviewRequested { review_id: "review-1".into(), changeset });
        assert!(matches!(a.mode, Mode::Review(_)));
        a.review_mut().unwrap().mark_read();
        a.handle_key(press_mod(KeyCode::Enter, KeyModifiers::CONTROL));
        assert_eq!(a.outbox, vec![Command::ReviewDecision { review_id: "review-1".into(), decision: ReviewDecision::Approve }]);
        a.apply_event(Event::ReviewClosed { outcome: ReviewOutcome::Saved { files: vec!["f.rs".into()], comments_resolved: 0 } });
        assert!(matches!(a.mode, Mode::Conversation));
        assert!(matches!(a.log.last(), Some(LogEntry::Review { .. })));
    }

    #[test]
    fn typing_in_a_review_goes_to_its_field_and_enter_sends_it_as_a_comment() {
        let mut a = app();
        let changeset = Changeset { files: vec![ChangedFile { path: "f.rs".into(), before: Some("x\n".into()), after: "y\n".into() }] };
        a.apply_event(Event::ReviewRequested { review_id: "r".into(), changeset });
        type_str(&mut a, "rename it");
        assert_eq!(a.input, "rename it");
        a.handle_key(press(KeyCode::Enter));
        let Some(Command::ReviewDecision { decision: ReviewDecision::Comment { comments }, .. }) = a.outbox.last() else { panic!("{:?}", a.outbox) };
        assert_eq!(comments[0].text, "rename it");
        assert!(a.input.is_empty());
    }

    #[test]
    fn the_context_bar_reads_the_last_steps_prompt_over_the_window() {
        let mut a = app().with_catalogue(catalogue(), Some("anthropic".into()));
        assert_eq!(a.status.context_window, Some(1_000_000));
        assert_eq!(a.status.context_percent(), None, "nothing measured yet");
        a.apply_event(Event::StepEnded {
            turn_id: TurnId(1),
            step_id: StepId(1),
            outcome: StepOutcome {
                stop_reason: StopReason::EndTurn,
                usage: UsageStats { input_tokens: 300_000, output_tokens: 10 },
                cache: CacheStats { cache_creation_input_tokens: 0, cache_read_input_tokens: 110_000 },
            },
        });
        assert_eq!(a.status.context_percent(), Some(41));
    }

    #[test]
    fn a_failed_turn_is_a_sentence_with_its_detail_folded() {
        let mut a = app();
        a.apply_event(Event::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::Error("boom\nstack".into()) });
        assert!(matches!(&a.log[0], LogEntry::Failure { message, detail: Some(d), open: false } if message.contains("boom") && d.contains("stack")));
        assert_eq!(a.log[1], LogEntry::TurnBreak);

        // A provider's body is a detail, not a sentence.
        let body = r#"provider error 400: {"error":{"message":"the request was malformed"}}"#;
        a.apply_event(Event::TurnEnded { turn_id: TurnId(2), reason: TurnEndReason::Error(body.into()) });
        assert!(matches!(&a.log[2], LogEntry::Failure { message, detail: Some(d), .. } if message == "The turn did not finish: provider error 400." && d == body));
    }

    #[test]
    fn ctrl_c_cancels_a_running_turn_then_quits_on_repeat() {
        let mut a = app();
        a.turn_active = true;
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(a.outbox, vec![Command::Cancel]);
        assert!(!a.should_quit);
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.should_quit);
    }

    #[test]
    fn escape_stops_a_running_turn_and_does_nothing_idle() {
        let mut a = app();
        a.handle_key(press(KeyCode::Esc));
        assert!(a.outbox.is_empty() && !a.should_quit);
        a.turn_active = true;
        a.handle_key(press(KeyCode::Esc));
        assert_eq!(a.outbox, vec![Command::Cancel]);
    }

    #[test]
    fn history_loaded_replays_work_and_turn_breaks() {
        let mut a = app();
        a.apply_event(Event::HistoryLoaded {
            records: vec![
                LogRecord::TurnStarted { turn_id: TurnId(1) },
                LogRecord::UserMessage { turn_id: TurnId(1), text: "hi".into() },
                LogRecord::ToolUse { turn_id: TurnId(1), step_id: StepId(1), call: ToolCall { id: "c1".into(), name: "run".into(), input: serde_json::json!({"program": "ls"}) } },
                LogRecord::ToolResult { turn_id: TurnId(1), step_id: StepId(1), result: ToolResult { call_id: "c1".into(), content: "ok".into(), is_error: false } },
                LogRecord::AssistantMessage { turn_id: TurnId(1), step_id: StepId(1), text: "Done.".into() },
                LogRecord::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::EndTurn },
            ],
        });
        assert!(matches!(&a.log[1], LogEntry::Work { items, .. } if items[0].fact.as_deref() == Some("ok")));
        assert_eq!(a.log.last(), Some(&LogEntry::TurnBreak));
    }
}
