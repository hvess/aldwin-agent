use std::collections::{HashMap, VecDeque};
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

/// `diff` rides along here (not just `call_id`) for the same reason
/// `PendingPrompt` already carries its own `payload`: `ui::draw`'s decision
/// panel needs the full content of whichever request is at the front of the
/// queue on every frame, and re-deriving that by scanning `App::log` for a
/// matching, still-unresolved `LogEntry::ApprovalCard` would make the panel
/// depend on an invariant ("there's always exactly one such entry") the type
/// system can't enforce, instead of just holding what it needs directly.
pub struct PendingApproval {
    pub call_id: String,
    pub diff:    String,
}

pub struct PendingPrompt {
    pub call_id: String,
    pub payload: PromptPayload,
}

/// Which queue is currently interactive: the front of `pending_approvals` if
/// it holds anything, else the front of `pending_prompts`, else neither.
/// This is the *single* place "approvals resolve before prompts" is decided
/// — `App::decision_options`/`decline_outcome` and `ui::decision_panel_lines`
/// all go through `App::pending_front` instead of independently re-checking
/// `pending_approvals.front().is_some()` themselves. That used to be
/// duplicated across four call sites; a rust-skills audit flagged it as a
/// maintainability risk — the exact "priority checked one way here, another
/// way there" bug already happened once this session, in the other
/// direction, at the status-line/`handle_key` boundary — so it's
/// consolidated here rather than left to drift.
pub enum PendingFront<'a> {
    Approval(&'a PendingApproval),
    Prompt(&'a PendingPrompt),
    None,
}

/// What resolving a `DecisionOption` actually does — one variant per pending
/// gate, so `App::resolve_decision` knows which queue to pop from without
/// re-inspecting the payload the option was built from.
#[derive(Debug, Clone, PartialEq)]
pub enum DecisionOutcome {
    Approve(bool),
    Prompt(PromptResponse),
}

/// One selectable, numbered choice in the decision panel's list — a human
/// label (rendered as `"{n}. {label}"` by `ui.rs`) plus the concrete
/// `Command` selecting it produces. Built fresh from whichever request is at
/// the front of the queue (`App::decision_options`) on every draw/keypress,
/// so rendering and resolution can never disagree about what option N means.
pub struct DecisionOption {
    pub label:   String,
    pub outcome: DecisionOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermState {
    Allowed,
    Denied,
}

/// A tool call currently in flight, for the status line's "active tools"
/// list. Carries the human-readable name (not just the opaque `call_id`) so
/// the status line can show what's actually running, not just an id — the
/// name only otherwise exists transiently (`App::pending_tool_names`) or on
/// the matching `ToolActivity` log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningTool {
    pub call_id: String,
    pub name:    String,
}

pub struct StatusInfo {
    pub model_name:    String,
    pub turn:          Option<u64>,
    pub step:          Option<u64>,
    pub running_tools: Vec<RunningTool>,
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
    /// The log panel's real inner render height (post-border), last set by
    /// `ui::draw` alongside `render_width` — same reasoning: `hero_lines`
    /// needs a pane height to vertically center the welcome banner, and
    /// that number is only known at render time. Not used by scroll math
    /// itself (`total_lines()` only needs `render_width`), only by the
    /// hero-centering path.
    pub render_height:    u16,
    pub input:             String,
    pub cursor:            usize, // char index into `input`
    /// Queued, not a single slot — parallel tool use can dispatch several
    /// Edit calls in one step, each requesting approval independently (see
    /// `dispatch_tools`' `future::join_all` in mjolnir-core), so more than
    /// one can be outstanding at once. A second `ToolApprovalRequested`
    /// arriving while the first was still an `Option` silently overwrote
    /// it — the first call's approval channel then hung forever with no
    /// key able to reach it, which stalled that dispatch future (and, via
    /// `join_all`, the whole step) until Ctrl+C cancelled the turn; the
    /// developer only ever saw the one card that happened to win the
    /// overwrite. The front of the queue is the one actually interactive
    /// (`handle_approval_key`/`handle_prompt_key` only ever act on it);
    /// resolving it pops the front and the next queued one becomes
    /// interactive automatically. See `mjolnir-tui.md`'s Progress note.
    pub pending_approvals: VecDeque<PendingApproval>,
    /// Same reasoning as `pending_approvals` — a generic permission prompt
    /// can equally arrive for more than one dispatched call at once.
    pub pending_prompts:   VecDeque<PendingPrompt>,
    /// Index into whichever `App::decision_options()` list is current —
    /// moved by Up/Down, confirmed by Enter, per explicit developer request
    /// that the decision panel be a real navigable numbered list ("1. Yes
    /// 2. Yes session 3. No") rather than raw keyboard-shortcut hints
    /// ("[o]nce [s]ession..."). Reset to 0 whenever the front of either
    /// queue changes — a fresh request's list always starts unselected at
    /// its first (least consequential) option, never wherever the cursor
    /// happened to sit for a previous, unrelated request.
    pub decision_selected: usize,
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
            render_height: 24,
            input: String::new(),
            cursor: 0,
            pending_approvals: VecDeque::new(),
            pending_prompts: VecDeque::new(),
            decision_selected: 0,
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
        crate::ui::log_row_count(self, self.render_width, self.render_height)
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
                let name = self.pending_tool_names.remove(&call_id).unwrap_or_default();
                self.status.running_tools.push(RunningTool { call_id: call_id.clone(), name: name.clone() });
                match self.active_step_calls(step_id) {
                    Some(calls) => calls.push(ToolActivityEntry { call_id, name, status: ToolActivityStatus::Running }),
                    None => {
                        self.push(LogEntry::ToolActivity { step_id, calls: vec![ToolActivityEntry { call_id, name, status: ToolActivityStatus::Running }] })
                    }
                }
            }
            Event::ToolApprovalRequested { call_id, diff, .. } => {
                // Approvals always take interactive priority over prompts
                // (see `handle_key`/`decision_options`), so this becomes the
                // new front-and-center list the moment `pending_approvals`
                // itself was empty — regardless of whether a prompt was
                // already showing. The cursor must start fresh on it, not
                // wherever it happened to sit for whatever was showing
                // before.
                if self.pending_approvals.is_empty() {
                    self.decision_selected = 0;
                }
                self.pending_approvals.push_back(PendingApproval { call_id: call_id.clone(), diff: diff.clone() });
                self.push(LogEntry::ApprovalCard { call_id, diff, resolution: None });
            }
            Event::ToolCompleted { step_id, result, .. } => {
                self.status.running_tools.retain(|t| t.call_id != result.call_id);
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
                        // Only becomes interactive (and so only needs a
                        // fresh cursor) when *both* queues were empty — a
                        // prompt never preempts an already-pending approval.
                        if self.pending_approvals.is_empty() && self.pending_prompts.is_empty() {
                            self.decision_selected = 0;
                        }
                        self.pending_prompts.push_back(PendingPrompt { call_id: call_id.clone(), payload: payload.clone() });
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

        if !self.pending_approvals.is_empty() || !self.pending_prompts.is_empty() {
            self.handle_decision_key(key);
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

    /// The decision panel's selectable options for whichever request is at
    /// the front of the queue, in the order they're numbered/listed —
    /// empty when nothing is pending. Mirrors `handle_key`'s own priority:
    /// approvals before prompts (`ui::decision_panel_lines` must show
    /// exactly this same list, in this same order, or a developer could
    /// pick "option 2" expecting one outcome and get another).
    pub fn decision_options(&self) -> Vec<DecisionOption> {
        match self.pending_front() {
            PendingFront::Approval(_) => vec![
                DecisionOption { label: "Approve".into(), outcome: DecisionOutcome::Approve(true) },
                DecisionOption { label: "Deny".into(), outcome: DecisionOutcome::Approve(false) },
            ],
            PendingFront::Prompt(pending) => match &pending.payload {
                // Allow tiers before deny tiers, once→session→project→always
                // within each — same tier ordering the old o/s/p/a shortcuts
                // used, just spelled out as list labels instead of letters.
                PromptPayload::Tool { .. } => [
                    (Decision::Allow, ToolTier::Once, "Allow once"),
                    (Decision::Allow, ToolTier::Session, "Allow for this session"),
                    (Decision::Allow, ToolTier::Project, "Allow for this project"),
                    (Decision::Allow, ToolTier::Always, "Always allow"),
                    (Decision::Deny, ToolTier::Once, "Deny once"),
                    (Decision::Deny, ToolTier::Session, "Deny for this session"),
                    (Decision::Deny, ToolTier::Project, "Deny for this project"),
                    (Decision::Deny, ToolTier::Always, "Always deny"),
                ]
                .into_iter()
                .map(|(decision, tier, label)| DecisionOption { label: label.into(), outcome: DecisionOutcome::Prompt(PromptResponse::Tool { decision, tier }) })
                .collect(),
                PromptPayload::ContextFile { .. } => vec![
                    DecisionOption {
                        label:   "Inject for this session".into(),
                        outcome: DecisionOutcome::Prompt(PromptResponse::ContextFile { approve: true, tier: Some(ContextFileTier::Session) }),
                    },
                    DecisionOption {
                        label:   "Inject for this project".into(),
                        outcome: DecisionOutcome::Prompt(PromptResponse::ContextFile { approve: true, tier: Some(ContextFileTier::Project) }),
                    },
                    DecisionOption { label: "No".into(), outcome: DecisionOutcome::Prompt(PromptResponse::ContextFile { approve: false, tier: None }) },
                ],
                // Never actually sent through this round trip in production —
                // Edit uses the separate ToolApprovalRequested/ApprovalCard
                // path instead — so there's no real option list to offer.
                PromptPayload::Edit { .. } => Vec::new(),
            },
            PendingFront::None => Vec::new(),
        }
    }

    /// The safe "decline" outcome for whichever request is at the front of
    /// the queue, reachable via Ctrl+C regardless of where the list cursor
    /// sits — always the least consequential choice (deny *once*), not
    /// whatever happens to be the list's last entry: for a Tool prompt
    /// that's "always deny," a far more consequential and harder-to-reverse
    /// action than the one-time decline Ctrl+C has always meant (see
    /// mjolnir-tui.md's 2026-08-29 live-run fix and its Pitfall on requiring
    /// an unambiguous way out). Keeping this as its own dedicated mapping,
    /// rather than deriving it from list order, means a developer who
    /// doesn't know (or care about) the list at all still gets the same
    /// low-stakes safety net Ctrl+C always provided.
    fn decline_outcome(&self) -> Option<DecisionOutcome> {
        match self.pending_front() {
            PendingFront::Approval(_) => Some(DecisionOutcome::Approve(false)),
            PendingFront::Prompt(pending) => match &pending.payload {
                PromptPayload::Tool { .. } => Some(DecisionOutcome::Prompt(PromptResponse::Tool { decision: Decision::Deny, tier: ToolTier::Once })),
                PromptPayload::ContextFile { .. } => Some(DecisionOutcome::Prompt(PromptResponse::ContextFile { approve: false, tier: None })),
                PromptPayload::Edit { .. } => None,
            },
            PendingFront::None => None,
        }
    }

    /// The single source of truth for "which pending request is currently
    /// interactive" — see `PendingFront`'s own doc comment for why this
    /// exists instead of each caller re-checking `pending_approvals.front()`
    /// independently.
    pub fn pending_front(&self) -> PendingFront<'_> {
        if let Some(approval) = self.pending_approvals.front() {
            PendingFront::Approval(approval)
        } else if let Some(prompt) = self.pending_prompts.front() {
            PendingFront::Prompt(prompt)
        } else {
            PendingFront::None
        }
    }

    /// Navigates/resolves the decision panel's numbered list — replaces the
    /// old per-payload letter-shortcut handling (`y`/`n`, `o`/`s`/`p`/`a` +
    /// Shift variants) per explicit developer request: "make sure the
    /// approval options appear as a list and not some weird keyboard
    /// shortcuts... key bindings for 1-3 or selecting with arrow keys and
    /// pressing enter are valid inputs." Up/Down move `decision_selected`
    /// (clamped, not wrapping); Enter confirms whichever option is
    /// currently selected; a digit key `1`-`9` jumps to and immediately
    /// confirms that option directly, without needing Enter first; any
    /// other key is silently dropped — no typing ahead, same as before.
    fn handle_decision_key(&mut self, key: KeyEvent) {
        let options = self.decision_options();
        if options.is_empty() {
            return;
        }
        // Ctrl+C always resolves as the safe decline, regardless of cursor
        // position — see `decline_outcome`'s own doc comment.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(outcome) = self.decline_outcome() {
                self.resolve_decision(outcome);
            }
            return;
        }
        match key.code {
            KeyCode::Up => self.decision_selected = self.decision_selected.saturating_sub(1),
            KeyCode::Down => self.decision_selected = (self.decision_selected + 1).min(options.len() - 1),
            KeyCode::Enter => {
                let idx = self.decision_selected.min(options.len() - 1);
                self.resolve_decision(options.into_iter().nth(idx).expect("idx clamped to options.len() - 1 above").outcome);
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let idx = (c as u8 - b'1') as usize;
                if let Some(opt) = options.into_iter().nth(idx) {
                    self.resolve_decision(opt.outcome);
                }
            }
            _ => {} // any other key is silently dropped — no typing ahead
        }
    }

    /// Pops whichever queue `outcome` came from, records the resolution on
    /// that call's own historical log entry, and sends the matching
    /// `Command` — the shared tail both `handle_decision_key` (Enter/digit/
    /// Ctrl+C) paths resolve through, so there's exactly one place that
    /// does this bookkeeping. Resets `decision_selected` since whatever
    /// becomes the new front (the next queued item, or nothing) needs its
    /// own list to start unselected at the top — see the field's own doc
    /// comment on `App`.
    fn resolve_decision(&mut self, outcome: DecisionOutcome) {
        self.decision_selected = 0;
        match outcome {
            DecisionOutcome::Approve(decision) => {
                // Only ever the front of the queue — see `pending_approvals`'
                // own doc comment. Popping it is what makes the next queued
                // approval (if any) interactive on the very next keystroke.
                let Some(pending) = self.pending_approvals.pop_front() else { return };
                for entry in self.log.iter_mut() {
                    if let LogEntry::ApprovalCard { call_id, resolution, .. } = entry {
                        if *call_id == pending.call_id {
                            *resolution = Some(decision);
                        }
                    }
                }
                self.outbox.push(if decision { Command::ApproveTool { call_id: pending.call_id } } else { Command::DenyTool { call_id: pending.call_id } });
            }
            DecisionOutcome::Prompt(response) => {
                let Some(pending) = self.pending_prompts.pop_front() else { return };
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
        assert!(!app.pending_approvals.is_empty());
    }

    /// The decision panel is a numbered list now (`App::decision_options`),
    /// not raw letter shortcuts — per explicit developer request: "make sure
    /// the approval options appear as a list... key bindings for 1-3 or
    /// selecting with arrow keys and pressing enter are valid inputs."
    /// Enter confirms whichever option is currently selected; the list
    /// starts on its first (least consequential) option, "Approve," by
    /// default.
    #[test]
    fn approval_card_enter_confirms_the_default_first_option() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        assert_eq!(app.decision_options().first().map(|o| o.label.as_str()), Some("Approve"));
        app.handle_key(press(KeyCode::Enter));
        assert!(app.pending_approvals.is_empty());
        assert_eq!(app.outbox, vec![Command::ApproveTool { call_id: "c1".into() }]);
    }

    #[test]
    fn approval_card_digit_2_denies_directly_without_enter() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        app.handle_key(press(KeyCode::Char('2')));
        assert_eq!(app.outbox, vec![Command::DenyTool { call_id: "c1".into() }]);
    }

    #[test]
    fn approval_card_arrow_down_then_enter_denies() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        app.handle_key(press(KeyCode::Down));
        assert_eq!(app.decision_selected, 1);
        app.handle_key(press(KeyCode::Enter));
        assert_eq!(app.outbox, vec![Command::DenyTool { call_id: "c1".into() }]);
    }

    /// Down at the list's last option (and Up at its first) must clamp, not
    /// wrap — an accidental extra Down keypress shouldn't silently jump the
    /// cursor back onto "Approve" for a two-option list.
    #[test]
    fn approval_card_arrow_navigation_clamps_at_the_list_ends() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        app.handle_key(press(KeyCode::Down));
        app.handle_key(press(KeyCode::Down));
        assert_eq!(app.decision_selected, 1, "Down must clamp at the last option, not wrap");
        app.handle_key(press(KeyCode::Up));
        app.handle_key(press(KeyCode::Up));
        assert_eq!(app.decision_selected, 0, "Up must clamp at the first option, not wrap");
    }

    #[test]
    fn approval_card_ignores_unrecognized_keys_no_typing_ahead() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        app.handle_key(press(KeyCode::Char('x')));
        app.handle_key(press(KeyCode::Char('0'))); // the list is 1-indexed — no option 0
        assert!(!app.pending_approvals.is_empty(), "an unrecognized key must not resolve the card");
        assert!(app.outbox.is_empty());
    }

    /// Regression test: parallel tool use can dispatch several Edit calls in
    /// one step (`Agent::dispatch_tools` drives them concurrently via
    /// `future::join_all`), each independently requesting approval. A second
    /// `ToolApprovalRequested` arriving while the first was still unresolved
    /// used to silently overwrite `pending_approval` (a single `Option`),
    /// leaving the first call's approval channel stuck forever with no key
    /// able to reach it — reported live as "the LLM requests multiple diffs
    /// ... and the user can only approve one thing". Both must stay
    /// individually resolvable, front of the queue first.
    #[test]
    fn two_pending_approvals_are_queued_not_overwritten_and_resolve_in_order() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff-1".into() });
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c2".into(), diff: "diff-2".into() });
        assert_eq!(app.pending_approvals.len(), 2, "the second request must be queued, not overwrite the first");

        app.handle_key(press(KeyCode::Char('1'))); // option 1: Approve
        assert_eq!(app.outbox, vec![Command::ApproveTool { call_id: "c1".into() }], "the first (front of queue) call must resolve first");
        assert_eq!(app.pending_approvals.len(), 1, "the second request must still be pending and resolvable after the first");

        app.handle_key(press(KeyCode::Char('2'))); // option 2: Deny
        assert_eq!(app.outbox, vec![Command::ApproveTool { call_id: "c1".into() }, Command::DenyTool { call_id: "c2".into() }]);
        assert!(app.pending_approvals.is_empty());

        // Both cards' own resolutions in the log must be independently
        // recorded, not just whichever call_id happened to be tracked.
        let resolutions: Vec<(String, Option<bool>)> = app
            .log
            .iter()
            .filter_map(|e| match e {
                LogEntry::ApprovalCard { call_id, resolution, .. } => Some((call_id.clone(), *resolution)),
                _ => None,
            })
            .collect();
        assert_eq!(resolutions, vec![("c1".to_string(), Some(true)), ("c2".to_string(), Some(false))]);
    }

    /// Regression/consolidation test for a rust-skills audit finding: an
    /// approval must take interactive priority over an already-pending
    /// prompt (the same "approvals before prompts" rule `handle_key`,
    /// `decision_options`, `decline_outcome`, and `ui::decision_panel_lines`
    /// all now read from the single `App::pending_front` accessor, instead
    /// of each independently re-checking `pending_approvals.front()`).
    /// Exercises the outcome through public behavior — the resolved
    /// `Command` and which queue empties — rather than reaching into
    /// `pending_front()` directly, so this stays a behavioral guarantee, not
    /// a test of the accessor's own plumbing.
    #[test]
    fn a_pending_approval_takes_priority_over_an_already_pending_prompt() {
        let mut app = app();
        let payload = serde_json::to_value(PromptPayload::Tool { kind: "shell".into(), target: "git status".into() }).unwrap();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });

        // The approval must be what the very next keypress resolves, even
        // though the prompt arrived first.
        app.handle_key(press(KeyCode::Enter));
        assert_eq!(app.outbox, vec![Command::ApproveTool { call_id: "c1".into() }], "the approval, not the earlier-queued prompt, must be front-of-line");
        assert!(app.pending_approvals.is_empty());
        assert_eq!(app.pending_prompts.len(), 1, "the prompt must still be queued behind it, untouched");

        // Once the approval is out of the way, the prompt becomes current.
        app.handle_key(press(KeyCode::Char('3'))); // option 3: allow, project tier
        assert!(app.pending_prompts.is_empty());
        match app.outbox.last() {
            Some(Command::PromptResponse { payload, .. }) => {
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Project });
            }
            other => panic!("expected PromptResponse, got {other:?}"),
        }
    }

    #[test]
    fn ctrl_c_denies_a_pending_approval_card_instead_of_being_swallowed() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested { turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "diff".into() });
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.pending_approvals.is_empty(), "Ctrl+C must resolve a pending approval card, not get stuck");
        assert_eq!(app.outbox, vec![Command::DenyTool { call_id: "c1".into() }]);
    }

    #[test]
    fn ctrl_c_declines_a_pending_tool_prompt_instead_of_being_swallowed() {
        let mut app = app();
        let payload = serde_json::to_value(PromptPayload::Tool { kind: "shell".into(), target: "rm -rf /".into() }).unwrap();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.pending_prompts.is_empty(), "Ctrl+C must resolve a pending permission prompt, not get stuck");
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
        assert!(app.pending_prompts.is_empty());
        match app.outbox.last() {
            Some(Command::PromptResponse { payload, .. }) => {
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::ContextFile { approve: false, tier: None });
            }
            other => panic!("expected PromptResponse, got {other:?}"),
        }
    }

    #[test]
    fn permission_prompt_resolves_on_a_numbered_selection_and_records_resolution() {
        let mut app = app();
        let payload = serde_json::to_value(PromptPayload::Tool { kind: "shell".into(), target: "git status".into() }).unwrap();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });
        assert!(!app.pending_prompts.is_empty());
        assert_eq!(app.decision_options()[2].label, "Allow for this project");

        app.handle_key(press(KeyCode::Char('3'))); // option 3: allow, project tier
        assert!(app.pending_prompts.is_empty());
        match app.outbox.last() {
            Some(Command::PromptResponse { payload, .. }) => {
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Project });
            }
            other => panic!("expected PromptResponse, got {other:?}"),
        }
    }

    /// Same regression as `two_pending_approvals_are_queued_not_overwritten_
    /// and_resolve_in_order`, for the generic permission-prompt round trip —
    /// a second `PromptRequested` used to overwrite `pending_prompt`
    /// (a single `Option`) and strand the first request's channel forever.
    #[test]
    fn two_pending_prompts_are_queued_not_overwritten_and_resolve_in_order() {
        let mut app = app();
        let payload_1 = serde_json::to_value(PromptPayload::Tool { kind: "shell".into(), target: "git status".into() }).unwrap();
        let payload_2 = serde_json::to_value(PromptPayload::ContextFile { path: "AGENTS.md".into() }).unwrap();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload: payload_1 });
        app.apply_event(Event::PromptRequested { call_id: "call-2".into(), payload: payload_2 });
        assert_eq!(app.pending_prompts.len(), 2, "the second request must be queued, not overwrite the first");

        app.handle_key(press(KeyCode::Char('3'))); // call-1: Tool, option 3 = allow at project tier
        assert_eq!(app.pending_prompts.len(), 1, "the second request must still be pending and resolvable after the first");
        match &app.outbox[0] {
            Command::PromptResponse { call_id, payload } => {
                assert_eq!(call_id, "call-1");
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::Tool { decision: Decision::Allow, tier: ToolTier::Project });
            }
            other => panic!("expected PromptResponse, got {other:?}"),
        }

        assert_eq!(app.decision_options().last().map(|o| o.label.as_str()), Some("No"), "call-2 is a ContextFile prompt — option 3 is its decline");
        app.handle_key(press(KeyCode::Char('3'))); // call-2: ContextFile, option 3 = decline
        assert!(app.pending_prompts.is_empty());
        match &app.outbox[1] {
            Command::PromptResponse { call_id, payload } => {
                assert_eq!(call_id, "call-2");
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::ContextFile { approve: false, tier: None });
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
        assert_eq!(app.status.running_tools, vec![RunningTool { call_id: "c1".into(), name: "read".into() }]);

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
        assert!(app.pending_prompts.is_empty());
    }
}
