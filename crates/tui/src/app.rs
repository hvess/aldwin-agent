use std::collections::HashMap;
use std::time::Duration;

use aldwin_core::{
    Answer, Command, Event, LogRecord, PlanStep, Question, ReviewDecision, ReviewOutcome,
    StepState, TurnEndReason,
};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::text::Line;

use crate::activity::Activity;
use crate::draft::{self, Draft};
use crate::list::{List, ListOutcome, ListRow};
use crate::log::{plural, Log, LogEntry, Took, WorkItem};
use crate::motion::{ticks, Motion};
use crate::palette::Theme;
use crate::resume::SessionChoice;
use crate::review::{Review, ReviewOutcome as ReviewKey};
use crate::scroll::{ScrollState, WHEEL_ROWS};
use crate::ui::Transcript;
use crate::version::{GIT_HASH, VERSION};

/// Ticks within which a second Ctrl+C quits.
const DOUBLE_CTRL_C_TICKS: u64 = ticks(Duration::from_secs(2));

/// One catalogue provider, formatted by aldwin-cli; this crate never sees
/// an endpoint or key variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderChoice {
    /// The catalogue id, the `p` of `/model p/m`.
    pub id: String,
    /// A short description, beside its row.
    pub purpose: String,
    /// Its models, in listed order.
    pub models: Vec<ModelChoice>,
    /// The subscription an account needs (ADR 0012), shown in the
    /// `/connect` list; `None` for a key-only provider.
    pub account: Option<String>,
}

/// One model a provider offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    /// The model id, the `m` of `/model p/m`.
    pub id: String,
    /// A short description, beside its row.
    pub purpose: String,
    /// Context window in tokens; the context bar's denominator.
    pub context: u32,
}

/// One row of the `/` menu; aldwin-cli owns the commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandChoice {
    /// The command's name, without its slash.
    pub name: String,
    /// What it is for, beside its row.
    pub summary: String,
}

/// Who asked the question on screen; decides where the answer goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asker {
    /// The agent, through `ask`; answered with a `Command::Answer`.
    Agent { call_id: String },
    /// No model is configured: which provider. The answer opens the model
    /// question; `then` is the held first message.
    Provider { then: Option<String> },
    /// Bare `/connect`: which account (ADR 0012). The answer submits
    /// `/connect <provider>`.
    Connection,
    /// Which of `provider`'s models. The answer submits `/model p/m`, then
    /// `then` if any.
    Model {
        provider: String,
        then: Option<String>,
    },
    /// Bare `/resume`: which session. The answer submits `/resume <id>`.
    Session,
}

/// A question on screen: the design's `QuestionPanel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asking {
    pub question: Question,
    pub list: List,
    pub asker: Asker,
    /// The waiting review the agent's question is drawn over, in its bottom
    /// band; back on screen once the question leaves it.
    covers: Option<Box<Review>>,
}

impl Asking {
    fn new(question: Question, list: List, asker: Asker) -> Self {
        Self {
            question,
            list,
            asker,
            covers: None,
        }
    }

    /// What holds the screen once this question leaves it: the review it
    /// covered, or the conversation.
    fn leave(&mut self) -> Mode {
        self.covers
            .take()
            .map_or(Mode::Conversation, |review| Mode::Review(*review))
    }
}

/// The `/` menu: commands whose name starts with `filter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMenu {
    pub filter: String,
    pub list: List,
}

impl CommandMenu {
    fn open(commands: &[CommandChoice]) -> Self {
        let mut menu = Self {
            filter: String::new(),
            list: List::new(Vec::new()),
        };
        menu.refilter(commands);
        menu
    }

    /// Narrows the rows to the filter and selects the top match, which
    /// frame F completes.
    fn refilter(&mut self, commands: &[CommandChoice]) {
        let rows = commands
            .iter()
            .filter(|c| c.name.starts_with(self.filter.as_str()))
            .map(|c| ListRow::with_detail(format!("/{}", c.name), c.summary.clone()))
            .collect();
        self.list = List::new(rows);
    }

    /// What is typed so far, slash included.
    fn typed(&self) -> String {
        format!("/{}", self.filter)
    }

    /// The current row's name, without its slash.
    fn current(&self) -> Option<&str> {
        let row = self.list.rows.get(self.list.selected)?;
        Some(row.label.strip_prefix('/').unwrap_or(&row.label))
    }

    /// The current command's untyped rest: the field's grey completion.
    pub(crate) fn completion(&self) -> &str {
        self.current()
            .and_then(|name| name.strip_prefix(self.filter.as_str()))
            .unwrap_or_default()
    }

    /// Whether the filter spells a whole command; the only time the field's
    /// text is blue.
    pub(crate) fn spells_a_command(&self) -> bool {
        self.list
            .rows
            .iter()
            .any(|row| row.label.strip_prefix('/') == Some(self.filter.as_str()))
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
    /// The bare model id; empty when nothing is configured.
    pub model_name: String,
    /// The release version.
    pub version: String,
    /// The build's short git hash.
    pub commit: String,
    /// The working directory's name.
    pub project: String,
    /// The git branch, if a checkout.
    pub branch: Option<String>,
    /// The model's context window in tokens, if the catalogue knows it.
    pub context_window: Option<u32>,
    /// Tokens the last request carried.
    pub context_used: Option<u32>,
}

impl StatusInfo {
    /// The context bar's percentage, capped at 100; `None` unless both
    /// halves are known and the window is non-zero.
    pub fn context_percent(&self) -> Option<u8> {
        let (used, window) = (self.context_used?, self.context_window?);
        if window == 0 {
            return None;
        }
        Some(((u64::from(used) * 100 / u64::from(window)).min(100)) as u8)
    }
}

/// Application state and the logic that mutates it. `ui/` only reads it;
/// `run.rs` only feeds it events and keys and interprets nothing, so all
/// behaviour is testable without a terminal.
#[derive(Debug)]
pub struct App {
    pub(crate) log: Log,
    pub(crate) mode: Mode,
    pub(crate) scroll: ScrollState,
    /// The log area's width, cached by `ui::draw` for scrolling between
    /// draws.
    pub(crate) render_width: u16,
    /// What is typed into the field.
    pub(crate) draft: Draft,
    /// Messages sent while a turn runs, in order (frame K): sent as one
    /// turn when it finishes, or back into the field on `esc`, a stop or a
    /// failure with no review on screen. Never a `/` command.
    pub(crate) queued: Vec<String>,
    /// The field's text width, cached by its draw like `render_width`.
    pub(crate) composer_width: u16,
    /// First visual row of the draft the field shows.
    pub(crate) composer_top: usize,
    pub(crate) status: StatusInfo,
    pub(crate) should_quit: bool,
    /// From `TurnStarted` until the matching `TurnEnded`.
    pub(crate) turn_active: bool,
    /// From a submitted message until its turn starts. Never set by a `/`
    /// command: the interceptor answers it with no turn, and core takes a
    /// message after it, so it must not queue one.
    pub(crate) awaiting_turn: bool,
    /// A stop was requested and the turn has not ended; another `esc` does
    /// nothing.
    stopping: bool,
    /// The agent's question the next submission answers in words ("Chat
    /// about this"); kept whole so `esc` can return to its options.
    pub(crate) answering: Option<Asking>,
    /// Whether the current turn's disclosures (work, thoughts, failure
    /// details) are open; Space toggles it (frames B, C and J).
    pub(crate) details_open: bool,
    /// The first line of the last message the developer typed, not a
    /// command, without a trailing `.` or `!`: a review's title. Never an
    /// echoed follow-up; `None` after `/clear` or `/resume` until one is
    /// typed.
    request: Option<String>,
    /// Index in `log` of the message that opened the current (or last)
    /// turn. Not the last `UserMessage`: a "Chat about this" answer is one
    /// too, mid-turn.
    turn_start: usize,
    last_ctrl_c: Option<u64>,
    pub(crate) tick: u64,
    /// What the running turn is doing, for the working line; stale while
    /// idle.
    pub(crate) activity: Activity,
    /// Filled on `ToolUseRequested` (the only event with a tool's name and
    /// input), consumed on `ToolDispatched`.
    pending_calls: HashMap<String, (String, serde_json::Value)>,
    /// Commands to send; `run.rs` drains it after each call.
    pub(crate) outbox: Vec<Command>,
    catalogue: Vec<ProviderChoice>,
    current_provider: Option<String>,
    sessions: Vec<SessionChoice>,
    commands: Vec<CommandChoice>,
    pub(crate) theme: Theme,
    pub(crate) motion: Motion,
    transcript: Transcript,
}

impl App {
    /// A session with an empty log on `model_name`, which is empty when
    /// nothing is configured.
    pub fn new(model_name: String) -> Self {
        Self {
            log: Log::default(),
            mode: Mode::Conversation,
            scroll: ScrollState::default(),
            render_width: 80,
            draft: Draft::default(),
            queued: Vec::new(),
            composer_width: 74,
            composer_top: 0,
            status: StatusInfo {
                model_name,
                version: VERSION.to_string(),
                commit: GIT_HASH.to_string(),
                project: String::new(),
                branch: None,
                context_window: None,
                context_used: None,
            },
            should_quit: false,
            turn_active: false,
            awaiting_turn: false,
            stopping: false,
            answering: None,
            details_open: false,
            request: None,
            turn_start: 0,
            last_ctrl_c: None,
            tick: 0,
            activity: Activity::default(),
            pending_calls: HashMap::new(),
            outbox: Vec::new(),
            catalogue: Vec::new(),
            current_provider: None,
            sessions: Vec::new(),
            commands: Vec::new(),
            theme: Theme::default(),
            motion: Motion::default(),
            transcript: Transcript::default(),
        }
    }

    /// The past sessions bare `/resume` offers, newest first.
    pub fn with_sessions(mut self, sessions: Vec<SessionChoice>) -> Self {
        self.sessions = sessions;
        self
    }

    /// The `/` menu's rows, in the order drawn.
    pub fn with_commands(mut self, commands: Vec<CommandChoice>) -> Self {
        self.commands = commands;
        self
    }

    /// The catalogue and the session's provider row; sets `context_window`
    /// when the row and model are found.
    pub fn with_catalogue(
        mut self,
        catalogue: Vec<ProviderChoice>,
        current_provider: Option<String>,
    ) -> Self {
        self.catalogue = catalogue;
        self.current_provider = current_provider;
        self.status.context_window =
            self.context_window_for(self.current_provider.as_deref(), &self.status.model_name);
        self
    }

    /// The starting theme.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Whether the caret and the working line move.
    ///
    /// ```
    /// use aldwin_tui::{App, Motion};
    ///
    /// let _app = App::new("claude-sonnet-5".into()).with_motion(Motion::Reduced);
    /// ```
    pub fn with_motion(mut self, motion: Motion) -> Self {
        self.motion = motion;
        self
    }

    /// The project name and git branch for the launch card, read by
    /// aldwin-cli.
    pub fn with_facts(mut self, project: &str, branch: Option<&str>) -> Self {
        self.status.project = project.into();
        self.status.branch = branch.map(str::to_string);
        self
    }

    fn context_window_for(&self, provider: Option<&str>, model: &str) -> Option<u32> {
        let p = self
            .catalogue
            .iter()
            .find(|p| Some(p.id.as_str()) == provider)?;
        p.models.iter().find(|m| m.id == model).map(|m| m.context)
    }

    /// While the working line is not on screen its stall clock does not
    /// run: a question or a review is the developer's time, not a stall.
    pub(crate) fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if !self.is_working() {
            self.activity.touch(self.tick);
        }
    }

