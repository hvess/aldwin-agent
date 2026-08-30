use std::collections::HashMap;
use std::sync::Arc;

use mjolnir_core::{Command, Event, StepId};
use mjolnir_permissions::{CheckOutcome, ContextFileTier, Decision, Engine, PromptPayload, PromptResponse, ToolTier};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::log::{summarise, LogEntry, ToolActivityEntry, ToolActivityStatus};
use crate::scroll::ScrollState;

const SUMMARY_MAX_LEN: usize = 80;

/// (line, col) of `cursor` (a char index into `input`, same unit
/// `App::cursor` is kept in) — both counted in chars, 0-indexed. Shared by
/// `App::move_cursor_vertical` (cursor navigation) and `ui::draw_input`
/// (placing the real terminal cursor), so the two can't disagree about
/// where the cursor visually sits.
pub(crate) fn cursor_line_col(input: &str, cursor: usize) -> (usize, usize) {
    let mut line = 0usize;
    let mut col = 0usize;
    for c in input.chars().take(cursor) {
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

pub struct PendingApproval {
    pub call_id: String,
}

pub struct PendingPrompt {
    pub call_id: String,
    pub payload: PromptPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermState {
    Allowed,
    Denied,
}

pub struct StatusInfo {
    pub model_name:    String,
    pub turn:          Option<u64>,
    pub step:          Option<u64>,
    pub running_tools: Vec<String>,
    pub read:          PermState,
    pub shell:         PermState,
    pub edit:          PermState,
}

impl StatusInfo {
    fn refresh_permissions(&mut self, engine: &Engine) {
        self.read = perm_state(engine, "read", false);
        self.shell = perm_state(engine, "shell", false);
        // Always Denied by construction — edit_class: true never returns
        // Allow — but computed the same way as the others rather than
        // hardcoded, so a future change to Engine's edit_class handling
        // can't silently desync the status bar from reality.
        self.edit = perm_state(engine, "edit", true);
    }
}

/// `CheckOutcome::PromptRequired` (deny-by-absence) counts as Denied here —
/// matches mjolnir-tui.md's two-state "allowed or denied" status bar
/// vocabulary; this is a glanceable summary against an empty target (the
/// broadest possible grant), not a precise per-pattern oracle.
fn perm_state(engine: &Engine, kind: &str, edit_class: bool) -> PermState {
    match engine.check_tool(kind, "", edit_class) {
        CheckOutcome::Allow => PermState::Allowed,
        CheckOutcome::Deny | CheckOutcome::PromptRequired(_) => PermState::Denied,
    }
}

/// Application state and the pure logic that mutates it. Rendering (`ui.rs`)
/// only ever reads from this; the actual terminal/event-loop glue (`run.rs`)
/// only ever calls `apply_event`/`handle_key` and does no interpretation of
/// its own — kept this way so both are unit-testable without a real
/// terminal or channels.
pub struct App {
    permissions: Arc<Engine>,

    pub log:              Vec<LogEntry>,
    pub thinking:          bool,
    pub scroll:            ScrollState,
    /// The log area's real render width, last set by `ui::draw` right
    /// before it calls `total_lines()`. `total_lines()` needs a width to
    /// count wrapped screen rows (see its doc comment); scroll navigation
    /// (`handle_key`, `push`) happens between draws with no render access
    /// of its own, so it reads this cached value rather than the true
    /// current-frame width — off by at most one stale frame, on a
    /// terminal resize, until the next draw corrects it. Defaults to a
    /// plausible starting width so `total_lines()` is never called before
    /// any draw has run.
    pub render_width:     u16,
    pub input:             String,
    pub cursor:            usize, // char index into `input`
    pub pending_approval:  Option<PendingApproval>,
    pub pending_prompt:    Option<PendingPrompt>,
    pub status:            StatusInfo,
    pub should_quit:       bool,
    /// True from `TurnStarted` until the matching `TurnEnded` — drives the
    /// "working" activity indicator in `ui::build_log_lines` for the stretch
    /// of a turn (between tool calls, before the first token streams back)
    /// that `thinking` alone doesn't cover, since `thinking` is only set
    /// between `ThinkingStart`/`ThinkingEnd` (extended-thinking blocks).
    pub turn_active:       bool,
    /// Free-running animation-frame counter, advanced by `tick` (called by
    /// `run.rs` on a fixed timer) — not wall-clock time itself, so the
    /// spinner's frame selection stays deterministic and testable without a
    /// real clock.
    pub tick:              u64,

    /// Populated on `ToolUseRequested` (the one event that carries the
    /// tool's name), consumed on `ToolDispatched` (which only carries
    /// `call_id`) to give `ToolActivityEntry` a name at all.
    pending_tool_names: HashMap<String, String>,

    /// Commands `handle_key`/`apply_event` want sent — drained by the event
    /// loop after each call, rather than this struct holding a live sender,
    /// so both can be exercised in tests without a channel.
    pub outbox: Vec<Command>,
}

impl App {
    pub fn new(model_name: String, permissions: Arc<Engine>) -> Self {
        let mut status = StatusInfo { model_name, turn: None, step: None, running_tools: vec![], read: PermState::Denied, shell: PermState::Denied, edit: PermState::Denied };
        status.refresh_permissions(&permissions);
        Self {
            permissions,
            log: Vec::new(),
            thinking: false,
            scroll: ScrollState::default(),
            render_width: 80,
            input: String::new(),
            cursor: 0,
            pending_approval: None,
            pending_prompt: None,
            status,
            should_quit: false,
            turn_active: false,
            tick: 0,
            pending_tool_names: HashMap::new(),
            outbox: Vec::new(),
        }
    }

    /// Advances the animation-frame counter — called by `run.rs` on a fixed
    /// timer so the "working"/"thinking" spinner animates independently of
    /// core events or keystrokes.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn push(&mut self, entry: LogEntry) {
        self.log.push(entry);
        self.scroll.on_content_grew(self.total_lines());
    }

    /// Total rendered terminal rows across the whole log, wrapping
    /// included — what `ScrollState` actually needs to compare against
    /// `viewport_height` (also rows), not `self.log.len()` (entry count)
    /// and not a logical (pre-wrap) line count either. Mixing entry count
    /// in for `viewport_height` is what made scrolling effectively a
    /// no-op before `total_lines` existed at all: a handful of entries
    /// routinely render to far more rows than the viewport, so an
    /// entry-count-based `max_offset` stayed 0 long after there was real
    /// content to scroll to. Using a *logical* line count (one row per
    /// source line) fixed that but stayed wrong on its own terms: any
    /// single line wide enough to wrap at the current `render_width` — a
    /// long tool-result summary, a long retry message, a long assistant
    /// line — rendered as more screen rows than it counted as, so
    /// `ScrollState`'s offset drifted out of sync with what was actually
    /// on screen and clipped content at the bottom of the log area (see
    /// mjolnir-tui.md's 2026-08-29 scrolling-fix Progress note). Delegates
    /// to `ui::log_row_count`, which counts the exact same wrapped rows
    /// `ui::draw_log` renders, using ratatui's own wrapper rather than a
    /// hand-kept approximation.
    pub fn total_lines(&self) -> usize {
        crate::ui::log_row_count(self, self.render_width)
    }

    fn active_step_calls(&mut self, step_id: StepId) -> Option<&mut Vec<ToolActivityEntry>> {
        self.log.iter_mut().rev().find_map(|e| match e {
            LogEntry::ToolActivity { step_id: sid, calls } if *sid == step_id => Some(calls),
            _ => None,
        })
    }

    pub fn apply_event(&mut self, event: Event) {
        match event {
            Event::TurnStarted { turn_id } => {
                self.status.turn = Some(turn_id.0);
                self.status.step = None;
                self.turn_active = true;
            }
            Event::TextDelta { text, .. } => {
                if let Some(LogEntry::AssistantText { text: buf }) = self.log.last_mut() {
                    buf.push_str(&text);
                } else {
                    self.push(LogEntry::AssistantText { text });
                }
            }
            Event::ThinkingStart { .. } => self.thinking = true,
            Event::ThinkingEnd { .. } => self.thinking = false,
            Event::ToolUseRequested { call, .. } => {
                self.pending_tool_names.insert(call.id, call.name);
            }
            Event::ToolDispatched { step_id, call_id, .. } => {
                self.status.running_tools.push(call_id.clone());
                let name = self.pending_tool_names.remove(&call_id).unwrap_or_default();
                match self.active_step_calls(step_id) {
                    Some(calls) => calls.push(ToolActivityEntry { call_id, name, status: ToolActivityStatus::Running }),
                    None => {
                        self.push(LogEntry::ToolActivity { step_id, calls: vec![ToolActivityEntry { call_id, name, status: ToolActivityStatus::Running }] })
                    }
                }
            }
            Event::ToolApprovalRequested { call_id, diff, .. } => {
                self.pending_approval = Some(PendingApproval { call_id: call_id.clone() });
                self.push(LogEntry::ApprovalCard { call_id, diff, resolution: None });
            }
            Event::ToolCompleted { step_id, result, .. } => {
                self.status.running_tools.retain(|id| id != &result.call_id);
                let summary = summarise(&result.content, SUMMARY_MAX_LEN);
                if let Some(calls) = self.active_step_calls(step_id) {
                    if let Some(call) = calls.iter_mut().find(|c| c.call_id == result.call_id) {
                        call.status = ToolActivityStatus::Completed { is_error: result.is_error, summary };
                    }
                }
            }
            Event::StepEnded { .. } => {}
            Event::RetryAttempt { info, .. } => self.push(LogEntry::RetryAttempt { info }),
            Event::TurnEnded { reason, .. } => {
                self.status.running_tools.clear();
                self.turn_active = false;
                self.push(LogEntry::TurnEnded { reason: reason.into() });
            }
            Event::PromptRequested { call_id, payload } => {
                let parsed: Result<PromptPayload, _> = serde_json::from_value(payload);
                match parsed {
                    Ok(payload) => {
                        self.pending_prompt = Some(PendingPrompt { call_id: call_id.clone(), payload: payload.clone() });
                        self.push(LogEntry::PermissionPrompt { call_id, payload, resolution: None });
                    }
                    Err(e) => self.push(LogEntry::Error { message: format!("malformed permission prompt: {e}") }),
                }
            }
            Event::PermissionsChanged { .. } => self.status.refresh_permissions(&self.permissions),
            Event::Notice { message } => self.push(LogEntry::Notice { message }),
            // `/clear` — core's ConversationLog is authoritative for what
            // the LLM sees, so the TUI's own rendered log must actually be
            // wiped in step with it, not just told about it via a Notice
            // (which only appends). Resetting `scroll` to its default drops
            // any stale offset/`following` state from before the clear; the
            // welcome banner reappears on the next draw since it's rendered
            // whenever `log` is empty (see `ui::build_log_lines`).
            Event::HistoryCleared => {
                self.log.clear();
                self.scroll = ScrollState::default();
                self.status.turn = None;
                self.status.step = None;
                self.thinking = false;
                self.turn_active = false;
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }

        if self.pending_approval.is_some() {
            self.handle_approval_key(key);
            return;
        }
        if self.pending_prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Enter, m) if m.contains(KeyModifiers::SHIFT) => self.insert_char('\n'),
            (KeyCode::Enter, _) => self.submit(),
            // Shift+Enter's reporting is terminal-dependent (plain xterm
            // without the Kitty keyboard protocol can't distinguish it from
            // bare Enter) — Ctrl+J (linefeed) is a fallback that gets
            // through on terminals where Shift+Enter doesn't.
            (KeyCode::Char('j'), m) if m.contains(KeyModifiers::CONTROL) => self.insert_char('\n'),
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => self.cancel_or_quit(),
            (KeyCode::Backspace, _) => self.backspace(),
            (KeyCode::Delete, _) => self.delete_forward(),
            (KeyCode::Left, _) => self.cursor = self.cursor.saturating_sub(1),
            (KeyCode::Right, _) => self.cursor = (self.cursor + 1).min(self.input.chars().count()),
            (KeyCode::Home, _) => self.cursor = 0,
            (KeyCode::End, _) => {
                self.cursor = self.input.chars().count();
                self.scroll.jump_to_bottom(self.total_lines());
            }
            (KeyCode::PageUp, _) => self.scroll.page_up(),
            (KeyCode::PageDown, _) => self.scroll.page_down(self.total_lines()),
            // Within a multi-line draft, Up/Down move the cursor between its
            // lines first; only once there's no further line to move to
            // (a single-line draft, or already at the draft's first/last
            // line) do they fall through to scrolling the log. Previously
            // this — and a separate set of vim-style j/k/G bindings — used
            // "only when the input is empty" as the guard, which silently
            // swallowed the first keystroke of any message starting with
            // j, k, or a capital G instead of inserting it (the vim
            // bindings are gone outright: PageUp/PageDown/Home/End already
            // cover keyboard scrolling without that ambiguity).
            (KeyCode::Up, _) => {
                if !self.move_cursor_vertical(-1) {
                    self.scroll.line_up();
                }
            }
            (KeyCode::Down, _) => {
                if !self.move_cursor_vertical(1) {
                    self.scroll.line_down(self.total_lines());
                }
            }
            (KeyCode::Char(c), _) => self.insert_char(c),
            _ => {}
        }
    }

    /// Moves the cursor to the line `delta` rows away (by source line, not
    /// wrapped screen row — the input box is short enough that this rarely
    /// matters, and ratatui's own wrap point isn't available to this pure
    /// logic layer without threading render width all the way through key
    /// handling), preserving column where possible. Returns `false` (leaving
    /// the cursor untouched) when there's no such line — a single-line
    /// draft, or already at its first/last line — so callers can fall
    /// through to scrolling the log instead.
    fn move_cursor_vertical(&mut self, delta: isize) -> bool {
        let lines: Vec<&str> = self.input.split('\n').collect();
        let (line, col) = cursor_line_col(&self.input, self.cursor);
        let Some(target) = line.checked_add_signed(delta).filter(|&t| t < lines.len()) else { return false };
        let target_len = lines[target].chars().count();
        let new_col = col.min(target_len);
        let idx: usize = lines[..target].iter().map(|l| l.chars().count() + 1).sum::<usize>() + new_col;
        self.cursor = idx;
        true
    }

    fn insert_char(&mut self, c: char) {
        let byte_idx = self.input.char_indices().nth(self.cursor).map(|(i, _)| i).unwrap_or(self.input.len());
        self.input.insert(byte_idx, c);
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let byte_idx = self.input.char_indices().nth(self.cursor - 1).map(|(i, _)| i).unwrap();
        self.input.remove(byte_idx);
        self.cursor -= 1;
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
        self.push(LogEntry::UserMessage { text: text.clone() });
        self.outbox.push(Command::Submit { text });
    }

    /// Ctrl+C: cancel the active turn if one is running (a turn is "active"
    /// once TurnStarted has landed and hasn't yet been closed out by
    /// TurnEnded — tracked via the last TurnEnded/TurnStarted seen in the
    /// log), otherwise exit.
    fn cancel_or_quit(&mut self) {
        let turn_active = self.log.iter().rev().find_map(|e| match e {
            LogEntry::TurnEnded { .. } => Some(false),
            LogEntry::UserMessage { .. } => Some(true),
            _ => None,
        });
        if turn_active == Some(true) {
            self.outbox.push(Command::Cancel);
        } else {
            self.should_quit = true;
        }
    }

    fn handle_approval_key(&mut self, key: KeyEvent) {
        let decision = match key.code {
            KeyCode::Char('y') => true,
            KeyCode::Char('n') => false,
            // Ctrl+C must always be a way out, even mid-approval — denying
            // is the safe default and matches 'n', rather than leaving the
            // developer with no responsive key at all if they don't already
            // know y/n.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => false,
            _ => return, // any other key is silently dropped — no typing ahead
        };
        let Some(pending) = self.pending_approval.take() else { return };
        for entry in self.log.iter_mut() {
            if let LogEntry::ApprovalCard { call_id, resolution, .. } = entry {
                if *call_id == pending.call_id {
                    *resolution = Some(decision);
                }
            }
        }
        self.outbox.push(if decision { Command::ApproveTool { call_id: pending.call_id } } else { Command::DenyTool { call_id: pending.call_id } });
    }

    /// Tool four-tier: o/s/p/a = allow once/session/project/always;
    /// shift O/S/P/A = deny at the same tiers. Context-file two-tier: s/p =
    /// approve session/project, n = decline. Labels are rendered in the
    /// card itself (see `ui.rs`) — see mjolnir-tui.md's Pitfall on
    /// requiring an unambiguous labeled key.
    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let Some(pending) = &self.pending_prompt else { return };
        // Ctrl+C always declines, regardless of payload shape — same
        // rationale as handle_approval_key: a stuck prompt with no
        // recognized key otherwise has no escape hatch.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let decline = match &pending.payload {
                PromptPayload::Tool { .. } => Some(PromptResponse::Tool { decision: Decision::Deny, tier: ToolTier::Once }),
                PromptPayload::ContextFile { .. } => Some(PromptResponse::ContextFile { approve: false, tier: None }),
                PromptPayload::Edit { .. } => None,
            };
            if let Some(response) = decline {
                self.resolve_prompt(response);
            }
            return;
        }
        let response = match &pending.payload {
            PromptPayload::Tool { .. } => match key.code {
                KeyCode::Char('o') => Some(PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Once }),
                KeyCode::Char('s') => Some(PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Session }),
                KeyCode::Char('p') => Some(PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Project }),
                KeyCode::Char('a') => Some(PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Always }),
                KeyCode::Char('O') => Some(PromptResponse::Tool { decision: Decision::Deny, tier: ToolTier::Once }),
                KeyCode::Char('S') => Some(PromptResponse::Tool { decision: Decision::Deny, tier: ToolTier::Session }),
                KeyCode::Char('P') => Some(PromptResponse::Tool { decision: Decision::Deny, tier: ToolTier::Project }),
                KeyCode::Char('A') => Some(PromptResponse::Tool { decision: Decision::Deny, tier: ToolTier::Always }),
                _ => None,
            },
            PromptPayload::ContextFile { .. } => match key.code {
                KeyCode::Char('s') => Some(PromptResponse::ContextFile { approve: true, tier: Some(ContextFileTier::Session) }),
                KeyCode::Char('p') => Some(PromptResponse::ContextFile { approve: true, tier: Some(ContextFileTier::Project) }),
                KeyCode::Char('n') => Some(PromptResponse::ContextFile { approve: false, tier: None }),
                _ => None,
            },
            // Never actually sent through this channel in production (Edit
            // uses the separate ToolApprovalRequested round trip) — no key
            // resolves it; it can only be dismissed by the round trip never
            // arriving, which isn't reachable in practice.
            PromptPayload::Edit { .. } => None,
        };
        let Some(response) = response else { return };
        self.resolve_prompt(response);
    }

    fn resolve_prompt(&mut self, response: PromptResponse) {
        let Some(pending) = self.pending_prompt.take() else { return };
        let call_id = pending.call_id;
        let label = format!("{response:?}");
        for entry in self.log.iter_mut() {
            if let LogEntry::PermissionPrompt { call_id: entry_call_id, resolution, .. } = entry {
                if *entry_call_id == call_id {
                    *resolution = Some(label.clone());
                }
            }
        }
        let payload = serde_json::to_value(&response).expect("PromptResponse always serialises");
        self.outbox.push(Command::PromptResponse { call_id, payload });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mjolnir_config::Config;
    use mjolnir_core::{ToolCall, ToolResult, TurnEndReason, TurnId};
    use ratatui::crossterm::event::KeyEventState;

    fn engine() -> Arc<Engine> {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
        Arc::new(Engine::new(config))
    }

    fn app() -> App {
        App::new("claude-sonnet-5".into(), engine())
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

    #[test]
    fn typing_and_enter_submits_and_clears_input() {
        let mut app = app();
        type_str(&mut app, "hello");
        app.handle_key(press(KeyCode::Enter));
        assert_eq!(app.input, "");
        assert_eq!(app.outbox, vec![Command::Submit { text: "hello".into() }]);
        assert!(matches!(app.log.last(), Some(LogEntry::UserMessage { text }) if text == "hello"));
    }

    #[test]
    fn empty_submit_is_a_no_op() {
        let mut app = app();
        app.handle_key(press(KeyCode::Enter));
        assert!(app.outbox.is_empty());
        assert!(app.log.is_empty());
    }

    #[test]
    fn shift_enter_inserts_a_newline_instead_of_submitting() {
        let mut app = app();
        type_str(&mut app, "a");
        app.handle_key(press_mod(KeyCode::Enter, KeyModifiers::SHIFT));
        type_str(&mut app, "b");
        assert_eq!(app.input, "a\nb");
        assert!(app.outbox.is_empty());
    }

    #[test]
    fn ctrl_j_is_a_newline_fallback_for_terminals_that_eat_shift_enter() {
        let mut app = app();
        type_str(&mut app, "a");
        app.handle_key(press_mod(KeyCode::Char('j'), KeyModifiers::CONTROL));
        type_str(&mut app, "b");
        assert_eq!(app.input, "a\nb");
        assert!(app.outbox.is_empty());
    }

    /// Regression test: a former vim-style binding treated 'G'/'j'/'k' as
    /// scroll commands whenever the input was empty, which silently ate the
    /// very first keystroke of any message starting with one of those
    /// letters instead of inserting it — this is what the developer
    /// actually hit typing a capital G as the first character of a draft.
    #[test]
    fn typing_g_j_or_k_as_the_first_character_inserts_it_instead_of_scrolling() {
        for first in ["G", "j", "k"] {
            let mut app = app();
            type_str(&mut app, first);
            assert_eq!(app.input, first, "first keystroke {first:?} must be inserted, not swallowed as a scroll command");
        }
    }

    #[test]
    fn up_and_down_navigate_a_multiline_draft_before_falling_through_to_scroll() {
        let mut app = app();
        type_str(&mut app, "line one\nline two\nline three");
        // Cursor starts at the end (line 2, col 10 within "line three").
        // "line two" is only 8 chars, so the column clamps on the way up.
        app.handle_key(press(KeyCode::Up));
        assert_eq!(super::cursor_line_col(&app.input, app.cursor), (1, 8), "Up should clamp to the shorter middle line's length");
        app.handle_key(press(KeyCode::Up));
        assert_eq!(super::cursor_line_col(&app.input, app.cursor), (0, 8), "Up again should land on the same column, which the first line can also fit");
        // No line above the first — Up here must not move the cursor
        // further (it falls through to scrolling the log instead).
        app.handle_key(press(KeyCode::Up));
        assert_eq!(super::cursor_line_col(&app.input, app.cursor), (0, 8));
    }

    #[test]
    fn up_arrow_scrolls_the_log_when_the_draft_is_single_line() {
        let mut app = app();
        for i in 0..20 {
            app.log.push(LogEntry::AssistantText { text: format!("line-{i}") });
        }
        app.render_width = 80;
        app.scroll.set_viewport_height(5, app.total_lines());
        let before = app.scroll.offset;
        app.handle_key(press(KeyCode::Up));
        assert!(app.scroll.offset < before, "Up must scroll the log when there's no draft line to navigate to");
    }

    #[test]
    fn backspace_removes_the_character_before_the_cursor() {
        let mut app = app();
        type_str(&mut app, "ab");
        app.handle_key(press(KeyCode::Backspace));
        assert_eq!(app.input, "a");
    }

    #[test]
    fn ctrl_c_with_no_active_turn_quits() {
        let mut app = app();
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
        assert!(app.outbox.is_empty());
    }

    #[test]
    fn ctrl_c_with_an_active_turn_cancels_instead_of_quitting() {
        let mut app = app();
        type_str(&mut app, "go");
        app.handle_key(press(KeyCode::Enter)); // submits -> turn considered active
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.should_quit);
        assert_eq!(app.outbox.last(), Some(&Command::Cancel));
    }

    #[test]
    fn ctrl_c_after_turn_ended_quits_again() {
        let mut app = app();
        type_str(&mut app, "go");
        app.handle_key(press(KeyCode::Enter));
        app.apply_event(Event::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::EndTurn });
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn typing_ahead_while_an_approval_card_is_pending_is_silently_dropped() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "-a\n+b\n".into() });
        type_str(&mut app, "hello");
        assert_eq!(app.input, "", "keystrokes must not leak into the input buffer while a card is pending");
        assert!(app.pending_approval.is_some());
    }

    #[test]
    fn approval_card_requires_y_or_n_not_enter() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        app.handle_key(press(KeyCode::Enter));
        assert!(app.pending_approval.is_some(), "Enter must not resolve the card");
        app.handle_key(press(KeyCode::Char('y')));
        assert!(app.pending_approval.is_none());
        assert_eq!(app.outbox, vec![Command::ApproveTool { call_id: "c1".into() }]);
    }

    #[test]
    fn approval_card_n_denies() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        app.handle_key(press(KeyCode::Char('n')));
        assert_eq!(app.outbox, vec![Command::DenyTool { call_id: "c1".into() }]);
    }

    #[test]
    fn ctrl_c_denies_a_pending_approval_card_instead_of_being_swallowed() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.pending_approval.is_none(), "Ctrl+C must resolve a pending approval card, not get stuck");
        assert_eq!(app.outbox, vec![Command::DenyTool { call_id: "c1".into() }]);
    }

    #[test]
    fn ctrl_c_declines_a_pending_tool_prompt_instead_of_being_swallowed() {
        let mut app = app();
        let payload = serde_json::to_value(PromptPayload::Tool { kind: "shell".into(), target: "rm -rf /".into() }).unwrap();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.pending_prompt.is_none(), "Ctrl+C must resolve a pending permission prompt, not get stuck");
        match app.outbox.last() {
            Some(Command::PromptResponse { payload, .. }) => {
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::Tool { decision: Decision::Deny, tier: ToolTier::Once });
            }
            other => panic!("expected PromptResponse, got {other:?}"),
        }
    }

    #[test]
    fn ctrl_c_declines_a_pending_context_file_prompt() {
        let mut app = app();
        let payload = serde_json::to_value(PromptPayload::ContextFile { path: "AGENTS.md".into() }).unwrap();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.pending_prompt.is_none());
        match app.outbox.last() {
            Some(Command::PromptResponse { payload, .. }) => {
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::ContextFile { approve: false, tier: None });
            }
            other => panic!("expected PromptResponse, got {other:?}"),
        }
    }

    #[test]
    fn permission_prompt_resolves_on_labeled_key_and_records_resolution() {
        let mut app = app();
        let payload = serde_json::to_value(PromptPayload::Tool { kind: "shell".into(), target: "git status".into() }).unwrap();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });
        assert!(app.pending_prompt.is_some());

        app.handle_key(press(KeyCode::Char('p'))); // allow, project tier
        assert!(app.pending_prompt.is_none());
        match app.outbox.last() {
            Some(Command::PromptResponse { payload, .. }) => {
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Project });
            }
            other => panic!("expected PromptResponse, got {other:?}"),
        }
    }

    #[test]
    fn text_deltas_within_a_step_coalesce_into_one_log_entry() {
        let mut app = app();
        app.apply_event(Event::TextDelta { turn_id: TurnId(1), step_id: StepId(1), text: "Hel".into() });
        app.apply_event(Event::TextDelta { turn_id: TurnId(1), step_id: StepId(1), text: "lo".into() });
        assert_eq!(app.log.len(), 1);
        assert!(matches!(&app.log[0], LogEntry::AssistantText { text } if text == "Hello"));
    }

    #[test]
    fn thinking_start_and_end_toggle_a_flag_not_a_log_entry() {
        let mut app = app();
        app.apply_event(Event::ThinkingStart { turn_id: TurnId(1), step_id: StepId(1) });
        assert!(app.thinking);
        assert!(app.log.is_empty());
        app.apply_event(Event::ThinkingEnd { turn_id: TurnId(1), step_id: StepId(1) });
        assert!(!app.thinking);
    }

    #[test]
    fn tool_activity_groups_by_step_and_closes_out_on_completion() {
        let mut app = app();
        let call = ToolCall { id: "c1".into(), name: "read".into(), input: serde_json::json!({}) };
        app.apply_event(Event::ToolUseRequested { turn_id: TurnId(1), step_id: StepId(1), call });
        app.apply_event(Event::ToolDispatched { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into() });
        assert_eq!(app.status.running_tools, vec!["c1".to_string()]);

        app.apply_event(Event::ToolCompleted {
            turn_id: TurnId(1),
            step_id: StepId(1),
            result: ToolResult { call_id: "c1".into(), content: "file contents".into(), is_error: false },
        });
        assert!(app.status.running_tools.is_empty());

        let LogEntry::ToolActivity { calls, .. } = &app.log[0] else { panic!("expected ToolActivity") };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read", "name from ToolUseRequested must survive to the activity entry");
        assert!(matches!(&calls[0].status, ToolActivityStatus::Completed { is_error: false, summary } if summary == "file contents"));
    }

    #[test]
    fn a_second_tool_in_the_same_step_joins_the_existing_group() {
        let mut app = app();
        app.apply_event(Event::ToolDispatched { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into() });
        app.apply_event(Event::ToolDispatched { turn_id: TurnId(1), step_id: StepId(1), call_id: "c2".into() });
        assert_eq!(app.log.len(), 1, "both calls in step 1 must group into one ToolActivity entry");
        let LogEntry::ToolActivity { calls, .. } = &app.log[0] else { panic!("expected ToolActivity") };
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn retry_attempt_renders_as_a_visible_log_entry() {
        let mut app = app();
        let info = mjolnir_core::RetryInfo { provider: "anthropic".into(), status: Some(529), message: "overloaded".into(), attempt: 1 };
        app.apply_event(Event::RetryAttempt { turn_id: TurnId(1), step_id: StepId(1), info: info.clone() });
        assert!(matches!(app.log.last(), Some(LogEntry::RetryAttempt { info: i }) if *i == info));
    }

    #[test]
    fn permissions_changed_refreshes_the_status_bar() {
        let mut app = app();
        assert_eq!(app.status.read, PermState::Denied);
        app.apply_event(Event::PermissionsChanged { payload: serde_json::Value::Null });
        // Still denied (no grant recorded) but proves the refresh path runs
        // without panicking on an opaque payload it doesn't need to parse.
        assert_eq!(app.status.read, PermState::Denied);
    }

    #[test]
    fn notice_event_becomes_a_notice_log_entry() {
        let mut app = app();
        app.apply_event(Event::Notice { message: "unknown slash command: /foo".into() });
        assert!(matches!(app.log.last(), Some(LogEntry::Notice { message }) if message == "unknown slash command: /foo"));
    }

    #[test]
    fn history_cleared_wipes_the_rendered_log_and_turn_state() {
        let mut app = app();
        app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        app.push(LogEntry::AssistantText { text: "hi".into() });
        assert!(!app.log.is_empty());
        assert!(app.turn_active);

        app.apply_event(Event::HistoryCleared);
        assert!(app.log.is_empty(), "the visible log must be wiped in step with core's conversation history");
        assert!(!app.turn_active);
        assert_eq!(app.status.turn, None);
    }

    #[test]
    fn turn_active_tracks_turn_started_and_ended() {
        let mut app = app();
        assert!(!app.turn_active);
        app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        assert!(app.turn_active);
        app.apply_event(Event::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::EndTurn });
        assert!(!app.turn_active);
    }

    #[test]
    fn malformed_prompt_payload_is_a_log_error_not_a_panic() {
        let mut app = app();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload: serde_json::json!({"shape": "unknown_shape"}) });
        assert!(matches!(app.log.last(), Some(LogEntry::Error { .. })));
        assert!(app.pending_prompt.is_none());
    }
}