    /// Whether anything on screen reads `tick`: the caret, blinking wherever
    /// a field is drawn, and the working line under it. Under reduced motion
    /// only the working line's timer moves.
    pub(crate) fn is_animating(&self) -> bool {
        match self.motion {
            Motion::Full => matches!(
                self.mode,
                Mode::Conversation | Mode::Review(_) | Mode::Commands(_)
            ),
            Motion::Reduced => self.is_working(),
        }
    }

    /// The review on screen, under the agent's question too.
    pub(crate) fn review(&self) -> Option<&Review> {
        match &self.mode {
            Mode::Review(r) => Some(r),
            Mode::Question(asking) => asking.covers.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn review_mut(&mut self) -> Option<&mut Review> {
        match &mut self.mode {
            Mode::Review(r) => Some(r),
            Mode::Question(asking) => asking.covers.as_deref_mut(),
            _ => None,
        }
    }

    /// True while something other than the field takes keys. A review's
    /// field takes them only while it answers the agent in words.
    fn band_is_held(&self) -> bool {
        match self.mode {
            Mode::Conversation => false,
            Mode::Review(_) => self.answering.is_none(),
            Mode::Question(_) | Mode::Commands(_) => true,
        }
    }

    fn busy(&self) -> bool {
        self.turn_active || self.awaiting_turn
    }

    /// Whether the footer is the working line: a turn runs and nothing on
    /// screen waits on the developer.
    pub(crate) fn is_working(&self) -> bool {
        self.busy()
            && self.answering.is_none()
            && match &self.mode {
                Mode::Conversation => true,
                Mode::Review(review) => review.waiting(),
                Mode::Question(_) | Mode::Commands(_) => false,
            }
    }

    /// The current turn's entries, or the last turn's while idle.
    pub(crate) fn this_turn(&self) -> &[LogEntry] {
        self.log.tail(self.turn_start)
    }

    fn this_turn_mut(&mut self) -> &mut [LogEntry] {
        self.log.tail_mut(self.turn_start)
    }

    /// Logs `message` and marks it as the turn's start.
    fn open_turn(&mut self, message: String) {
        self.turn_start = self.log.len();
        self.push(LogEntry::UserMessage { text: message });
    }

    /// On `TurnStarted` (live or replayed), the turn starts at the last
    /// `UserMessage`; nothing is said inside a turn before it begins.
    fn mark_turn_started(&mut self) {
        if let Some(i) = self
            .log
            .iter()
            .rposition(|e| matches!(e, LogEntry::UserMessage { .. }))
        {
            self.turn_start = i;
        }
    }

    fn reset_conversation(&mut self) {
        self.request = None;
        self.turn_start = 0;
        self.log.clear();
        self.scroll = ScrollState::default();
        self.turn_active = false;
        self.awaiting_turn = false;
        self.stopping = false;
        self.details_open = false;
    }

    fn push(&mut self, entry: LogEntry) {
        self.log.push(entry);
        let total = self.total_lines();
        self.scroll.on_content_grew(total);
    }

    pub(crate) fn total_lines(&mut self) -> usize {
        self.sync_transcript();
        self.transcript.len()
    }

    #[cfg(test)]
    pub(crate) fn blocks_rebuilt(&self) -> usize {
        self.transcript.rebuilt()
    }

    pub(crate) fn transcript_view(&mut self, height: usize) -> Vec<Line<'static>> {
        self.sync_transcript();
        let total = self.transcript.len();
        self.scroll.set_viewport_height(height, total);
        self.transcript.viewport(self.scroll.offset, height)
    }

    fn sync_transcript(&mut self) {
        let changed = self.log.take_changed();
        let mut transcript = std::mem::take(&mut self.transcript);
        transcript.sync(self, self.render_width, changed);
        self.transcript = transcript;
    }

    /// The streaming thinking block's text and time: the last entry, while
    /// it runs.
    fn running_thought(&mut self) -> Option<(&mut String, &mut Took)> {
        match self.log.last_mut() {
            Some(LogEntry::Thinking { text, took, .. }) if *took == Took::Running => {
                Some((text, took))
            }
            _ => None,
        }
    }

    /// The current step's `Work` items: the last entry, if it is `Work`.
    fn open_work(&mut self) -> Option<&mut Vec<WorkItem>> {
        match self.log.last_mut() {
            Some(LogEntry::Work { items, .. }) => Some(items),
            _ => None,
        }
    }

    fn record_call(&mut self, call_id: String, name: &str, input: &serde_json::Value) {
        let Some((verb, target)) = WorkItem::describe(name, input) else {
            return;
        };
        let item = WorkItem {
            call_id,
            verb,
            target,
            fact: None,
            failed: false,
        };
        let open = self.details_open;
        match self.open_work() {
            Some(items) => items.push(item),
            None => self.push(LogEntry::Work {
                items: vec![item],
                open,
            }),
        }
    }

    /// Hands the working line the turn's latest call still running: a
    /// `WorkItem` with no fact yet.
    fn follow_calls(&mut self) {
        let running = self
            .this_turn()
            .iter()
            .rev()
            .find_map(|entry| match entry {
                LogEntry::Work { items, .. } => items.iter().rev().find(|i| i.fact.is_none()),
                _ => None,
            })
            .cloned();
        self.activity.calls_changed(running.as_ref(), self.tick);
    }

    fn finish_call(&mut self, call_id: &str, content: &str, is_error: bool) {
        // Within this turn: `turn_start` moves only between turns, and a
        // cancelled turn's results are sent before it ends (`abort_dispatch`),
        // so the call was made after it. Every `Work` entry of it, newest first, not
        // just the last one; found before it is borrowed mutably, so only
        // that entry counts as changed.
        let start = self.turn_start.min(self.log.len());
        let turn = self.log.tail(start);
        if let Some(i) = turn.iter().rposition(|e| {
            matches!(e, LogEntry::Work { items, .. } if items.iter().any(|i| i.call_id == call_id))
        }) {
            if let LogEntry::Work { items, .. } = &mut self.log[start + i] {
                if let Some(item) = items.iter_mut().find(|i| i.call_id == call_id) {
                    item.failed = is_error;
                    item.fact = Some(item.verb.fact(content, is_error));
                }
            }
            return;
        }
        // Otherwise an `ask` result answers the newest open question row.
        // `plan` results are dropped.
        if let Some(i) = turn
            .iter()
            .rposition(|e| matches!(e, LogEntry::Question { answer: None, .. }))
        {
            if let LogEntry::Question { answer, .. } = &mut self.log[start + i] {
                *answer = Some(Answer::words_of(content).to_string());
            }
        }
    }

    /// One loaded record, as the entry the live path would produce.
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
            LogRecord::ToolUse { call, .. } => self.record_call(call.id, &call.name, &call.input),
            LogRecord::ToolResult { result, .. } => {
                self.finish_call(&result.call_id, &result.content, result.is_error)
            }
            LogRecord::TurnEnded { reason, .. } => self.push_turn_end(reason),
            LogRecord::Thinking { text, seconds, .. } => self.log.push(LogEntry::Thinking {
                text,
                took: seconds.into(),
                open: false,
            }),
            // Encrypted: nothing to show.
            LogRecord::RedactedThinking { .. } => {}
            LogRecord::TurnStarted { .. } => self.mark_turn_started(),
            LogRecord::StepBoundary { .. } => {}
        }
    }

    fn push_turn_end(&mut self, reason: TurnEndReason) {
        // Amber means running only: an ended turn's running step goes back
        // to pending, and a thought it cut off no longer says "Thinking".
        for entry in self.this_turn_mut() {
            match entry {
                LogEntry::Plan { steps } => {
                    for step in steps.iter_mut().filter(|s| s.state == StepState::Running) {
                        step.state = StepState::Pending;
                    }
                }
                LogEntry::Thinking { took, .. } if *took == Took::Running => *took = Took::Unknown,
                _ => {}
            }
        }
        match reason {
            TurnEndReason::EndTurn => self.push(LogEntry::TurnBreak),
            TurnEndReason::Cancelled => {
                // One sentence per stop: the first `Stopping` becomes
                // "Stopped." in place; repeats are removed.
                let stopped = LogEntry::Failure {
                    message: "Stopped.".into(),
                    detail: None,
                    open: false,
                };
                let start = self.turn_start.min(self.log.len());
                let notices: Vec<usize> = (start..self.log.len())
                    .filter(|&i| matches!(self.log[i], LogEntry::Stopping { .. }))
                    .collect();
                match notices.split_first() {
                    Some((&first, repeats)) => {
                        for &i in repeats.iter().rev() {
                            self.log.remove(i);
                        }
                        self.log[first] = stopped;
                    }
                    None => self.push(stopped),
                }
                self.push(LogEntry::TurnBreak);
            }
            TurnEndReason::Error(failure) => {
                self.push(LogEntry::failed_turn(failure));
                self.push(LogEntry::TurnBreak);
            }
        }
    }

    /// Folds one core event into the log, mode and status; any command it
    /// calls for goes to the outbox.
    pub fn apply_event(&mut self, event: Event) {
        self.activity.touch(self.tick);
        match event {
            Event::TurnStarted { .. } => {
                if !self.busy() {
                    self.activity = Activity::new(self.tick);
                }
                self.mark_turn_started();
                self.turn_active = true;
                self.awaiting_turn = false;
                self.stopping = false;
                self.details_open = false;
            }
            Event::TextDelta { text, .. } => {
                self.activity.replying(self.tick);
                if let Some(LogEntry::AssistantText { text: buf }) = self.log.last_mut() {
                    buf.push_str(&text);
                } else {
                    self.push(LogEntry::AssistantText { text });
                }
            }
            Event::ThinkingStart { .. } => {
                self.activity.thinking(self.tick);
                let open = self.details_open;
                self.push(LogEntry::Thinking {
                    text: String::new(),
                    took: Took::Running,
                    open,
                });
            }
            Event::ThinkingDelta { text, .. } => {
                self.activity.thinking(self.tick);
                if let Some((buf, _)) = self.running_thought() {
                    buf.push_str(&text);
                }
            }
            Event::ThinkingEnd { seconds, .. } => {
                if let Some((_, took)) = self.running_thought() {
                    *took = seconds.into();
                }
            }
            Event::ToolUseRequested { call, .. } => {
                self.pending_calls.insert(call.id, (call.name, call.input));
            }
            Event::ToolDispatched { call_id, .. } => {
                if let Some((name, input)) = self.pending_calls.remove(&call_id) {
                    self.record_call(call_id, &name, &input);
                    self.follow_calls();
                }
            }
            Event::ToolCompleted { result, .. } => {
                // A call refused before dispatch never got `ToolDispatched`.
                if let Some((name, input)) = self.pending_calls.remove(&result.call_id) {
                    self.record_call(result.call_id.clone(), &name, &input);
                }
                self.finish_call(&result.call_id, &result.content, result.is_error);
                self.follow_calls();
            }
            Event::StepEnded { outcome, .. } => {
                self.status.context_used = Some(
                    outcome.usage.input_tokens
                        + outcome.cache.cache_read_input_tokens
                        + outcome.cache.cache_creation_input_tokens,
                );
            }
            Event::RetryAttempt { info, .. } => {
                self.activity.thinking(self.tick);
                self.push(LogEntry::retry(&info));
            }
            Event::TurnEnded { reason, .. } => {
                let finished = matches!(reason, TurnEndReason::EndTurn);
                self.pending_calls.clear();
                self.turn_active = false;
                self.awaiting_turn = false;
                self.stopping = false;
                // The agent's pending question ends with the turn.
                self.answering = None;
                if let Mode::Question(asking) = &mut self.mode {
                    if matches!(asking.asker, Asker::Agent { .. }) {
                        self.mode = asking.leave();
                    }
                }
                if self
                    .review()
                    .is_some_and(|r| r.closes_at_turn_end(finished))
                {
                    self.mode = Mode::Conversation;
                }
                self.push_turn_end(reason);
                // Never into a review's field, which shares `draft`.
                if finished {
                    self.send_queue();
                } else if self.review().is_none() {
                    self.take_back_queue();
                }
            }
            // Review comments, echoed like a typed message; the only
            // message the TUI did not send.
            Event::FollowUp { text, .. } => {
                if let Some(review) = self.review_mut() {
                    review.follow_up_started();
                }
                self.activity = Activity::new(self.tick);
                self.awaiting_turn = true;
                self.open_turn(text);
            }
            Event::PlanUpdated { steps, .. } => self.set_plan(steps),
            Event::QuestionAsked { call_id, question } => {
                self.push(LogEntry::Question {
                    question: question.question.clone(),
                    answer: None,
                });
                let rows = question.options.iter().cloned().map(ListRow::new).collect();
                self.show_question(Asking::new(
                    question,
                    List::new(rows),
                    Asker::Agent { call_id },
                ));
            }
            Event::ReviewRequested {
                review_id,
                changeset,
            } => {
                // An empty changeset (the dispatcher never sends one) is
                // answered with a discard rather than opened.
                match Review::open(review_id.clone(), changeset) {
                    Some(mut review) => {
                        // The agent's next round after comments replaces
                        // the waiting review in place.
                        match (self.review(), &self.request) {
                            (Some(previous), _) => review.carry_from(previous),
                            (None, Some(request)) => review.title = request.clone(),
                            (None, None) => {}
                        }
                        self.mode = Mode::Review(review);
                    }
                    None => self.outbox.push(Command::ReviewDecision {
                        review_id,
                        decision: ReviewDecision::Discard,
                    }),
                }
            }
            // Comments keep the review open until the agent's next changeset
            // replaces it, and get no row: at the turn's end they return as
            // a `FollowUp`, before a `run` as that step's result (ADR 0009
            // §4). Saved or discarded closes it with a row.
            Event::ReviewClosed {
                outcome: ReviewOutcome::Commented { .. },
            } => {
                if let Some(review) = self.review_mut() {
                    review.await_agent();
                }
            }
            Event::ReviewClosed { outcome } => {
                if matches!(self.mode, Mode::Review(_)) {
                    self.mode = Mode::Conversation;
                }
                self.push(LogEntry::Review { outcome });
            }
            Event::Notice { message } => {
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
                self.theme = Theme::from_config(Some(&theme));
            }
            Event::ModelChanged {
                provider,
                model,
                context_window,
            } => {
                self.status.context_window =
                    context_window.or_else(|| self.context_window_for(provider.as_deref(), &model));
                self.status.model_name = model;
                self.current_provider = provider;
            }
        }
    }

    /// Replaces the turn's one `Plan` entry in place, or adds it.
    fn set_plan(&mut self, steps: Vec<PlanStep>) {
        if let Some(entry) = self
            .this_turn_mut()
            .iter_mut()
            .find(|e| matches!(e, LogEntry::Plan { .. }))
        {
            *entry = LogEntry::Plan { steps };
        } else {
            self.push(LogEntry::Plan { steps });
        }
    }

    /// Handles a key press for whatever holds the screen; repeats and
    /// releases are ignored.
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
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => self.interrupt(),
            // Answering in words: back to the options, draft kept. Then the
            // queue back into the field; then stop while busy; nothing when
            // idle.
            (KeyCode::Esc, _) => {
                if !self.back_to_options() && !self.take_back_queue() && self.busy() {
                    self.stop("Stopping.");
                }
            }
            (KeyCode::Char('/'), _) if self.draft.is_empty() => {
                self.mode = Mode::Commands(CommandMenu::open(&self.commands))
            }
            (KeyCode::Char(' '), _) if self.draft.is_empty() && self.has_work() => {
                self.toggle_details()
            }
            (KeyCode::End, _) if self.draft.is_empty() => {
                let total = self.total_lines();
                self.scroll.jump_to_bottom(total);
            }
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
            _ => self.field_key(key),
        }
    }

    /// A key for the field: `↩` sends, `⇧↩`, `⌥↩` and `⌃J` break the line,
    /// the rest edit.
    fn field_key(&mut self, key: KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Enter, m) if m.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => {
                self.draft.insert('\n')
            }
            (KeyCode::Enter, _) => self.submit(),
            (KeyCode::Char('j'), m) if m.contains(KeyModifiers::CONTROL) => self.draft.insert('\n'),
            (code, modifiers) => {
                self.draft.edit(code, modifiers);
            }
        }
    }

    /// `esc` while answering in words: the question's options again, the
    /// draft kept. False when nothing was being answered.
    fn back_to_options(&mut self) -> bool {
        let Some(asking) = self.answering.take() else {
            return false;
        };
        self.show_question(asking);
        true
    }

    /// Puts `asking` on screen, over the review if one holds it; that review
    /// is a waiting one, since an open review holds the agent's turn.
    fn show_question(&mut self, mut asking: Asking) {
        if let Mode::Review(review) = std::mem::replace(&mut self.mode, Mode::Conversation) {
            asking.covers = Some(Box::new(review));
        }
        self.mode = Mode::Question(asking);
    }

    /// Whether the current turn has anything Space can open.
    fn has_work(&self) -> bool {
        self.this_turn().iter().any(LogEntry::has_details)
    }

    fn toggle_details(&mut self) {
        self.details_open = !self.details_open;
        let open = self.details_open;
        for entry in self.this_turn_mut() {
            match entry {
                LogEntry::Work { open: o, .. } | LogEntry::Thinking { open: o, .. } => *o = open,
                LogEntry::Failure {
                    open: o,
                    detail: Some(_),
                    ..
                } => *o = open,
                _ => {}
            }
        }
    }

    /// The `/` menu: letters filter, a digit picks a row. Any other
    /// character (a space before an argument, a `-`), or a filter matching
    /// nothing, closes the menu and moves the text into the field.
    fn handle_commands_key(&mut self, key: KeyEvent) {
        let Mode::Commands(menu) = &mut self.mode else {
            return;
        };
        let typing = !key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char(c) if typing && c.is_ascii_alphabetic() => {
                menu.filter.push(c.to_ascii_lowercase());
                menu.refilter(&self.commands);
                if menu.list.rows.is_empty() {
                    let typed = menu.typed();
                    self.leave_menu_typing(typed);
                }
                return;
            }
            KeyCode::Char(c) if typing && !c.is_ascii_digit() => {
                let typed = format!("{}{c}", menu.typed());
                self.leave_menu_typing(typed);
                return;
            }
            KeyCode::Backspace => {
                if menu.filter.pop().is_none() {
                    self.mode = Mode::Conversation;
                } else {
                    menu.refilter(&self.commands);
                }
                return;
            }
            _ => {}
        }
        match menu.list.handle_key(key.code, key.modifiers) {
            ListOutcome::Stay => {}
            ListOutcome::Close => self.mode = Mode::Conversation,
            ListOutcome::Chose(i) => {
                let command = menu.list.rows[i].label.clone();
                self.mode = Mode::Conversation;
                self.send(command);
            }
        }
    }

    fn leave_menu_typing(&mut self, typed: String) {
        self.mode = Mode::Conversation;
        self.draft.set(typed);
    }

    fn handle_question_key(&mut self, key: KeyEvent) {
        // `⌃C` on the agent's question interrupts the turn, not the list.
        let agent =
            matches!(&self.mode, Mode::Question(a) if matches!(a.asker, Asker::Agent { .. }));
        if agent && key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.interrupt();
            return;
        }
        let Mode::Question(asking) = &mut self.mode else {
            return;
        };
        let outcome = asking.list.handle_key(key.code, key.modifiers);
        let Mode::Question(mut asking) = std::mem::replace(&mut self.mode, Mode::Conversation)
        else {
            return;
        };
        match outcome {
            ListOutcome::Stay => self.mode = Mode::Question(asking),
            ListOutcome::Close => {
                self.mode = asking.leave();
                self.close_question(asking);
            }
            ListOutcome::Chose(i) => {
                self.mode = asking.leave();
                self.answer(asking, i);
            }
        }
    }

    /// `esc` on a question. The agent's cannot be dismissed (its tool
    /// waits), so it becomes "Chat about this"; the provider and model
    /// questions put their held message back in the field.
    fn close_question(&mut self, asking: Asking) {
        match asking.asker {
            Asker::Agent { .. } => self.answering = Some(asking),
            Asker::Provider { then } | Asker::Model { then, .. } => {
                if let Some(text) = then {
                    self.draft.set(text);
                }
            }
            Asker::Connection | Asker::Session => {}
        }
    }

    fn answer(&mut self, asking: Asking, index: usize) {
        match asking.asker {
            Asker::Agent { ref call_id } => {
                let chat = asking
                    .question
                    .options
                    .get(index)
                    .is_some_and(|o| Question::is_chat_about_this(o));
                if chat {
                    self.answering = Some(asking);
                } else {
                    self.outbox.push(Command::Answer {
                        call_id: call_id.clone(),
                        answer: Answer::Chose { index },
                    });
                }
            }
            Asker::Provider { then } => {
                if let Some(provider) = self.catalogue.get(index).cloned() {
                    self.open_model_question(provider, then);
                }
            }
            Asker::Connection => {
                if let Some(provider) = self.connectable().get(index) {
                    self.submit_text(format!("/connect {}", provider.id));
                }
            }
            Asker::Model { provider, then } => {
                let model = self
                    .catalogue
                    .iter()
                    .find(|p| p.id == provider)
                    .and_then(|p| p.models.get(index))
                    .map(|m| m.id.clone());
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
        let rows = self
            .catalogue
            .iter()
            .map(|p| ListRow::with_detail(p.id.clone(), p.purpose.clone()))
            .collect();
        let current = self
            .current_provider
            .as_deref()
            .and_then(|c| self.catalogue.iter().position(|p| p.id == c))
            .unwrap_or(0);
        let question = Question {
            question: "Where should the model run?".into(),
            detail: "Beside each: the key it reads from the environment, or /connect.".into(),
            options: self.catalogue.iter().map(|p| p.id.clone()).collect(),
        };
        self.mode = Mode::Question(Asking::new(
            question,
            List::new(rows).opened_on(current),
            Asker::Provider { then },
        ));
    }

    /// Catalogue rows reachable through an account.
    fn connectable(&self) -> Vec<&ProviderChoice> {
        self.catalogue
            .iter()
            .filter(|p| p.account.is_some())
            .collect()
    }

    /// Bare `/connect`: the connectable accounts (ADR 0012), each with its
    /// required subscription.
    fn open_connection_question(&mut self) {
        let connectable = self.connectable();
        let rows = connectable
            .iter()
            .map(|p| ListRow::with_detail(p.id.clone(), p.account.clone().unwrap_or_default()))
            .collect();
        let question = Question {
            question: "Which account?".into(),
            detail: "You sign in through your browser, and a model on that provider then runs on your subscription rather than an API key.".into(),
            options: connectable.iter().map(|p| p.id.clone()).collect(),
        };
        self.mode = Mode::Question(Asking::new(question, List::new(rows), Asker::Connection));
    }

    fn open_model_question(&mut self, provider: ProviderChoice, then: Option<String>) {
        let rows = provider
            .models
            .iter()
            .map(|m| ListRow::with_detail(m.id.clone(), m.purpose.clone()))
            .collect();
        let current = provider
            .models
            .iter()
            .position(|m| m.id == self.status.model_name)
            .unwrap_or(0);
        let question = Question {
            question: format!("Which {} model?", provider.id),
            detail: "Any model id the provider offers works; these are the known ones.".into(),
            options: provider.models.iter().map(|m| m.id.clone()).collect(),
        };
        self.mode = Mode::Question(Asking::new(
            question,
            List::new(rows).opened_on(current),
            Asker::Model {
                provider: provider.id,
                then,
            },
        ));
    }

    fn open_session_question(&mut self) {
        let rows = self
            .sessions
            .iter()
            .map(|s| {
                ListRow::with_detail(
                    s.title.clone(),
                    format!("{} · {}", s.when, plural(s.turns, "turn")),
                )
            })
            .collect();
        let question = Question {
            question: "Which conversation?".into(),
            detail: "Newest first. The one you pick continues in its own file.".into(),
            options: self.sessions.iter().map(|s| s.title.clone()).collect(),
        };
        self.mode = Mode::Question(Asking::new(question, List::new(rows), Asker::Session));
    }

    fn handle_review_key(&mut self, key: KeyEvent) {
        // `⌃C` never stops the turn behind a review: a stop leaves an
        // undecided one open with no turn to take its decision
        // (`Review::closes_at_turn_end`). A second press still quits.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.ctrl_c_repeated() {
                self.should_quit = true;
            }
            return;
        }
        // "Chat about this" over the review: the field takes the answer, as
        // the conversation's does, and `esc` goes back to the options.
        if self.answering.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.back_to_options();
                }
                KeyCode::Up => {
                    self.move_cursor_vertical(-1);
                }
                KeyCode::Down => {
                    self.move_cursor_vertical(1);
                }
                _ => self.field_key(key),
            }
            return;
        }
        // While the comments are with the agent, `esc` stops the turn, as
        // the footer says. With no turn running there is nothing to stop,
        // and `esc` leaves the review, so it always has a way out; a
        // changeset that comes later opens a review of its own.
        if key.code == KeyCode::Esc && self.review().is_some_and(Review::waiting) {
            if self.busy() {
                self.stop("Stopping.");
            } else {
                self.mode = Mode::Conversation;
            }
            return;
        }
        let general = self.draft.text().to_string();
        let typing = self
            .review()
            .is_some_and(|r| !r.commenting() && r.confirm.is_none());
        // Text keys edit `draft` (the review's "Ask for a change" field)
        // unless the comment field or discard question is open. Space, `?`
        // and Backspace are review keys until something is typed.
        let is_text = match key.code {
            KeyCode::Char(' ' | '?') => !general.is_empty(),
            KeyCode::Char(_) => !key.modifiers.contains(KeyModifiers::CONTROL),
            KeyCode::Backspace => !general.is_empty(),
            _ => false,
        };
        if typing && is_text {
            self.draft.edit(key.code, key.modifiers);
            return;
        }
        let Some(review) = self.review_mut() else {
            return;
        };
        match review.handle_key(key.code, key.modifiers, &general) {
            ReviewKey::Stay => {}
            ReviewKey::Decide(decision) => {
                let review_id = review.review_id.clone();
                if matches!(decision, ReviewDecision::Comment { .. }) {
                    self.draft.take();
                }
                self.outbox.push(Command::ReviewDecision {
                    review_id,
                    decision,
                });
            }
        }
    }

    /// Capture the mouse only while a review is open (ADR 0010); the
    /// conversation keeps the terminal's own selection.
    pub(crate) fn wants_mouse(&self) -> bool {
        self.review().is_some()
    }

    /// Routes a mouse event to the review if open; otherwise the wheel
    /// scrolls the transcript unless a question or the menu is up.
    pub fn handle_mouse(&mut self, event: MouseEvent) {
        if let Some(review) = self.review_mut() {
            review.handle_mouse(event.kind, event.column, event.row);
            return;
        }
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

    /// Inserts a bracketed paste into the field whole and sanitised, so its
    /// newlines never submit; ignored unless the field takes the keys.
    pub fn paste(&mut self, text: &str) {
        if self.band_is_held() {
            return;
        }
        self.draft.insert_str(&draft::sanitize(text));
    }

    fn move_cursor_vertical(&mut self, delta: isize) -> bool {
        let layout = self.draft.layout(self.composer_width as usize);
        match layout.step_row(self.draft.cursor(), delta) {
            Some(cursor) => {
                self.draft.move_to(cursor);
                true
            }
            None => false,
        }
    }

    /// `↩` in the field.
    fn submit(&mut self) {
        if self.draft.text().trim().is_empty() {
            return;
        }
        let text = self.draft.take();
        self.send(text);
    }

    /// The one path for a line typed or picked from the `/` menu, so both
    /// behave the same.
    fn send(&mut self, text: String) {
        // "Chat about this": what was typed answers the question.
        if let Some(asking) = self.answering.take() {
            if let Asker::Agent { call_id } = asking.asker {
                self.push(LogEntry::UserMessage { text: text.clone() });
                self.outbox.push(Command::Answer {
                    call_id,
                    answer: Answer::Said { text },
                });
            }
            return;
        }
        let command = text.trim();
        if self.busy() && !command.starts_with('/') {
            self.queued.push(text);
            let total = self.total_lines();
            self.scroll.on_content_grew(total);
            return;
        }
        // No model yet: hold the first message through the provider and
        // model questions, then send it.
        if self.status.model_name.is_empty()
            && !self.catalogue.is_empty()
            && !command.starts_with('/')
        {
            self.open_provider_question(Some(text));
            return;
        }
        // Bare `/resume`, `/model` and `/connect` are asked here as a list;
        // with nothing to list, the interceptor answers them.
        if command == "/resume" && !self.sessions.is_empty() {
            self.open_session_question();
            return;
        }
        if command == "/model" && !self.catalogue.is_empty() {
            self.open_provider_question(None);
            return;
        }
        if command == "/connect" && !self.connectable().is_empty() {
            self.open_connection_question();
            return;
        }
        self.submit_text(text);
    }

    /// Logs and submits `text`; every submission ends here. Mid-turn only a
    /// `/` command arrives, and the running turn keeps its start and clock.
    fn submit_text(&mut self, text: String) {
        if self.busy() {
            self.push(LogEntry::UserMessage { text: text.clone() });
        } else {
            if !text.trim_start().starts_with('/') {
                let first = text.lines().next().unwrap_or("").trim();
                self.request = Some(first.trim_end_matches(['.', '!']).to_string());
                self.activity = Activity::new(self.tick);
                self.awaiting_turn = true;
            }
            self.open_turn(text.clone());
        }
        self.outbox.push(Command::Submit { text });
    }

    /// Sends the queue as one turn, a message a line, once nothing holds it:
    /// no turn runs and no review is on screen (a review's comments start a
    /// follow-up turn first).
    fn send_queue(&mut self) {
        if self.queued.is_empty() || self.busy() || self.review().is_some() {
            return;
        }
        let text = std::mem::take(&mut self.queued).join("\n");
        self.submit_text(text);
    }

    /// Moves the queue into the field, ahead of what is typed there. False
    /// when nothing was queued.
    fn take_back_queue(&mut self) -> bool {
        if self.queued.is_empty() {
            return false;
        }
        let mut text = std::mem::take(&mut self.queued).join("\n");
        if !self.draft.is_empty() {
            text.push('\n');
            text.push_str(&self.draft.take());
        }
        self.draft.set(text);
        true
    }

    /// Sends `Cancel` once per turn and logs `notice` each time.
    fn stop(&mut self, notice: &str) {
        if !self.stopping {
            self.stopping = true;
            self.outbox.push(Command::Cancel);
        }
        self.push(LogEntry::Stopping {
            message: notice.into(),
        });
    }

    /// Records a `⌃C` press; whether it follows the last one within
    /// `DOUBLE_CTRL_C_TICKS`.
    fn ctrl_c_repeated(&mut self) -> bool {
        let repeat = self
            .last_ctrl_c
            .is_some_and(|t| self.tick.saturating_sub(t) <= DOUBLE_CTRL_C_TICKS);
        self.last_ctrl_c = Some(self.tick);
        repeat
    }

    /// `⌃C`: stop a running turn, else clear the draft, else quit; a second
    /// press within `DOUBLE_CTRL_C_TICKS` always quits.
    fn interrupt(&mut self) {
        if self.ctrl_c_repeated() {
            self.should_quit = true;
        } else if self.busy() {
            self.stop("Stopping. Press ⌃C again to leave Aldwin.");
        } else if !self.draft.is_empty() {
            self.draft.take();
        } else {
            self.should_quit = true;
        }
    }
}

/// Seams for `tests/render_snapshot.rs` and `examples/preview.rs` to seed
/// scenes without a provider.
#[cfg(feature = "test-util")]
impl App {
    /// Appends an entry as the live path would.
    pub fn seed(&mut self, entry: LogEntry) {
        self.push(entry);
    }

    /// The session facts, for a scene to set directly.
    pub fn status_mut(&mut self) -> &mut StatusInfo {
        &mut self.status
    }

    /// The open review, when one is on screen.
    pub fn review_for_tests(&mut self) -> Option<&mut Review> {
        self.review_mut()
    }

    /// Advances the clock `n` ticks, as `run.rs`'s ticker does.
    ///
    /// ```
    /// use aldwin_tui::App;
    ///
    /// let mut app = App::new("claude-sonnet-5".into());
    /// app.advance(10);
    /// ```
    pub fn advance(&mut self, n: u64) {
        for _ in 0..n {
            self.tick();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use aldwin_core::{
        CacheStats, ChangedFile, Changeset, Failure, FailureKind, StepId, StepOutcome, StopReason,
        ToolCall, ToolResult, TurnId, UsageStats,
    };
    use ratatui::crossterm::event::KeyEventState;

    use crate::log::Verb;

    const CHAT_ABOUT_THIS: &str = Question::CHAT_ABOUT_THIS;

    /// Rows of the menu, as aldwin-cli hands them in.
    pub(crate) fn commands() -> Vec<CommandChoice> {
        [
            ("resume", "Pick up an earlier conversation"),
            ("model", "Change the model"),
            ("quit", "Leave Aldwin"),
            ("exit", "Leave Aldwin"),
            ("clear", "Start a fresh conversation in this project"),
        ]
        .into_iter()
        .map(|(name, summary)| CommandChoice {
            name: name.into(),
            summary: summary.into(),
        })
        .collect()
    }

    fn app() -> App {
        App::new("claude-sonnet-5".into()).with_commands(commands())
    }

    fn press(code: KeyCode) -> KeyEvent {
        press_mod(code, KeyModifiers::NONE)
    }

    fn press_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle_key(press(KeyCode::Char(c)));
        }
    }

    fn catalogue() -> Vec<ProviderChoice> {
        vec![
            ProviderChoice {
                id: "anthropic".into(),
                purpose: "claude models".into(),
                models: vec![ModelChoice {
                    id: "claude-sonnet-5".into(),
                    purpose: "balanced".into(),
                    context: 1_000_000,
                }],
                account: None,
            },
            ProviderChoice {
                id: "openai".into(),
                purpose: "gpt models".into(),
                models: vec![ModelChoice {
                    id: "gpt-5".into(),
                    purpose: "balanced".into(),
                    context: 400_000,
                }],
                account: None,
            },
            ProviderChoice {
                id: "xai".into(),
                purpose: "grok models".into(),
                models: vec![ModelChoice {
                    id: "grok-4.7".into(),
                    purpose: "balanced".into(),
                    context: 500_000,
                }],
                account: Some("SuperGrok or X Premium".into()),
            },
        ]
    }

    /// A key-only provider is not listed.
    #[test]
    fn bare_connect_lists_the_accounts_and_submits_the_command() {
        let mut a = app().with_catalogue(catalogue(), Some("anthropic".into()));
        type_str(&mut a, "/connect");
        a.handle_key(press(KeyCode::Enter));
        let Mode::Question(asking) = &a.mode else {
            panic!("connection question")
        };
        assert!(matches!(asking.asker, Asker::Connection));
        assert_eq!(asking.question.options, ["xai"]);
        assert_eq!(asking.list.rows[0].detail, "SuperGrok or X Premium");

        a.handle_key(press(KeyCode::Char('1')));
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "/connect xai".into()
            }]
        );
    }

    /// The interceptor then reports rather than asks.
    #[test]
    fn connect_with_no_account_to_offer_is_forwarded() {
        let mut a = app().with_catalogue(catalogue()[..2].to_vec(), Some("anthropic".into()));
        type_str(&mut a, "/connect");
        a.handle_key(press(KeyCode::Enter));
        assert!(matches!(a.mode, Mode::Conversation));
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "/connect".into()
            }]
        );
    }

    #[test]
    fn typing_and_enter_submits_and_echoes() {
        let mut a = app();
        type_str(&mut a, "hello");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "hello".into()
            }]
        );
        assert_eq!(
            a.log[..],
            vec![LogEntry::UserMessage {
                text: "hello".into()
            }]
        );
        assert!(a.awaiting_turn);
    }

    #[test]
    fn a_slash_in_an_empty_field_opens_the_menu_and_elsewhere_types() {
        let mut a = app();
        type_str(&mut a, "a/b");
        assert!(matches!(a.mode, Mode::Conversation));
        assert_eq!(a.draft.text(), "a/b");
        a.draft.take();
        a.handle_key(press(KeyCode::Char('/')));
        let Mode::Commands(menu) = &a.mode else {
            panic!("the menu opens")
        };
        assert_eq!(menu.list.rows.len(), 5);
        assert_eq!(menu.list.rows[0].label, "/resume");
    }

    #[test]
    fn the_menu_filters_by_what_is_typed_and_backspace_past_the_slash_closes_it() {
        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        type_str(&mut a, "cl");
        let Mode::Commands(menu) = &a.mode else {
            panic!()
        };
        assert_eq!(menu.list.rows.len(), 1);
        assert_eq!(menu.list.rows[0].label, "/clear");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "/clear".into()
            }]
        );

        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        a.handle_key(press(KeyCode::Backspace));
        assert!(matches!(a.mode, Mode::Conversation));
    }

    #[test]
    fn quit_from_the_menu_submits_what_it_names() {
        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        a.handle_key(press(KeyCode::Char('3')));
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "/quit".into()
            }]
        );
    }

    /// `/e` must narrow to `/exit`, not close the menu.
    #[test]
    fn exit_is_offered_by_the_menu_as_well_as_quit() {
        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        a.handle_key(press(KeyCode::Char('e')));
        let Mode::Commands(menu) = &a.mode else {
            panic!("the menu stays open")
        };
        assert_eq!(menu.completion(), "xit");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "/exit".into()
            }]
        );
    }

    #[test]
    fn model_from_the_menu_asks_provider_then_model_and_submits_the_command() {
        let mut a = app().with_catalogue(catalogue(), Some("anthropic".into()));
        a.handle_key(press(KeyCode::Char('/')));
        a.handle_key(press(KeyCode::Char('2')));
        let Mode::Question(asking) = &a.mode else {
            panic!("provider question")
        };
        assert!(matches!(asking.asker, Asker::Provider { .. }));
        assert_eq!(asking.list.selected, 0, "opens on the current provider");
        a.handle_key(press(KeyCode::Char('2')));
        let Mode::Question(asking) = &a.mode else {
            panic!("model question")
        };
        assert!(matches!(&asking.asker, Asker::Model { provider, .. } if provider == "openai"));
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "/model openai/gpt-5".into()
            }]
        );
    }

    /// `/model` must be sent before the held message.
    #[test]
    fn with_no_model_the_first_message_waits_for_the_two_questions() {
        let mut a = App::new(String::new())
            .with_commands(commands())
            .with_catalogue(catalogue(), None);
        type_str(&mut a, "add rate limiting");
        a.handle_key(press(KeyCode::Enter));
        assert!(
            matches!(&a.mode, Mode::Question(q) if matches!(q.asker, Asker::Provider { then: Some(_) }))
        );
        assert!(a.outbox.is_empty());
        a.handle_key(press(KeyCode::Char('1')));
        a.handle_key(press(KeyCode::Char('1')));
        assert_eq!(
            a.outbox,
            vec![
                Command::Submit {
                    text: "/model anthropic/claude-sonnet-5".into()
                },
                Command::Submit {
                    text: "add rate limiting".into()
                }
            ]
        );
        assert!(
            matches!(&a.log[a.turn_start], LogEntry::UserMessage { text } if text == "add rate limiting"),
            "the held message opens its turn"
        );
        assert_eq!(a.request.as_deref(), Some("add rate limiting"));
    }

    /// Core takes a message behind a command, so only a message's turn
    /// holds the next one back.
    #[test]
    fn a_message_after_a_command_is_sent_not_queued() {
        let mut a = app();
        a.submit_text("/theme".into());
        send_line(&mut a, "add a limit");
        assert!(a.queued.is_empty());
        assert_eq!(
            a.outbox.last(),
            Some(&Command::Submit {
                text: "add a limit".into()
            })
        );
    }

    #[test]
    fn an_agent_question_takes_the_band_and_a_number_answers_it() {
        let mut a = app();
        a.apply_event(Event::QuestionAsked {
            call_id: "q1".into(),
            question: Question {
                question: "Limit anonymous?".into(),
                detail: "why".into(),
                options: vec!["Yes".into(), "No".into(), CHAT_ABOUT_THIS.into()],
            },
        });
        assert!(matches!(a.mode, Mode::Question(_)));
        type_str(&mut a, "x");
        assert!(
            a.draft.is_empty(),
            "no typing ahead while a question is open"
        );
        a.handle_key(press(KeyCode::Char('2')));
        assert_eq!(
            a.outbox,
            vec![Command::Answer {
                call_id: "q1".into(),
                answer: Answer::Chose { index: 1 }
            }]
        );
        assert!(matches!(a.mode, Mode::Conversation));
    }

    #[test]
    fn chat_about_this_makes_the_next_message_the_answer() {
        let mut a = app();
        a.apply_event(Event::QuestionAsked {
            call_id: "q1".into(),
            question: Question {
                question: "Q?".into(),
                detail: String::new(),
                options: vec!["Yes".into(), CHAT_ABOUT_THIS.into()],
            },
        });
        a.handle_key(press(KeyCode::Esc));
        assert!(a.answering.is_some());
        type_str(&mut a, "only for keyed requests");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(
            a.outbox,
            vec![Command::Answer {
                call_id: "q1".into(),
                answer: Answer::Said {
                    text: "only for keyed requests".into()
                }
            }]
        );
        assert!(a.answering.is_none());
    }

    /// Regression: `finish_call` stopped at the first `Work` entry, so the
    /// answer never reached its question row.
    #[test]
    fn an_answered_question_gets_its_answer_even_after_other_work() {
        let mut a = app();
        a.log.push(LogEntry::UserMessage { text: "go".into() });
        a.log.push(LogEntry::Work {
            items: vec![],
            open: false,
        });
        a.apply_event(Event::QuestionAsked {
            call_id: "q1".into(),
            question: Question {
                question: "Q?".into(),
                detail: String::new(),
                options: vec!["Yes".into(), CHAT_ABOUT_THIS.into()],
            },
        });
        a.handle_key(press(KeyCode::Char('1')));
        a.apply_event(Event::ToolCompleted {
            turn_id: TurnId(1),
            step_id: StepId(1),
            result: ToolResult {
                call_id: "q1".into(),
                content: Answer::Chose { index: 0 }
                    .to_result(&["Yes".into()])
                    .unwrap(),
                is_error: false,
            },
        });
        assert!(
            matches!(a.log.last(), Some(LogEntry::Question { answer: Some(ans), .. }) if ans == "Yes"),
            "{:?}",
            a.log
        );
    }

    #[test]
    fn a_review_over_nothing_is_answered_rather_than_opened() {
        let mut a = app();
        a.apply_event(Event::ReviewRequested {
            review_id: "r".into(),
            changeset: Changeset::default(),
        });
        assert!(matches!(a.mode, Mode::Conversation));
        assert_eq!(
            a.outbox,
            vec![Command::ReviewDecision {
                review_id: "r".into(),
                decision: ReviewDecision::Discard
            }]
        );
    }

    #[test]
    fn tool_calls_become_a_work_disclosure_with_facts() {
        let mut a = app();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        let call = ToolCall {
            id: "c1".into(),
            name: "read".into(),
            input: serde_json::json!({"path": "src/x.rs"}),
        };
        a.apply_event(Event::ToolUseRequested {
            turn_id: TurnId(1),
            step_id: StepId(1),
            call,
        });
        a.apply_event(Event::ToolDispatched {
            turn_id: TurnId(1),
            step_id: StepId(1),
            call_id: "c1".into(),
        });
        a.apply_event(Event::ToolCompleted {
            turn_id: TurnId(1),
            step_id: StepId(1),
            result: ToolResult {
                call_id: "c1".into(),
                content: "a\nb\n".into(),
                is_error: false,
            },
        });
        let Some(LogEntry::Work { items, open }) = a.log.last() else {
            panic!("{:?}", a.log)
        };
        assert!(!open);
        assert_eq!(items[0].verb, Verb::Read);
        assert_eq!(items[0].target, "src/x.rs");
        assert_eq!(items[0].fact.as_deref(), Some("2 lines"));
    }

    #[test]
    fn a_result_finishes_its_call_after_a_command_typed_mid_turn() {
        let mut a = app();
        a.submit_text("go".into());
        a.apply_event(Event::ToolUseRequested {
            turn_id: TurnId(1),
            step_id: StepId(1),
            call: ToolCall {
                id: "c1".into(),
                name: "read".into(),
                input: serde_json::json!({"path": "src/x.rs"}),
            },
        });
        a.apply_event(Event::ToolDispatched {
            turn_id: TurnId(1),
            step_id: StepId(1),
            call_id: "c1".into(),
        });
        a.submit_text("/help".into());
        a.apply_event(Event::ToolCompleted {
            turn_id: TurnId(1),
            step_id: StepId(1),
            result: ToolResult {
                call_id: "c1".into(),
                content: "a\n".into(),
                is_error: false,
            },
        });
        let work = a.log.iter().find_map(|e| match e {
            LogEntry::Work { items, .. } => Some(items),
            _ => None,
        });
        assert_eq!(work.unwrap()[0].fact.as_deref(), Some("1 line"));
    }

    #[test]
    fn space_on_an_empty_field_toggles_the_details_and_otherwise_types() {
        let mut a = app();
        a.log.push(LogEntry::UserMessage { text: "go".into() });
        a.log.push(LogEntry::Work {
            items: vec![],
            open: false,
        });
        a.handle_key(press(KeyCode::Char(' ')));
        assert!(matches!(
            a.log.last(),
            Some(LogEntry::Work { open: true, .. })
        ));
        assert!(a.draft.is_empty());
        type_str(&mut a, "a ");
        assert_eq!(a.draft.text(), "a ");
    }

    #[test]
    fn the_plan_is_one_entry_per_turn_replaced_in_place() {
        let mut a = app();
        a.log.push(LogEntry::UserMessage { text: "go".into() });
        let step = |t: &str, s| PlanStep {
            text: t.into(),
            state: s,
        };
        a.apply_event(Event::PlanUpdated {
            turn_id: TurnId(1),
            steps: vec![step("Count", StepState::Running)],
        });
        a.apply_event(Event::PlanUpdated {
            turn_id: TurnId(1),
            steps: vec![
                step("Count", StepState::Done),
                step("Check", StepState::Running),
            ],
        });
        assert_eq!(
            a.log
                .iter()
                .filter(|e| matches!(e, LogEntry::Plan { .. }))
                .count(),
            1
        );
        let Some(LogEntry::Plan { steps }) = a.log.last() else {
            panic!()
        };
        assert_eq!(steps.len(), 2);
    }

    fn plan_states(a: &App) -> Vec<StepState> {
        let plans: Vec<_> = a
            .log
            .iter()
            .filter_map(|e| {
                if let LogEntry::Plan { steps } = e {
                    Some(steps)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(plans.len(), 1, "one plan entry per turn: {:?}", a.log);
        plans[0].iter().map(|s| s.state).collect()
    }

    /// Amber means running; applies to finished and stopped turns alike.
    #[test]
    fn a_step_still_running_when_the_turn_ends_goes_back_to_pending() {
        use StepState::{Done, Pending, Running};
        for reason in [TurnEndReason::EndTurn, TurnEndReason::Cancelled] {
            let mut a = app();
            a.submit_text("go".into());
            a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            let step = |t: &str, s| PlanStep {
                text: t.into(),
                state: s,
            };
            a.apply_event(Event::PlanUpdated {
                turn_id: TurnId(1),
                steps: vec![
                    step("Count", Done),
                    step("Check", Running),
                    step("Ship", Pending),
                ],
            });
            a.apply_event(Event::TurnEnded {
                turn_id: TurnId(1),
                reason,
            });
            assert_eq!(plan_states(&a), vec![Done, Pending, Pending]);
        }
    }

    /// The plan is still replaced in place after the answer, and settled at
    /// the turn's end.
    #[test]
    fn a_chat_about_this_answer_does_not_split_the_turn() {
        use StepState::{Done, Pending, Running};
        let mut a = app();
        a.submit_text("go".into());
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        let step = |t: &str, s| PlanStep {
            text: t.into(),
            state: s,
        };
        a.apply_event(Event::PlanUpdated {
            turn_id: TurnId(1),
            steps: vec![step("Count", Running)],
        });
        a.apply_event(Event::QuestionAsked {
            call_id: "q1".into(),
            question: Question {
                question: "Q?".into(),
                detail: String::new(),
                options: vec!["Yes".into(), CHAT_ABOUT_THIS.into()],
            },
        });
        a.handle_key(press(KeyCode::Char('2')));
        for c in "it depends".chars() {
            a.handle_key(press(KeyCode::Char(c)));
        }
        a.handle_key(press(KeyCode::Enter));
        assert!(
            matches!(a.log.last(), Some(LogEntry::UserMessage { .. })),
            "the answer is in the log as a message"
        );
        a.apply_event(Event::PlanUpdated {
            turn_id: TurnId(1),
            steps: vec![step("Count", Done), step("Check", Running)],
        });
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::EndTurn,
        });
        assert_eq!(plan_states(&a), vec![Done, Pending]);
    }

    /// ADR 0010 §3.
    #[test]
    fn the_mouse_is_wanted_while_a_review_is_open_and_goes_to_it() {
        let mut a = app();
        assert!(!a.wants_mouse());
        let after = (1..=60).map(|i| format!("line {i}\n")).collect::<String>();
        a.apply_event(Event::ReviewRequested {
            review_id: "r".into(),
            changeset: Changeset {
                files: vec![ChangedFile {
                    path: "f.rs".into(),
                    before: None,
                    after,
                }],
            },
        });
        assert!(a.wants_mouse());
        let transcript = a.scroll.offset;
        a.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(
            a.review().unwrap().scroll,
            WHEEL_ROWS,
            "the wheel scrolls the diff"
        );
        assert_eq!(
            a.scroll.offset, transcript,
            "and not the conversation behind it"
        );
        a.apply_event(Event::ReviewClosed {
            outcome: ReviewOutcome::Saved {
                files: vec!["f.rs".into()],
                comments_resolved: 0,
            },
        });
        assert!(
            !a.wants_mouse(),
            "the conversation gives the mouse back to the terminal"
        );
    }

    #[test]
    fn a_review_takes_the_screen_and_its_decision_is_sent_with_its_id() {
        let mut a = app();
        let changeset = Changeset {
            files: vec![ChangedFile {
                path: "f.rs".into(),
                before: Some("x\n".into()),
                after: "y\n".into(),
            }],
        };
        a.apply_event(Event::ReviewRequested {
            review_id: "review-1".into(),
            changeset,
        });
        assert!(matches!(a.mode, Mode::Review(_)));
        a.review_mut().unwrap().mark_read();
        a.handle_key(press_mod(KeyCode::Enter, KeyModifiers::CONTROL));
        assert_eq!(
            a.outbox,
            vec![Command::ReviewDecision {
                review_id: "review-1".into(),
                decision: ReviewDecision::Approve
            }]
        );
        a.apply_event(Event::ReviewClosed {
            outcome: ReviewOutcome::Saved {
                files: vec!["f.rs".into()],
                comments_resolved: 0,
            },
        });
        assert!(matches!(a.mode, Mode::Conversation));
        assert!(matches!(a.log.last(), Some(LogEntry::Review { .. })));
    }

    /// A review over `f.rs`, opened as the turn's end would.
    fn open_review(a: &mut App, after: &str) {
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        a.apply_event(Event::ReviewRequested {
            review_id: "review-1".into(),
            changeset: Changeset {
                files: vec![ChangedFile {
                    path: "f.rs".into(),
                    before: Some("x\n".into()),
                    after: after.into(),
                }],
            },
        });
    }

    /// Sends `text` as a general comment and lets core answer.
    fn send_comment(a: &mut App, text: &str) {
        type_str(a, text);
        a.handle_key(press_mod(KeyCode::Enter, KeyModifiers::CONTROL));
        assert!(matches!(
            a.outbox.pop(),
            Some(Command::ReviewDecision {
                decision: ReviewDecision::Comment { .. },
                ..
            })
        ));
        a.apply_event(Event::ReviewClosed {
            outcome: ReviewOutcome::Commented { comments: 1 },
        });
    }

    /// Regression: sending comments closed the review, and the next one
    /// opened fresh.
    #[test]
    fn comments_keep_the_review_open_until_the_next_changeset_replaces_it() {
        let mut a = app();
        a.submit_text("Add a limit.".into());
        open_review(&mut a, "y\n");
        assert_eq!(a.review().unwrap().title, "Add a limit");
        a.review_mut().unwrap().mark_read();
        send_comment(&mut a, "rename it");
        assert!(a.review().is_some_and(Review::waiting));
        assert!(a.wants_mouse());
        assert!(
            !a.log.iter().any(|e| matches!(e, LogEntry::Review { .. })),
            "no row for the comments"
        );

        // The finished turn starts the follow-up turn (ADR 0009 §4).
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::EndTurn,
        });
        a.apply_event(Event::FollowUp {
            turn_id: TurnId(2),
            text: "On f.rs, line 1:\nrename it".into(),
        });
        a.apply_event(Event::TurnStarted { turn_id: TurnId(2) });
        assert!(a.review().is_some_and(Review::waiting), "still waiting");

        a.apply_event(Event::ReviewRequested {
            review_id: "review-2".into(),
            changeset: Changeset {
                files: vec![ChangedFile {
                    path: "f.rs".into(),
                    before: Some("x\n".into()),
                    after: "z\n".into(),
                }],
            },
        });
        let review = a.review().expect("the review is still on screen");
        assert!(!review.waiting());
        assert_eq!(review.review_id, "review-2");
        assert_eq!(
            review.title, "Add a limit",
            "the echoed comments are not the title"
        );
        assert!(!review.all_read(), "the changed file is read again");
        a.review_mut().unwrap().mark_read();
        a.handle_key(press_mod(KeyCode::Enter, KeyModifiers::CONTROL));
        assert!(matches!(
            a.outbox.last(),
            Some(Command::ReviewDecision {
                review_id,
                decision: ReviewDecision::Approve
            }) if review_id == "review-2"
        ));
        a.apply_event(Event::ReviewClosed {
            outcome: ReviewOutcome::Saved {
                files: vec!["f.rs".into()],
                comments_resolved: 1,
            },
        });
        assert!(matches!(a.mode, Mode::Conversation));
    }

    fn think(a: &mut App, fragments: &[&str]) {
        let (turn_id, step_id) = (TurnId(1), StepId(1));
        a.apply_event(Event::ThinkingStart { turn_id, step_id });
        for text in fragments {
            a.apply_event(Event::ThinkingDelta {
                turn_id,
                step_id,
                text: (*text).into(),
            });
        }
    }

    /// ADR 0015: a thinking block is one entry where it happened, filled as
    /// it streams and timed once it ends; Space opens it with the work.
    #[test]
    fn a_thought_streams_into_one_entry_and_says_how_long_it_took() {
        let mut a = app();
        a.submit_text("Add a limit.".into());
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        think(&mut a, &["Weighing ", "it up."]);
        assert_eq!(
            a.log.last(),
            Some(&LogEntry::Thinking {
                text: "Weighing it up.".into(),
                took: Took::Running,
                open: false,
            })
        );
        a.apply_event(Event::ThinkingEnd {
            turn_id: TurnId(1),
            step_id: StepId(1),
            seconds: Some(12),
        });
        a.apply_event(Event::TextDelta {
            turn_id: TurnId(1),
            step_id: StepId(1),
            text: "Adding it.".into(),
        });
        assert!(matches!(
            &a.log[a.log.len() - 2..],
            [
                LogEntry::Thinking {
                    took: Took::Seconds(12),
                    ..
                },
                LogEntry::AssistantText { .. }
            ]
        ));
        a.handle_key(press(KeyCode::Char(' ')));
        assert!(
            matches!(
                &a.log[a.log.len() - 2],
                LogEntry::Thinking { open: true, .. }
            ),
            "Space opens it"
        );
    }

    #[test]
    fn a_thought_the_turn_cut_off_no_longer_says_thinking() {
        let mut a = app();
        a.submit_text("Add a limit.".into());
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        think(&mut a, &["Weighing"]);
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::Cancelled,
        });
        assert!(a.log.iter().any(|e| matches!(
            e,
            LogEntry::Thinking {
                took: Took::Unknown,
                ..
            }
        )));
    }

    #[test]
    fn a_resumed_thought_is_drawn_as_it_was_and_a_redacted_one_is_not() {
        let mut a = app();
        let thought = |seconds| LogRecord::Thinking {
            turn_id: TurnId(1),
            step_id: StepId(1),
            text: "hm".into(),
            signature: String::new(),
            seconds,
        };
        a.apply_event(Event::HistoryLoaded {
            records: vec![
                LogRecord::TurnStarted { turn_id: TurnId(1) },
                LogRecord::UserMessage {
                    turn_id: TurnId(1),
                    text: "go".into(),
                },
                thought(Some(7)),
                thought(None),
                LogRecord::RedactedThinking {
                    turn_id: TurnId(1),
                    step_id: StepId(1),
                    data: "opaque".into(),
                },
            ],
        });
        let took: Vec<Took> = a
            .log
            .iter()
            .filter_map(|e| match e {
                LogEntry::Thinking { took, .. } => Some(*took),
                _ => None,
            })
            .collect();
        assert_eq!(took, vec![Took::Seconds(7), Took::Unknown]);
    }

    /// A waiting review inside its follow-up turn, and the agent asking.
    fn ask_over_a_waiting_review(a: &mut App) {
        a.submit_text("Add a limit.".into());
        open_review(a, "y\n");
        a.handle_key(press(KeyCode::Char('?')));
        send_comment(a, "per key?");
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::EndTurn,
        });
        a.apply_event(Event::FollowUp {
            turn_id: TurnId(2),
            text: "per key?".into(),
        });
        a.apply_event(Event::TurnStarted { turn_id: TurnId(2) });
        a.apply_event(Event::QuestionAsked {
            call_id: "q1".into(),
            question: Question {
                question: "Limit anonymous requests too?".into(),
                detail: "why".into(),
                options: vec!["Yes".into(), "No".into(), CHAT_ABOUT_THIS.into()],
            },
        });
        a.outbox.clear();
    }

    /// Regression: the question replaced the waiting review, and the next
    /// changeset opened a fresh one with nothing carried.
    #[test]
    fn an_agent_question_is_asked_over_a_waiting_review_and_leaves_it_waiting() {
        let mut a = app();
        ask_over_a_waiting_review(&mut a);
        assert!(matches!(a.mode, Mode::Question(_)));
        assert!(
            a.review().is_some_and(Review::waiting),
            "the review is under it"
        );
        assert!(a.wants_mouse());

        a.handle_key(press(KeyCode::Char('2')));
        assert_eq!(
            a.outbox,
            vec![Command::Answer {
                call_id: "q1".into(),
                answer: Answer::Chose { index: 1 }
            }]
        );
        assert!(
            matches!(&a.mode, Mode::Review(r) if r.waiting()),
            "the review is back, still waiting"
        );

        a.apply_event(Event::ReviewRequested {
            review_id: "review-2".into(),
            changeset: Changeset {
                files: vec![ChangedFile {
                    path: "f.rs".into(),
                    before: Some("x\n".into()),
                    after: "z\n".into(),
                }],
            },
        });
        let review = a.review().expect("the next round replaces it in place");
        assert_eq!(review.review_id, "review-2");
        assert!(review.keys_shown, "carried from the review it replaced");
    }

    #[test]
    fn chat_about_this_over_a_review_answers_in_its_field_and_esc_goes_back() {
        let mut a = app();
        ask_over_a_waiting_review(&mut a);
        a.handle_key(press(KeyCode::Esc));
        assert!(a.answering.is_some());
        assert!(
            matches!(a.mode, Mode::Review(_)),
            "the review holds the screen"
        );
        type_str(&mut a, "only ");
        a.handle_key(press(KeyCode::Esc));
        assert!(
            matches!(&a.mode, Mode::Question(q) if q.covers.is_some()),
            "back to the options, over the review"
        );
        a.paste("dropped");
        a.handle_key(press(KeyCode::Esc));
        a.paste("keyed\nones");
        a.handle_key(press(KeyCode::Up));
        type_str(&mut a, "!");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(
            a.outbox,
            vec![Command::Answer {
                call_id: "q1".into(),
                answer: Answer::Said {
                    text: "only! keyed\nones".into()
                }
            }],
            "a paste and the arrows work as in the conversation's field"
        );
        assert!(a.answering.is_none());
        assert!(matches!(&a.mode, Mode::Review(r) if r.waiting()));
    }

    /// Comments sent before a `run` wait within the turn; a turn that ends
    /// on the question keeps the review the question covered.
    #[test]
    fn a_turn_ending_on_the_question_puts_the_review_back() {
        let mut a = app();
        open_review(&mut a, "y\n");
        send_comment(&mut a, "rename it");
        a.apply_event(Event::QuestionAsked {
            call_id: "q1".into(),
            question: Question {
                question: "Q?".into(),
                detail: String::new(),
                options: vec!["Yes".into(), CHAT_ABOUT_THIS.into()],
            },
        });
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::EndTurn,
        });
        assert!(matches!(&a.mode, Mode::Review(r) if r.waiting()));
    }

    #[test]
    fn esc_stops_the_turn_a_waiting_review_is_inside_and_the_stop_closes_it() {
        let mut a = app();
        open_review(&mut a, "y\n");
        send_comment(&mut a, "rename it");
        a.handle_key(press(KeyCode::Esc));
        assert_eq!(a.outbox, vec![Command::Cancel]);
        assert!(a.review().is_some(), "open until core answers");
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::Cancelled,
        });
        assert!(matches!(a.mode, Mode::Conversation));
        assert!(!a.wants_mouse());
    }

    /// Regression: a follow-up that staged nothing new left its reply
    /// hidden behind the review.
    #[test]
    fn a_follow_up_turn_that_ends_without_a_changeset_closes_the_review() {
        let mut a = app();
        open_review(&mut a, "y\n");
        send_comment(&mut a, "why this?");
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::EndTurn,
        });
        a.apply_event(Event::FollowUp {
            turn_id: TurnId(2),
            text: "why this?".into(),
        });
        a.apply_event(Event::TurnStarted { turn_id: TurnId(2) });
        assert!(a.review().is_some(), "waiting while the follow-up runs");
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(2),
            reason: TurnEndReason::EndTurn,
        });
        assert!(
            matches!(a.mode, Mode::Conversation),
            "the reply is on screen"
        );
    }

    /// Regression: an echoed follow-up could stand last and title a review.
    #[test]
    fn a_review_is_titled_by_what_was_typed_never_by_an_echo() {
        let mut a = app();
        a.submit_text("Add a limit.".into());
        a.submit_text("/theme light".into());
        a.apply_event(Event::FollowUp {
            turn_id: TurnId(2),
            text: "I discarded the staged changes; nothing was written.".into(),
        });
        open_review(&mut a, "y\n");
        assert_eq!(a.review().unwrap().title, "Add a limit");
    }

    /// Before a `run`, comments come back within the turn, and the review
    /// of what is still staged replaces the waiting one in that turn.
    #[test]
    fn comments_sent_before_a_run_are_answered_within_the_turn() {
        let mut a = app();
        a.submit_text("Add a limit.".into());
        open_review(&mut a, "y\n");
        send_comment(&mut a, "rename it");
        a.apply_event(Event::ReviewRequested {
            review_id: "review-2".into(),
            changeset: Changeset {
                files: vec![ChangedFile {
                    path: "f.rs".into(),
                    before: Some("x\n".into()),
                    after: "z\n".into(),
                }],
            },
        });
        let review = a.review().expect("replaced in place");
        assert!(!review.waiting());
        assert_eq!(review.title, "Add a limit");
    }

    /// A waiting review with no turn running is never a dead end.
    #[test]
    fn esc_leaves_a_waiting_review_when_no_turn_is_running() {
        let mut a = app();
        open_review(&mut a, "y\n");
        send_comment(&mut a, "rename it");
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::EndTurn,
        });
        assert!(a.review().is_some_and(Review::waiting));
        a.handle_key(press(KeyCode::Esc));
        assert!(a.outbox.is_empty(), "nothing to stop");
        assert!(matches!(a.mode, Mode::Conversation));
    }

    #[test]
    fn a_failed_turn_closes_a_waiting_review_too() {
        let mut a = app();
        open_review(&mut a, "y\n");
        send_comment(&mut a, "rename it");
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::Error(Failure::other("boom")),
        });
        assert!(matches!(a.mode, Mode::Conversation));
    }

    #[test]
    fn typing_in_a_review_goes_to_its_field_and_enter_sends_it_as_a_comment() {
        let mut a = app();
        let changeset = Changeset {
            files: vec![ChangedFile {
                path: "f.rs".into(),
                before: Some("x\n".into()),
                after: "y\n".into(),
            }],
        };
        a.apply_event(Event::ReviewRequested {
            review_id: "r".into(),
            changeset,
        });
        type_str(&mut a, "rename it");
        assert_eq!(a.draft.text(), "rename it");
        a.handle_key(press(KeyCode::Enter));
        let Some(Command::ReviewDecision {
            decision: ReviewDecision::Comment { comments },
            ..
        }) = a.outbox.last()
        else {
            panic!("{:?}", a.outbox)
        };
        assert_eq!(comments[0].text, "rename it");
        assert!(a.draft.is_empty());
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
                usage: UsageStats {
                    input_tokens: 300_000,
                    output_tokens: 10,
                },
                cache: CacheStats {
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 110_000,
                },
            },
        });
        assert_eq!(a.status.context_percent(), Some(41));
    }

    #[test]
    fn a_failed_turn_is_a_sentence_with_its_detail_folded() {
        let mut a = app();
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::Error(Failure::other("boom\nstack")),
        });
        assert!(
            matches!(&a.log[0], LogEntry::Failure { message, detail: Some(d), open: false } if message == "The turn stopped before it finished. The detail says why." && d == "boom\nstack")
        );
        assert_eq!(a.log[1], LogEntry::TurnBreak);

        // A provider's body is a detail, not a sentence.
        let body = r#"provider error 400: {"error":{"message":"the request was malformed"}}"#;
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(2),
            reason: TurnEndReason::Error(Failure {
                kind: FailureKind::Provider { status: 400 },
                message: body.into(),
            }),
        });
        assert!(
            matches!(&a.log[2], LogEntry::Failure { message, detail: Some(d), .. } if message == "The provider turned the request down. The detail says why." && d == body)
        );

        // A turn Aldwin could not send is its own sentence, with nothing to
        // disclose.
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(3),
            reason: TurnEndReason::Error(Failure {
                kind: FailureKind::NotSent,
                message: "No model is configured yet. Pick one with /model.".into(),
            }),
        });
        assert!(
            matches!(&a.log[4], LogEntry::Failure { message, detail: None, .. } if message == "No model is configured yet. Pick one with /model.")
        );
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

    fn send_line(a: &mut App, text: &str) {
        type_str(a, text);
        a.handle_key(press(KeyCode::Enter));
    }

    fn end_turn(a: &mut App, id: u64, reason: TurnEndReason) {
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(id),
            reason,
        });
    }

    #[test]
    fn messages_sent_mid_turn_go_as_one_turn_when_it_finishes() {
        let mut a = app();
        send_line(&mut a, "add a limit");
        a.outbox.clear();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        send_line(&mut a, "also a header");
        send_line(&mut a, "use 429");
        assert!(a.outbox.is_empty(), "nothing reaches core mid-turn");
        assert_eq!(a.queued, ["also a header", "use 429"]);
        let echoes = |a: &App| {
            a.log
                .iter()
                .filter(|e| matches!(e, LogEntry::UserMessage { .. }))
                .count()
        };
        assert_eq!(echoes(&a), 1, "the queue is not echoed yet");

        end_turn(&mut a, 1, TurnEndReason::EndTurn);
        let text = "also a header\nuse 429".to_string();
        assert_eq!(a.outbox, vec![Command::Submit { text: text.clone() }]);
        assert!(a.queued.is_empty() && a.busy());
        assert_eq!(echoes(&a), 2, "one echo for the whole queue");
        assert!(matches!(&a.log[a.turn_start], LogEntry::UserMessage { text: t } if *t == text));
    }

    #[test]
    fn esc_takes_the_queue_back_before_it_stops() {
        let mut a = app();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        send_line(&mut a, "also a header");
        send_line(&mut a, "use 429");
        type_str(&mut a, "and log");
        a.handle_key(press(KeyCode::Esc));
        assert_eq!(a.draft.text(), "also a header\nuse 429\nand log");
        assert_eq!(a.draft.cursor(), a.draft.text().chars().count());
        assert!(a.queued.is_empty() && a.outbox.is_empty());
        a.handle_key(press(KeyCode::Esc));
        assert_eq!(a.outbox, vec![Command::Cancel]);
    }

    #[test]
    fn a_stopped_or_failed_turn_puts_the_queue_back_in_the_field() {
        for reason in [
            TurnEndReason::Cancelled,
            TurnEndReason::Error(Failure::other("boom")),
        ] {
            let mut a = app();
            a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
            send_line(&mut a, "use 429");
            a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
            end_turn(&mut a, 1, reason);
            assert_eq!(a.outbox, vec![Command::Cancel], "nothing is sent");
            assert_eq!(a.draft.text(), "use 429");
            assert!(a.queued.is_empty());
        }
    }

    /// Regression: `⌃C` cancelled the turn and left the undecided review
    /// on screen with no turn behind it.
    #[test]
    fn ctrl_c_over_an_undecided_review_stops_nothing_and_quits_on_repeat() {
        let mut a = app();
        open_review(&mut a, "y\n");
        a.queued.push("use 429".into());
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.outbox.is_empty() && !a.should_quit);
        assert!(a.review().is_some());
        assert_eq!(a.queued, ["use 429"]);
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.should_quit);
    }

    #[test]
    fn ctrl_c_over_a_waiting_review_stops_nothing() {
        let mut a = app();
        open_review(&mut a, "y\n");
        send_comment(&mut a, "rename it");
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.outbox.is_empty() && !a.should_quit);
        assert!(a.review().is_some_and(Review::waiting));
    }

    #[test]
    fn the_queue_waits_for_the_turn_a_reviews_comments_start() {
        let mut a = app();
        open_review(&mut a, "y\n");
        send_comment(&mut a, "rename it");
        a.queued.push("use 429".into());
        end_turn(&mut a, 1, TurnEndReason::EndTurn);
        assert!(a.outbox.is_empty(), "the review's comments go first");
        a.apply_event(Event::FollowUp {
            turn_id: TurnId(2),
            text: "On f.rs, line 1:\nrename it".into(),
        });
        a.apply_event(Event::TurnStarted { turn_id: TurnId(2) });
        end_turn(&mut a, 2, TurnEndReason::EndTurn);
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "use 429".into()
            }]
        );
    }

    /// Regression: a command sent mid-turn became the turn's start, so the
    /// next `plan` update drew a second plan, and it restarted the clock.
    #[test]
    fn a_command_mid_turn_is_sent_at_once_and_the_turn_keeps_its_start() {
        let mut a = app();
        send_line(&mut a, "add a limit");
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        a.outbox.clear();
        a.advance(50);
        let start = a.turn_start;
        a.submit_text("/theme".into());
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "/theme".into()
            }]
        );
        assert!(a.queued.is_empty());
        assert_eq!(a.turn_start, start);
        assert_eq!(
            a.activity.line(a.tick).seconds,
            5,
            "the turn's clock runs on"
        );
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
                LogRecord::UserMessage {
                    turn_id: TurnId(1),
                    text: "hi".into(),
                },
                LogRecord::ToolUse {
                    turn_id: TurnId(1),
                    step_id: StepId(1),
                    call: ToolCall {
                        id: "c1".into(),
                        name: "run".into(),
                        input: serde_json::json!({"command": "ls"}),
                    },
                },
                LogRecord::ToolResult {
                    turn_id: TurnId(1),
                    step_id: StepId(1),
                    result: ToolResult {
                        call_id: "c1".into(),
                        content: "ok".into(),
                        is_error: false,
                    },
                },
                LogRecord::AssistantMessage {
                    turn_id: TurnId(1),
                    step_id: StepId(1),
                    text: "Done.".into(),
                },
                LogRecord::TurnEnded {
                    turn_id: TurnId(1),
                    reason: TurnEndReason::EndTurn,
                },
            ],
        });
        assert!(
            matches!(&a.log[1], LogEntry::Work { items, .. } if items[0].fact.as_deref() == Some("ok"))
        );
        assert_eq!(a.log.last(), Some(&LogEntry::TurnBreak));
    }

    fn ask(a: &mut App) {
        a.apply_event(Event::QuestionAsked {
            call_id: "q1".into(),
            question: Question {
                question: "Q?".into(),
                detail: String::new(),
                options: vec!["Yes".into(), CHAT_ABOUT_THIS.into()],
            },
        });
    }

    /// Regression: `esc` shared `⌃C`'s path, so a second `esc` within two
    /// seconds quit mid-turn.
    #[test]
    fn escape_only_ever_stops_and_asks_once() {
        let mut a = app();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        a.handle_key(press(KeyCode::Esc));
        a.handle_key(press(KeyCode::Esc));
        assert!(!a.should_quit, "a second esc does not leave Aldwin");
        assert_eq!(a.outbox, vec![Command::Cancel], "and asks for one stop");
        assert!(
            matches!(a.log.last(), Some(LogEntry::Stopping { message }) if message == "Stopping."),
            "the notice names no key that was not pressed: {:?}",
            a.log
        );
    }

    /// Regression: "Stopping." then "Stopped." was two sentences for one
    /// stop.
    #[test]
    fn a_stopped_turn_leaves_one_sentence() {
        let mut a = app();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        a.handle_key(press(KeyCode::Esc));
        a.handle_key(press(KeyCode::Esc));
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::Cancelled,
        });
        let said: Vec<&str> = a
            .log
            .iter()
            .filter_map(|e| match e {
                LogEntry::Notice { message }
                | LogEntry::Stopping { message }
                | LogEntry::Failure { message, .. } => Some(message.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(said, ["Stopped."]);
    }

    #[test]
    fn ctrl_c_says_its_own_key_is_the_way_out() {
        let mut a = app();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(
            matches!(a.log.last(), Some(LogEntry::Stopping { message }) if message.contains("⌃C again"))
        );
    }

    /// Regression: the list took `⌃C` as close, which for the agent's
    /// question meant "Chat about this".
    #[test]
    fn ctrl_c_during_an_agent_question_stops_the_turn() {
        let mut a = app();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        ask(&mut a);
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(a.outbox, vec![Command::Cancel]);
        assert!(a.answering.is_none() && !a.should_quit);
        a.apply_event(Event::TurnEnded {
            turn_id: TurnId(1),
            reason: TurnEndReason::Cancelled,
        });
        assert!(
            matches!(a.mode, Mode::Conversation),
            "the question goes with its turn"
        );
    }

    /// ADR 0012: an account provider runs without a key. Regression: the
    /// question said every provider needs one.
    #[test]
    fn the_provider_question_does_not_demand_a_key_of_every_provider() {
        let mut a = App::new(String::new()).with_catalogue(catalogue(), None);
        a.open_provider_question(None);
        let Mode::Question(q) = &a.mode else {
            panic!("the provider question is open")
        };
        assert!(!q.question.detail.contains("needs its key"));
        assert!(q.question.detail.contains("/connect"));
    }

    /// Regression: closing the provider question dropped the held message.
    #[test]
    fn closing_the_provider_question_puts_the_held_message_back() {
        for close in [
            press(KeyCode::Esc),
            press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut a = App::new(String::new())
                .with_commands(commands())
                .with_catalogue(catalogue(), None);
            type_str(&mut a, "add rate limiting");
            a.handle_key(press(KeyCode::Enter));
            a.handle_key(close);
            assert!(matches!(a.mode, Mode::Conversation));
            assert_eq!(a.draft.text(), "add rate limiting");
            assert!(a.outbox.is_empty() && !a.should_quit);
        }
    }

    /// Regression: after "Chat about this" the question vanished and `esc`
    /// stopped the whole turn.
    #[test]
    fn escape_while_answering_in_words_goes_back_to_the_options() {
        let mut a = app();
        a.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        ask(&mut a);
        a.handle_key(press(KeyCode::Char('2')));
        assert!(a.answering.is_some());
        type_str(&mut a, "it depends");
        a.handle_key(press(KeyCode::Esc));
        assert!(
            matches!(&a.mode, Mode::Question(q) if matches!(q.asker, Asker::Agent { .. })),
            "the options again"
        );
        assert!(a.outbox.is_empty(), "the turn was not stopped");
        assert_eq!(a.draft.text(), "it depends", "and the words are kept");
    }

    /// Regression: the menu swallowed every key but a letter, so `/theme
    /// light` could not be typed.
    #[test]
    fn a_command_the_menu_does_not_offer_is_typed_in_the_field() {
        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        type_str(&mut a, "theme light");
        assert!(matches!(a.mode, Mode::Conversation));
        assert_eq!(a.draft.text(), "/theme light");

        let mut a = app();
        a.handle_key(press(KeyCode::Char('/')));
        type_str(&mut a, "model");
        assert!(matches!(a.mode, Mode::Commands(_)), "still a menu row");
        type_str(&mut a, " x");
        a.handle_key(press(KeyCode::Enter));
        assert_eq!(
            a.outbox,
            vec![Command::Submit {
                text: "/model x".into()
            }]
        );
    }

    #[test]
    fn picking_resume_and_typing_it_do_the_same_thing() {
        let session = SessionChoice {
            id: "s1".into(),
            title: "rate limiting".into(),
            when: "2026-09-20 18:11".into(),
            turns: 3,
        };
        let mut picked = app().with_sessions(vec![session.clone()]);
        picked.handle_key(press(KeyCode::Char('/')));
        picked.handle_key(press(KeyCode::Char('1')));
        let mut typed = app().with_sessions(vec![session]);
        typed.draft.set("/resume".into());
        typed.handle_key(press(KeyCode::Enter));
        for a in [&picked, &typed] {
            assert!(matches!(&a.mode, Mode::Question(q) if q.asker == Asker::Session));
            assert!(a.outbox.is_empty());
        }
    }

    /// Regression: `⌃C` at idle quit with a half-typed message.
    #[test]
    fn ctrl_c_at_idle_clears_a_draft_before_it_quits() {
        let mut a = app();
        type_str(&mut a, "half a thought");
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.draft.is_empty());
        assert!(!a.should_quit, "the first clears");
        a.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.should_quit, "the second leaves");
    }

    #[test]
    fn a_review_says_nothing_into_the_conversation_when_it_opens() {
        let mut a = app();
        a.apply_event(Event::ReviewRequested {
            review_id: "r".into(),
            changeset: Changeset {
                files: vec![ChangedFile {
                    path: "f.rs".into(),
                    before: None,
                    after: "x\n".into(),
                }],
            },
        });
        assert!(a.log.is_empty(), "{:?}", a.log);
    }
}
