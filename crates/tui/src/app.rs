use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use aldwin_core::{Command, Event, LogRecord, StepId};
use aldwin_permissions::{Choice, Class, ContextFileTier, Engine, PromptPayload, PromptResponse, Rung};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};

use crate::log::{summarise, LogEntry, PromptResolution, ToolActivityEntry, ToolActivityStatus};
use crate::scroll::ScrollState;

const SUMMARY_MAX_LEN: usize = 80;

/// How long a second Ctrl+C still counts as "again" for the exit escape
/// hatch in `App::cancel_or_quit`, in `App::tick`s — `run.rs` advances that
/// counter every 120ms, so ~2 seconds. Measured in ticks rather than wall
/// clock so the behaviour stays deterministic and testable without a real
/// clock, the same reason `tick` itself is a counter (see its field doc).
const DOUBLE_CTRL_C_TICKS: u64 = 16;

/// Rows one wheel notch moves the transcript — the same three a terminal's
/// own alternate-scroll translation sends as cursor keys, so the two paths
/// into `ScrollState` can't disagree about how far a notch goes.
const WHEEL_ROWS: usize = 3;

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
/// label (rendered as `"{n}  {label}"` by `ui.rs`), a `detail` saying what
/// choosing it concretely does, plus the outcome selecting it produces.
/// Built fresh from whichever request is at the front of the queue
/// (`App::decision_options`) on every draw/keypress, so rendering and
/// resolution can never disagree about what option N means.
///
/// Both fields answer the same direct developer feedback on the panel —
/// "permissions are not clear, are we approving the tool? are we approving
/// the directory? what are we concretely doing" — against a list whose
/// labels ("Allow for this project") named a *tier* and nothing else: they
/// said neither how long the answer lasts nor where, if anywhere, it gets
/// written. The two do it in different shapes, because the design system
/// draws two controls (ADR 0003): a permission row is `5a`'s **sentence**,
/// which states its own rule and leaves `detail` empty, while first run's
/// catalogue rows are `5c`'s name + detail pair.
pub struct DecisionOption {
    pub label:   String,
    pub detail:  String,
    /// Byte range within `label` covering the grant pattern the sentence
    /// quotes — `5a` draws "the matched pattern one step quieter" than the
    /// words around it. `None` on every row that quotes no pattern: the
    /// `once` and `deny` rows, both approval rows, and every `5c`-shaped
    /// pair. Carried as a range rather than pre-split spans so the label
    /// stays one readable string for tests and for `decline_outcome`.
    pub pattern: Option<std::ops::Range<usize>>,
    pub outcome: DecisionOutcome,
}

/// The permanent record one answered prompt leaves in the log — what the
/// developer chose, said the way they chose it rather than the way it goes
/// over the wire.
///
/// Deliberately derived from the `PromptResponse` rather than copied off
/// the `DecisionOption` the developer picked: Ctrl+C resolves through
/// `decline_outcome`, which builds a response directly and has no option to
/// copy a label from, so sourcing it from the option would have left that
/// one path — the one a developer under time pressure is most likely to
/// take — with nothing to record. Phrasing follows `readme.md`'s Content
/// Fundamentals: lowercase, past tense, no trailing period, since this
/// renders as a right-flush result summary beside a tool line.
fn describe_response(response: &PromptResponse) -> PromptResolution {
    match response {
        PromptResponse::Tool { choice } | PromptResponse::WriteAttempt { choice } => PromptResolution {
            allowed: choice.is_allow(),
            label:   match choice {
                Choice::AllowOnce => "allowed once",
                Choice::AllowSession => "allowed for this session",
                Choice::AllowProject => "allowed for this project",
                Choice::AllowEverywhere => "always allowed",
                Choice::DenyOnce => "denied",
                Choice::DenySession => "denied for this session",
                Choice::DenyProject => "denied for this project",
                Choice::NeverAllow => "never allowed",
            }
            .into(),
        },
        PromptResponse::ContextFile { approve: false, .. } => PromptResolution { allowed: false, label: "not injected".into() },
        PromptResponse::ContextFile { tier, .. } => PromptResolution {
            allowed: true,
            label:   match tier {
                Some(ContextFileTier::Session) => "injected for this session",
                Some(ContextFileTier::Project) => "injected for this project",
                None => "injected",
            }
            .into(),
        },
    }
}

/// The eight rows of ADR 0004 §8, for one program and the class of the call
/// that raised the prompt.
///
/// There is no pattern to compute here, and that absence is the change: a
/// grant is a program and a class, so every row already knows its own rule
/// from the two things it was handed. The old model needed
/// `broad_pattern`/`directory_glob`/`program_glob` to guess how wide a glob
/// to write from a target string, and needed a `Tab` toggle so the developer
/// could correct the guess.
fn tool_options(program: &str, class: Class) -> Vec<DecisionOption> {
    Choice::ORDER
        .iter()
        .map(|&choice| {
            let label = choice.sentence(program, class);
            // `5a` draws the quoted rule one step quieter than the sentence
            // around it. The range is computed here, where the sentence is
            // assembled, so nothing downstream has to re-find a substring
            // that may legitimately repeat.
            let quoted = choice
                .rule(program, class)
                .and_then(|rule| label.find(&rule).map(|at| at..at + rule.len()));
            DecisionOption {
                label,
                detail: String::new(),
                pattern: quoted,
                outcome: DecisionOutcome::Prompt(PromptResponse::Tool { choice }),
            }
        })
        .collect()
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
    /// This build's release version and the commit it came from — read
    /// once here rather than by the render layer, which used to reach for
    /// `env!` mid-draw. Facts about the session belong with the rest of
    /// the session's state: it keeps `ui` a pure function of `App` (a
    /// render test can pin them instead of inheriting whatever the build
    /// happened to embed), and it puts them beside `model_name`, which is
    /// the same kind of fact and was already here.
    pub version:       String,
    pub commit:        String,
    /// The working directory, `~`-shortened like a shell prompt — the top
    /// bar's own left-group fact. Same reasoning as `version`/`commit`,
    /// and more so: this one varies by machine, so a render that read it
    /// directly could not be pinned by a test at all.
    pub cwd:           Option<String>,
    pub turn:          Option<u64>,
    pub step:          Option<u64>,
    pub running_tools: Vec<RunningTool>,
    /// The rung in force here, or `None` when neither file states one.
    ///
    /// Not a set of per-tool states any more. Under ADR 0004 what a program
    /// may do depends on the program and the class, so "is `shell` allowed"
    /// stopped having an answer — and the row that used to print
    /// `read:deny shell:deny edit:deny` was three answers to a question the
    /// model no longer asks. What a scope actually carries is one rung
    /// (§6), and that is what a developer glances at this bar for.
    pub access:        Option<Rung>,
}

impl StatusInfo {
    fn refresh_permissions(&mut self, engine: &Engine) {
        self.access = engine.effective_rung();
    }
}

/// The session's working directory, `~`-shortened like a shell prompt.
/// `None` only if the process's cwd genuinely can't be read — not worth a
/// placeholder for a case this rare.
pub(crate) fn current_dir_display() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return Some(cwd.display().to_string());
    };
    if cwd == home {
        return Some("~".to_string());
    }
    match cwd.strip_prefix(&home) {
        Ok(rest) if !rest.as_os_str().is_empty() => Some(format!("~/{}", rest.display())),
        _ => Some(cwd.display().to_string()),
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
    /// The composer's real text-column width, cached by `ui::chrome`'s own
    /// draw exactly as `render_width` is — the draft wraps to it, so Up and
    /// Down have to navigate by it, and key handling runs between draws
    /// with no render access of its own. Off by at most one stale frame,
    /// on a resize, until the next draw corrects it.
    pub composer_width:    u16,
    /// First visual row of the draft the composer band is showing. A draft
    /// taller than [`crate::ui::COMPOSER_MAX_ROWS`] scrolls inside its band
    /// rather than growing without bound — a pasted file would otherwise
    /// take the whole frame and leave no transcript at all. Maintained by
    /// the composer's draw, which is the only place that knows both the
    /// band's height and where the caret is.
    pub composer_top:      usize,
    /// Queued, not a single slot — parallel tool use can dispatch several
    /// Edit calls in one step, each requesting approval independently (see
    /// `dispatch_tools`' `future::join_all` in aldwin-core), so more than
    /// one can be outstanding at once. A second `ToolApprovalRequested`
    /// arriving while the first was still an `Option` silently overwrote
    /// it — the first call's approval channel then hung forever with no
    /// key able to reach it, which stalled that dispatch future (and, via
    /// `join_all`, the whole step) until Ctrl+C cancelled the turn; the
    /// developer only ever saw the one card that happened to win the
    /// overwrite. The front of the queue is the one actually interactive
    /// (`handle_approval_key`/`handle_prompt_key` only ever act on it);
    /// resolving it pops the front and the next queued one becomes
    /// interactive automatically. See `aldwin-tui.md`'s Progress note.
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
    /// True from a submitted message until the turn it asks for either
    /// starts (`TurnStarted`) or is answered without one ever starting —
    /// which is what a locally-handled slash command does, acknowledging
    /// itself with a `Notice`/`HistoryCleared`/`ThemeChanged` instead. Only
    /// `cancel_or_quit` reads it; see its doc comment for why the pair of
    /// flags exists at all.
    pub awaiting_turn:     bool,
    /// `tick` at the most recent Ctrl+C that cancelled rather than quit —
    /// the anchor for `cancel_or_quit`'s double-press exit.
    last_cancel_tick:      Option<u64>,
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

    /// The provider catalogue the model picker offers, in display order,
    /// with each row's models on it — the display halves only (an id and a
    /// purpose), exactly as first run takes them. Empty unless the caller
    /// supplied one (`App::with_catalogue`), in which case bare `/model`
    /// stays a plain command and aldwin-cli reports on it as before.
    pub catalogue: Vec<crate::first_run::ProviderChoice>,
    /// The catalogue id of the provider this session is actually running
    /// on, when it is one the catalogue knows — what the picker opens on
    /// and marks as current. `None` for a hand-written endpoint, which is
    /// a real configuration and not an error.
    pub current_provider: Option<String>,
    /// What to *call* the provider this session runs on, which is not the
    /// same question as which catalogue row the picker opens on. A
    /// hand-written endpoint matches no row, but its kind is still declared
    /// in `provider.yaml` — so this is the catalogue id where there is one
    /// and the bare kind (`openai-compatible`) where there is not, and it is
    /// never unknown for a configured session. `None` only for an `App`
    /// nobody gave a session to: tests, and `examples/preview.rs`.
    pub provider_label: Option<String>,
    /// Open only while the picker is on screen: it takes every key and the
    /// bottom band draws it instead of the composer, the same way a pending
    /// decision does.
    pub picker: Option<crate::picker::ModelPicker>,
    /// Open only while the session picker is on screen, on the same terms as
    /// `picker` above: the two are mutually exclusive by construction, since
    /// each is opened by a submission and a submission cannot happen while
    /// either holds the band.
    pub resume: Option<crate::resume::ResumePicker>,
    /// The past sessions bare `/resume` offers, newest first — display
    /// halves handed in by aldwin-cli's bootstrap (`App::with_sessions`),
    /// exactly as the model catalogue is. Empty unless the caller supplied
    /// one, in which case bare `/resume` stays a plain command and the
    /// interceptor answers it with a notice.
    pub sessions: Vec<crate::resume::SessionChoice>,

    /// Which fixed color `Palette` this session renders with — resolved
    /// once from `tui.yaml`'s `theme` field (`Theme::from_config`) before
    /// the first draw and never changed afterward (see `palette.rs`'s
    /// module doc comment for why this lives on `App` rather than global
    /// state). Defaults to `Theme::Dark` via `App::new`; `run.rs` overrides
    /// it with `App::with_theme` once config is available.
    pub theme: crate::palette::Theme,

    /// The transcript's screen rows, cached one log entry at a time — see
    /// [`crate::ui::Transcript`]. Kept up to date by
    /// [`App::sync_transcript`], which every reader below goes through.
    transcript: crate::ui::Transcript,
}

impl App {
    pub fn new(model_name: String, permissions: Arc<Engine>) -> Self {
        let mut status = StatusInfo {
            model_name,
            version: crate::version::VERSION.to_string(),
            commit: crate::version::GIT_HASH.to_string(),
            cwd: current_dir_display(),
            turn: None,
            step: None,
            running_tools: vec![],
            access: None,
        };
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
            composer_width: 74,
            composer_top: 0,
            pending_approvals: VecDeque::new(),
            pending_prompts: VecDeque::new(),
            decision_selected: 0,
            status,
            should_quit: false,
            turn_active: false,
            awaiting_turn: false,
            last_cancel_tick: None,
            tick: 0,
            pending_tool_names: HashMap::new(),
            outbox: Vec::new(),
            catalogue: Vec::new(),
            current_provider: None,
            provider_label: None,
            picker: None,
            resume: None,
            sessions: Vec::new(),
            theme: crate::palette::Theme::default(),
            transcript: crate::ui::Transcript::default(),
        }
    }

    /// The past sessions bare `/resume` offers. Builder-style for the same
    /// reason `with_catalogue` is: only aldwin-cli's bootstrap can read a
    /// history directory, and every other caller wants the empty default.
    ///
    /// They arrive already rendered for display — aldwin-tui reads no files
    /// and parses no timestamps, the same rule that keeps the catalogue's
    /// endpoints and key variables out of this crate.
    pub fn with_sessions(mut self, sessions: Vec<crate::resume::SessionChoice>) -> Self {
        self.sessions = sessions;
        self
    }

    /// Builder-style, for the same reason `with_theme` is: the catalogue is
    /// something only aldwin-cli's bootstrap has, and every other caller
    /// (tests, `examples/preview.rs`) wants the same empty default it
    /// already had. Without one, bare `/model` is forwarded to the
    /// interceptor and reports where the developer stands, as it always did.
    ///
    /// `current_provider` is the catalogue id of the row the session is
    /// running on — the model half comes from `status.model_name`, which
    /// this same bootstrap already sets.
    pub fn with_catalogue(mut self, catalogue: Vec<crate::first_run::ProviderChoice>, current_provider: Option<String>) -> Self {
        self.catalogue = catalogue;
        self.current_provider = current_provider;
        self
    }

    /// The provider's display name, set alongside the catalogue because it
    /// answers a different question: `with_catalogue` says which row the
    /// picker opens on, this says what the resting screen calls the
    /// provider. They part company for a hand-written endpoint, which has a
    /// kind to name but no row to open on.
    pub fn with_provider_label(mut self, label: Option<String>) -> Self {
        self.provider_label = label;
        self
    }

    /// Builder-style override for `theme` — kept separate from `App::new`'s
    /// own parameter list rather than adding a parameter there, so the
    /// many existing `App::new(model_name, permissions)` call sites (tests,
    /// `examples/preview.rs`, `aldwin-cli`'s bootstrap) don't all need to
    /// thread a theme through just to get the same `Dark` default they
    /// already had. `run.rs` is the one real caller that overrides it.
    pub fn with_theme(mut self, theme: crate::palette::Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Advances the animation-frame counter — called by `run.rs` on a fixed
    /// timer so the "working"/"thinking" spinner animates independently of
    /// core events or keystrokes.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn push(&mut self, entry: LogEntry) {
        self.log.push(entry);
        let total = self.total_lines();
        self.scroll.on_content_grew(total);
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
    /// aldwin-tui.md's 2026-08-29 scrolling-fix Progress note). It is now
    /// exactly the transcript's own row count, since those rows *are* the
    /// screen rows — there is no longer a separate counting pass that could
    /// drift from the rendering one.
    pub fn total_lines(&mut self) -> usize {
        self.sync_transcript();
        self.transcript.len()
    }

    /// Blocks the last transcript sync rebuilt — the incremental-render
    /// guarantee, countable. Test-only: nothing in the app reads it, and the
    /// two tests that assert on it used to time a render loop instead.
    #[cfg(test)]
    pub(crate) fn blocks_rebuilt(&self) -> usize {
        self.transcript.rebuilt()
    }

    /// The screen rows to draw for a `count`-row viewport starting at
    /// `offset` — what `ui::draw` hands to the log panel, and all it ever
    /// needs: the viewport, not the conversation. Fewer than `count` rows
    /// when the viewport opens inside a turn break (see
    /// `Transcript::viewport`); the log panel's own bottom anchoring pads
    /// the difference.
    pub fn transcript_slice(&mut self, offset: usize, count: usize) -> Vec<ratatui::text::Line<'static>> {
        self.sync_transcript();
        self.transcript.viewport(offset, count)
    }

    /// Brings the row cache up to date with the log at the current render
    /// size. Every reader goes through here rather than through an
    /// invalidation flag someone has to remember to set: `Transcript::sync`
    /// compares each entry against the value its rows were built from, so
    /// "what changed" is answered by the data itself and there is no way to
    /// mutate the log and forget to say so.
    ///
    /// Takes the cache out and puts it back because it needs `&App` to read
    /// the log and `&mut` the cache at once; moving a handful of `Vec`
    /// headers is free next to what it saves.
    fn sync_transcript(&mut self) {
        let mut transcript = std::mem::take(&mut self.transcript);
        transcript.sync(self, self.render_width, self.render_height);
        self.transcript = transcript;
    }

    fn active_step_calls(&mut self, step_id: StepId) -> Option<&mut Vec<ToolActivityEntry>> {
        self.log.iter_mut().rev().find_map(|e| match e {
            LogEntry::ToolActivity { step_id: sid, calls } if *sid == step_id => Some(calls),
            _ => None,
        })
    }

    /// One loaded record, as the entry the live path would have produced.
    ///
    /// The two vocabularies do not line up one-to-one, because `LogEntry` is
    /// built from *streaming* events and a record is what was committed
    /// afterwards. Three places that matters:
    ///
    /// * `AssistantMessage` is one entry where the live path accumulated many
    ///   `TextDelta`s into one — so it merges into a trailing `AssistantText`
    ///   exactly as the deltas did, which is what keeps two steps' prose from
    ///   splitting into two blocks the live session would have shown as one.
    /// * A tool call arrives already finished, so it goes straight to
    ///   `Completed` rather than passing through `Running`.
    /// * `TurnStarted` and `StepBoundary` produce nothing. They set live
    ///   status fields (`status.turn`, the spinner) that describe work in
    ///   flight, and nothing is in flight in a transcript.
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
            LogRecord::ToolUse { step_id, call, .. } => {
                let entry = ToolActivityEntry {
                    call_id: call.id,
                    name:    call.name,
                    // Overwritten by the matching `ToolResult` below. It can
                    // only survive as `Running` if the transcript ended
                    // mid-turn, and `load` truncates those away before they
                    // ever reach here.
                    status:  ToolActivityStatus::Running,
                };
                match self.active_step_calls(step_id) {
                    Some(calls) => calls.push(entry),
                    None => self.log.push(LogEntry::ToolActivity { step_id, calls: vec![entry] }),
                }
            }
            LogRecord::ToolResult { step_id, result, .. } => {
                let summary = summarise(&result.content, SUMMARY_MAX_LEN);
                if let Some(calls) = self.active_step_calls(step_id) {
                    if let Some(call) = calls.iter_mut().find(|c| c.call_id == result.call_id) {
                        call.status = ToolActivityStatus::Completed { is_error: result.is_error, summary };
                    }
                }
            }
            LogRecord::TurnEnded { reason, .. } => self.log.push(LogEntry::TurnEnded { reason: reason.into() }),
            // Thinking is carried in the transcript for the wire's sake
            // (ADR 0006), not for the reader's: the log shows what the agent
            // said, not what it thought. Drawing it needs a treatment the
            // design system does not specify yet — open-tasks entry 24.
            LogRecord::Thinking { .. } | LogRecord::RedactedThinking { .. } => {}
            LogRecord::TurnStarted { .. } | LogRecord::StepBoundary { .. } => {}
        }
    }

    pub fn apply_event(&mut self, event: Event) {
        match event {
            Event::TurnStarted { turn_id } => {
                self.status.turn = Some(turn_id.0);
                self.status.step = None;
                self.turn_active = true;
                self.awaiting_turn = false;
            }
            Event::TextDelta { text, .. } => {
                if let Some(LogEntry::AssistantText { text: buf }) = self.log.last_mut() {
                    buf.push_str(&text);
                } else {
                    self.push(LogEntry::AssistantText { text });
                }
            }
            Event::ThinkingStart { .. } => self.thinking = true,
            // Keeps the indicator alive across a long block without drawing
            // the text — see the `replay` arm above for why it isn't shown.
            Event::ThinkingDelta { .. } => self.thinking = true,
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
            // `thinking` is cleared here, not only on ThinkingEnd: a stream
            // that dies mid-thinking (transport error, idle timeout) never
            // sends the closing event, and a spinner still reading "thinking"
            // after the turn is over describes work that isn't happening.
            // Reachable on any provider, but routine on a reasoning model —
            // Lumo's `lumo-max` thinks on nearly every turn.
            Event::TurnEnded { reason, .. } => {
                self.status.running_tools.clear();
                self.turn_active = false;
                self.awaiting_turn = false;
                self.thinking = false;
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
            // A Notice is how a locally-handled slash command answers — no
            // turn is coming, so whatever `submit` was waiting for has
            // arrived (see `cancel_or_quit`). Harmless mid-turn: `turn_active`
            // is what says a turn is running then, and this doesn't touch it.
            Event::Notice { message } => {
                self.awaiting_turn = false;
                self.push(LogEntry::Notice { message });
            }
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
                self.awaiting_turn = false;
            }
            // `/resume` — the counterpart of `HistoryCleared` above, and it
            // starts by doing exactly what that arm does: core's
            // ConversationLog has been replaced wholesale, so the rendered
            // log has to be too, not appended to.
            //
            // What comes back is the *conversation*, not the session. Tool
            // activity replays as completed rows; approval cards and
            // permission prompts do not come back at all, because they exist
            // only as `LogEntry` and never as `LogRecord` (see
            // aldwin-history.md's Decision). That is a chosen loss: the
            // decisions a resumed session needs are re-asked, and default-deny
            // is not weakened by a card being redrawn.
            Event::HistoryLoaded { records } => {
                self.log.clear();
                self.scroll = ScrollState::default();
                self.status.turn = None;
                self.status.step = None;
                self.thinking = false;
                self.turn_active = false;
                self.awaiting_turn = false;
                for record in records {
                    self.replay(record);
                }
                self.sync_transcript();
            }
            // `/theme light|dark` — the interceptor already persisted this
            // to `tui.yaml` (see `Event::ThemeChanged`'s own doc comment in
            // aldwin-core); reparsing here rather than trusting the raw
            // string directly keeps the "anything unrecognized means dark"
            // fallback in exactly one place (`Theme::from_config`), the same
            // rule aldwin-cli's bootstrap already applies at startup.
            // Nothing else needs updating — `App::theme` is read fresh by
            // `ui::draw` on every frame, so the very next redraw already
            // reflects it.
            Event::ThemeChanged { theme } => {
                self.awaiting_turn = false;
                self.theme = crate::palette::Theme::from_config(Some(&theme));
            }
            // `/model` — the interceptor rebuilt the session's client before
            // sending this (see `Event::ModelChanged` in aldwin-core), so
            // by the time it lands the next turn really will run on this
            // model. Both bars read `status.model_name` on every draw, so
            // they follow on the very next redraw; `current_provider` is
            // updated in step because it is the other half of the same fact
            // — it decides which row the picker opens on.
            Event::ModelChanged { provider, model } => {
                self.awaiting_turn = false;
                self.status.model_name = model;
                self.current_provider = provider;
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

        // After decisions, never before: a permission prompt is the agent
        // waiting on an answer, and it takes the band (and the keys) even
        // with the picker open underneath. The picker is still there when
        // the prompt resolves.
        if self.resume.is_some() {
            self.handle_resume_key(key);
            return;
        }
        if self.picker.is_some() {
            self.handle_picker_key(key);
            return;
        }

        match (key.code, key.modifiers) {
            // A new line in the draft, three ways, because no one of them
            // reaches every terminal. Shift+Enter is what a developer
            // reaches for, but a terminal only reports it distinctly under
            // the Kitty keyboard protocol — `run.rs` asks for that at
            // startup where it is supported, and where it isn't, Shift+Enter
            // is literally indistinguishable from Enter on the wire. Alt+
            // Enter is what iTerm2 and Windows Terminal send for
            // Option/Alt+Return without any protocol extension, and Ctrl+J
            // (a bare linefeed) gets through everywhere else.
            (KeyCode::Enter, m) if m.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) => self.insert_char('\n'),
            (KeyCode::Enter, _) => self.submit(),
            (KeyCode::Char('j'), m) if m.contains(KeyModifiers::CONTROL) => self.insert_char('\n'),
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => self.cancel_or_quit(),
            (KeyCode::Backspace, _) => self.backspace(),
            (KeyCode::Delete, _) => self.delete_forward(),
            (KeyCode::Left, _) => self.cursor = self.cursor.saturating_sub(1),
            (KeyCode::Right, _) => self.cursor = (self.cursor + 1).min(self.input.chars().count()),
            // Scoped to the line the caret is on, not to the whole draft —
            // a draft is now routinely many lines (Shift+Enter, or a
            // pasted block), and Home jumping to the top of a pasted file
            // is not what either key means in any other editor. With no
            // draft to move within, End keeps its other job: putting the
            // transcript back on the live end of the conversation.
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
            // Within a multi-line draft, Up/Down move the cursor between its
            // rows first; only once there's no further row to move to
            // (a single-row draft, or already at the draft's first/last
            // row) do they fall through to scrolling the log. Previously
            // this — and a separate set of vim-style j/k/G bindings — used
            // "only when the input is empty" as the guard, which silently
            // swallowed the first keystroke of any message starting with
            // j, k, or a capital G instead of inserting it (the vim
            // bindings are gone outright: PageUp/PageDown/Home/End already
            // cover keyboard scrolling without that ambiguity).
            //
            // This is also the wheel's path into the log: `run.rs` leaves
            // the mouse to the terminal and asks it to translate notches
            // into cursor keys instead (see `ALTERNATE_SCROLL` there), so
            // every notch arrives here as an ordinary Up/Down and moves
            // exactly what the key itself would.
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
            (KeyCode::Char(c), _) => self.insert_char(c),
            _ => {}
        }
    }

    /// Scrolls the log for a wheel event, for the case where something has
    /// mouse reporting on after all.
    ///
    /// `run.rs` deliberately does not enable capture — the terminal keeps
    /// the mouse so that click-drag stays native text selection — so in an
    /// ordinary session these never arrive and the wheel comes through
    /// `handle_key` as cursor keys instead. But a session can inherit
    /// reporting from a mode a previous program left on, and a multiplexer
    /// can be configured to forward it; a dropped wheel event then reads as
    /// scrolling being broken. Three rows a notch is what a terminal's own
    /// alternate-scroll translation sends, so both paths move the same
    /// distance.
    pub fn handle_mouse(&mut self, event: MouseEvent) {
        if !self.pending_approvals.is_empty() || !self.pending_prompts.is_empty() || self.picker.is_some() || self.resume.is_some() {
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

    /// A bracketed paste — the clipboard's whole contents at once, newlines
    /// and all, dropped in at the caret.
    ///
    /// Without this the terminal sends a paste as if it had been *typed*,
    /// so every newline in it arrived as `KeyCode::Enter` and submitted the
    /// line above it: pasting a five-line block sent five separate
    /// messages, the first one being whatever had been pasted before the
    /// first newline. `run.rs` turns on bracketed paste so the terminal
    /// brackets the block and delivers it here instead.
    ///
    /// Ignored while a decision or the picker holds the band, matching
    /// `handle_key`: there is no composer on screen to paste into, and the
    /// placeholder says so.
    pub fn paste(&mut self, text: &str) {
        if !self.pending_approvals.is_empty() || !self.pending_prompts.is_empty() || self.picker.is_some() || self.resume.is_some() {
            return;
        }
        self.insert_str(&crate::draft::sanitize(text));
    }

    /// Moves the cursor to the visual row `delta` away, holding its display
    /// column where the target row is wide enough. Returns `false` (leaving
    /// the cursor untouched) when there's no such row — a single-row draft,
    /// or already at its first/last row — so callers can fall through to
    /// scrolling the log instead.
    ///
    /// By *wrapped* row, not source line: a pasted paragraph is one source
    /// line and several rows, and moving by source line there skipped the
    /// rows in between and put the caret somewhere the developer could not
    /// see it having moved to. `composer_width` is the column the composer
    /// last drew at.
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

    /// Inserts a whole block at the caret in one go. Not a loop over
    /// `insert_char`: that walks the draft from the start to translate the
    /// caret's character index into a byte offset, which is quadratic in
    /// the size of the paste, and a pasted file is exactly the case this
    /// exists for.
    fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let byte_idx = self.byte_at(self.cursor);
        self.input.insert_str(byte_idx, text);
        self.cursor += text.chars().count();
    }

    /// The byte offset of character `index`, or the draft's length past the
    /// end — `App::cursor` counts characters and `String` indexes bytes.
    fn byte_at(&self, index: usize) -> usize {
        self.input.char_indices().nth(index).map_or(self.input.len(), |(i, _)| i)
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

    /// Bare `/model`, with no argument — the one submission this side reads
    /// rather than forwards. The command is still aldwin-cli's: the picker
    /// answers by *typing* it (`/model provider/model`) once both halves are
    /// chosen, so the interceptor remains the only thing that decides which
    /// `provider.yaml` a choice lands in. What is intercepted here is how
    /// the question is asked, not what the answer does.
    ///
    /// An argument (`/model anthropic/claude-opus-5`) is left alone — a
    /// developer who names a model is not asking to be shown a list — and so
    /// is the bare form when no catalogue was handed in, which then reports
    /// where the developer stands exactly as before.
    const PICKER_COMMAND: &'static str = "/model";

    fn open_picker(&mut self) -> bool {
        let Some(picker) =
            crate::picker::ModelPicker::open(self.catalogue.clone(), self.current_provider.as_deref(), &self.status.model_name)
        else {
            return false;
        };
        self.picker = Some(picker);
        true
    }

    /// Bare `/resume`, on exactly the terms `PICKER_COMMAND` documents: the
    /// list is how the question is asked, and committing types
    /// `/resume <id>` so aldwin-cli's interceptor stays the one thing that
    /// knows what resuming does.
    ///
    /// With no sessions to offer — a project the harness has never recorded
    /// one in — this returns false and the bare command is forwarded, which
    /// the interceptor answers by saying so. An empty panel is not a way to
    /// tell the developer their history is empty.
    const RESUME_COMMAND: &'static str = "/resume";

    fn open_resume(&mut self) -> bool {
        let Some(picker) = crate::resume::ResumePicker::open(self.sessions.clone()) else {
            return false;
        };
        self.resume = Some(picker);
        true
    }

    fn handle_resume_key(&mut self, key: KeyEvent) {
        let Some(picker) = self.resume.as_mut() else { return };
        match picker.handle_key(key.code, key.modifiers).into_picker_outcome() {
            crate::picker::PickerOutcome::Stay => {}
            crate::picker::PickerOutcome::Close => self.resume = None,
            // `provider` carries the session id — see `ResumeOutcome`.
            crate::picker::PickerOutcome::Chosen { provider: id, .. } => {
                self.resume = None;
                self.submit_text(format!("{} {id}", Self::RESUME_COMMAND));
            }
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) {
        let Some(picker) = self.picker.as_mut() else { return };
        match picker.handle_key(key.code, key.modifiers) {
            crate::picker::PickerOutcome::Stay => {}
            crate::picker::PickerOutcome::Close => self.picker = None,
            crate::picker::PickerOutcome::Chosen { provider, model } => {
                self.picker = None;
                // Exactly what the developer would have typed, submitted the
                // way they would have submitted it — including the log entry,
                // so the transcript records the choice above the notice that
                // answers it.
                let argument = if model.is_empty() { provider } else { format!("{provider}/{model}") };
                self.submit_text(format!("{} {argument}", Self::PICKER_COMMAND));
            }
        }
    }

    fn submit(&mut self) {
        if self.input.trim().is_empty() {
            return;
        }
        if self.input.trim() == Self::PICKER_COMMAND && self.open_picker() {
            self.input.clear();
            self.cursor = 0;
            return;
        }
        if self.input.trim() == Self::RESUME_COMMAND && self.open_resume() {
            self.input.clear();
            self.cursor = 0;
            return;
        }
        let text = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.submit_text(text);
    }

    /// The tail every submission shares, typed or picked.
    fn submit_text(&mut self, text: String) {
        // Set for every submission, slash command included: this side can't
        // know which ones aldwin-cli's interceptor will handle itself, and
        // it doesn't need to — whatever the interceptor sends back (a
        // `Notice`, `HistoryCleared`, `ThemeChanged`) clears the flag just
        // as `TurnStarted` does. See `cancel_or_quit`.
        self.awaiting_turn = true;
        self.push(LogEntry::UserMessage { text: text.clone() });
        self.outbox.push(Command::Submit { text });
    }

    /// Ctrl+C: cancel the running turn if there is one, otherwise exit. A
    /// second Ctrl+C within `DOUBLE_CTRL_C_TICKS` always exits, whatever the
    /// state says.
    ///
    /// "Is a turn running" is read from `turn_active`/`awaiting_turn` — the
    /// flags `apply_event`/`submit` maintain from the events core actually
    /// sends. It used to be *inferred* by scanning the log backwards for the
    /// most recent `UserMessage` (meaning "running") or `TurnEnded` (meaning
    /// "finished"), and that is the reported "Ctrl+C after /theme appears to
    /// be broken" bug: a slash command is submitted like any other message,
    /// so `submit` pushes a `UserMessage` for it, but aldwin-cli's
    /// interceptor handles `/theme` (and `/help`, `/reload-config`, and any
    /// unknown command) entirely on its own — the core never sees it, no
    /// turn ever starts, and no `TurnEnded` is ever appended. The scan then
    /// found that `UserMessage` forever after and answered "a turn is
    /// running" to every subsequent Ctrl+C, so the key sent `Command::Cancel`
    /// into a session with nothing to cancel and the developer could never
    /// exit with it again. The flags can't drift that way: nothing sets them
    /// but the events that genuinely bracket a turn.
    ///
    /// `awaiting_turn` covers the real gap the log scan was reaching for —
    /// the stretch between submitting a message and `TurnStarted` arriving,
    /// when a turn is coming but isn't running yet; a Ctrl+C there must
    /// still cancel rather than quit out from under the request. The
    /// double-press escape hatch is the backstop for every remaining way
    /// "busy" could be wrong: if the flags ever say busy when nothing is,
    /// pressing again still exits, so the developer is never trapped in the
    /// session the way this bug trapped them.
    fn cancel_or_quit(&mut self) {
        let busy = self.turn_active || self.awaiting_turn;
        let repeat = self.last_cancel_tick.is_some_and(|t| self.tick.saturating_sub(t) <= DOUBLE_CTRL_C_TICKS);
        if busy && !repeat {
            self.last_cancel_tick = Some(self.tick);
            self.outbox.push(Command::Cancel);
            self.push(LogEntry::Notice { message: "cancelling — press ctrl+c again to exit".into() });
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
                DecisionOption { label: "Approve".into(), detail: "write this edit to the file".into(), pattern: None, outcome: DecisionOutcome::Approve(true) },
                DecisionOption { label: "Deny".into(), detail: "nothing is written; the agent is told no".into(), pattern: None, outcome: DecisionOutcome::Approve(false) },
            ],
            PendingFront::Prompt(pending) => match &pending.payload {
                // ADR 0004 §8: four allow tiers and four deny tiers,
                // mirrored, in one vertical list. The symmetry is the point
                // — a deny is a lock (§7), and a lock the developer can only
                // reach by hand-editing a file is a lock they will not set.
                //
                // Each row is a sentence that states its own rule (ADR 0003
                // §1, which 0004 leaves standing), and the class is quoted
                // inside it: these rows are about this program's *reads* or
                // its *writes*, so a `git: read` grant survives a `git:
                // write` deny. Row 8 is the one deliberately blunter row —
                // the whole program, everywhere.
                PromptPayload::Tool { program, declared, .. } => {
                    tool_options(program, *declared)
                }
                // The second question of ADR 0004 §4. The call claimed to be
                // a read and could not complete with the project read-only;
                // nothing landed. The rows are the same eight, because the
                // answer persists in exactly the same way — what differs is
                // that the class is now `write`, which is what the call
                // turned out to be.
                PromptPayload::WriteAttempt { program, .. } => {
                    tool_options(program, Class::Write)
                }
                PromptPayload::ContextFile { .. } => vec![
                    DecisionOption {
                        label:   "Inject for this session".into(),
                        detail:  "until aldwin exits; nothing is saved".into(),
                        pattern: None,
                        outcome: DecisionOutcome::Prompt(PromptResponse::ContextFile { approve: true, tier: Some(ContextFileTier::Session) }),
                    },
                    DecisionOption {
                        label:   "Inject for this project".into(),
                        detail:  "saved to .aldwin/context_files.yaml".into(),
                        pattern: None,
                        outcome: DecisionOutcome::Prompt(PromptResponse::ContextFile { approve: true, tier: Some(ContextFileTier::Project) }),
                    },
                    DecisionOption {
                        label:   "Don't inject".into(),
                        detail:  "the agent never sees this file; asked again next time".into(),
                        pattern: None,
                        outcome: DecisionOutcome::Prompt(PromptResponse::ContextFile { approve: false, tier: None }),
                    },
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
    /// aldwin-tui.md's 2026-08-29 live-run fix and its Pitfall on requiring
    /// an unambiguous way out). Keeping this as its own dedicated mapping,
    /// rather than deriving it from list order, means a developer who
    /// doesn't know (or care about) the list at all still gets the same
    /// low-stakes safety net Ctrl+C always provided.
    fn decline_outcome(&self) -> Option<DecisionOutcome> {
        match self.pending_front() {
            PendingFront::Approval(_) => Some(DecisionOutcome::Approve(false)),
            PendingFront::Prompt(pending) => match &pending.payload {
                // `Once` never persists a pattern (see `Engine::
                // record_tool_decision`'s `Once` arm), so the exact target
                // is passed here purely for a well-formed `PromptResponse`,
                // not because it takes effect.
                PromptPayload::Tool { .. } => {
                    Some(DecisionOutcome::Prompt(PromptResponse::Tool { choice: Choice::DenyOnce }))
                }
                PromptPayload::WriteAttempt { .. } => {
                    Some(DecisionOutcome::Prompt(PromptResponse::WriteAttempt { choice: Choice::DenyOnce }))
                }
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
    ///
    /// `Tab` used to flip the whole list between the exact and broad grant
    /// patterns. ADR 0003 moved scope onto the rows themselves, so there is
    /// no mode left to toggle and the key is no longer bound here.
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
                let label = describe_response(&response);
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
    use aldwin_config::Config;
    use aldwin_core::{ToolCall, ToolResult, TurnEndReason, TurnId};
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

    // ── `/resume` ─────────────────────────────────────────────────────────

    fn sessions() -> Vec<crate::resume::SessionChoice> {
        vec![
            crate::resume::SessionChoice {
                id: "0000000020-7".into(), title: "newer".into(), when: "2026-09-20 18:11".into(), turns: 2,
            },
            crate::resume::SessionChoice {
                id: "0000000010-3".into(), title: "older".into(), when: "2026-09-19 09:02".into(), turns: 5,
            },
        ]
    }

    fn call() -> ToolCall {
        ToolCall { id: "c1".into(), name: "read".into(), input: serde_json::json!({ "path": "retry.rs" }) }
    }

    /// Step 5's verify: a resumed transcript must render through to the same
    /// entries the live session produced, not to a second, nearly-right
    /// shape that drifts from it.
    #[test]
    fn a_resumed_log_renders_identically_to_the_live_one() {
        let (turn_id, step_id) = (TurnId(1), StepId(1));
        let result = ToolResult { call_id: "c1".into(), content: "fn backoff() {}".into(), is_error: false };

        // The live path: streamed events, in the order core emits them.
        let mut live = app();
        live.submit_text("explain the retry logic".into());
        live.apply_event(Event::TurnStarted { turn_id });
        live.apply_event(Event::TextDelta { turn_id, step_id, text: "Reading ".into() });
        live.apply_event(Event::TextDelta { turn_id, step_id, text: "it now.".into() });
        live.apply_event(Event::ToolUseRequested { turn_id, step_id, call: call() });
        live.apply_event(Event::ToolDispatched { turn_id, step_id, call_id: "c1".into() });
        live.apply_event(Event::ToolCompleted { turn_id, step_id, result: result.clone() });
        live.apply_event(Event::TurnEnded { turn_id, reason: TurnEndReason::EndTurn });

        // The resumed path: the records that same turn committed.
        let mut resumed = app();
        resumed.apply_event(Event::HistoryLoaded {
            records: vec![
                LogRecord::TurnStarted { turn_id },
                LogRecord::UserMessage { turn_id, text: "explain the retry logic".into() },
                LogRecord::AssistantMessage { turn_id, step_id, text: "Reading it now.".into() },
                LogRecord::ToolUse { turn_id, step_id, call: call() },
                LogRecord::ToolResult { turn_id, step_id, result },
                LogRecord::TurnEnded { turn_id, reason: TurnEndReason::EndTurn },
            ],
        });

        assert_eq!(resumed.log, live.log);
    }

    /// The counterpart of `HistoryCleared`: core replaced its log wholesale,
    /// so the rendered one is replaced too rather than appended to.
    #[test]
    fn resuming_replaces_the_rendered_log_rather_than_appending_to_it() {
        let mut app = app();
        app.submit_text("from before".into());
        app.apply_event(Event::HistoryLoaded {
            records: vec![
                LogRecord::UserMessage { turn_id: TurnId(1), text: "from the transcript".into() },
                LogRecord::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::EndTurn },
            ],
        });

        assert!(
            !app.log.iter().any(|e| matches!(e, LogEntry::UserMessage { text } if text == "from before")),
            "nothing from the replaced session survives"
        );
        assert!(matches!(app.log.first(), Some(LogEntry::UserMessage { text }) if text == "from the transcript"));
        assert!(!app.awaiting_turn, "the submission that asked for this has been answered");
    }

    /// An approval card and a permission prompt are `LogEntry` only — see
    /// the `HistoryLoaded` arm. This pins the loss as chosen rather than
    /// letting it be rediscovered as a bug.
    #[test]
    fn a_resumed_transcript_carries_the_conversation_not_the_decisions() {
        let mut app = app();
        app.apply_event(Event::ToolApprovalRequested {
            turn_id: TurnId(1), step_id: StepId(1), call_id: "c1".into(), diff: "- a\n+ b".into(),
        });
        assert!(app.log.iter().any(|e| matches!(e, LogEntry::ApprovalCard { .. })));

        app.apply_event(Event::HistoryLoaded {
            records: vec![
                LogRecord::UserMessage { turn_id: TurnId(1), text: "hello".into() },
                LogRecord::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::EndTurn },
            ],
        });
        assert!(
            !app.log.iter().any(|e| matches!(e, LogEntry::ApprovalCard { .. })),
            "the card does not come back, and is not expected to"
        );
    }

    /// Two steps' prose is one block, exactly as the live path's deltas
    /// accumulate it — a record per step must not split what the session
    /// showed as continuous.
    #[test]
    fn consecutive_assistant_records_merge_into_one_block() {
        let mut app = app();
        app.apply_event(Event::HistoryLoaded {
            records: vec![
                LogRecord::AssistantMessage { turn_id: TurnId(1), step_id: StepId(1), text: "first".into() },
                LogRecord::AssistantMessage { turn_id: TurnId(1), step_id: StepId(2), text: "second".into() },
                LogRecord::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::EndTurn },
            ],
        });
        let blocks = app.log.iter().filter(|e| matches!(e, LogEntry::AssistantText { .. })).count();
        assert_eq!(blocks, 1, "one block, not one per step");
        assert!(matches!(app.log.first(), Some(LogEntry::AssistantText { text }) if text == "first\nsecond"));
    }

    #[test]
    fn bare_resume_opens_the_session_list_and_submits_nothing() {
        let mut app = app().with_sessions(sessions());
        type_str(&mut app, "/resume");
        app.handle_key(press(KeyCode::Enter));
        assert!(app.resume.is_some(), "the list is open");
        assert!(app.outbox.is_empty(), "and nothing was sent");
        assert_eq!(app.input, "");
    }

    /// The picker answers by typing the command — so the interceptor decides
    /// what resuming does, exactly as it does for `/model`.
    #[test]
    fn committing_the_list_submits_the_command_the_developer_would_have_typed() {
        let mut app = app().with_sessions(sessions());
        type_str(&mut app, "/resume");
        app.handle_key(press(KeyCode::Enter));
        app.handle_key(press(KeyCode::Down));
        app.handle_key(press(KeyCode::Enter));

        assert!(app.resume.is_none(), "the list closes on committing");
        assert_eq!(app.outbox, vec![Command::Submit { text: "/resume 0000000010-3".into() }]);
    }

    /// A project with no recorded sessions forwards the bare command, which
    /// aldwin-cli answers by saying so. An empty panel is not an answer.
    #[test]
    fn bare_resume_with_no_history_is_forwarded_rather_than_opening_an_empty_panel() {
        let mut app = app();
        type_str(&mut app, "/resume");
        app.handle_key(press(KeyCode::Enter));
        assert!(app.resume.is_none());
        assert_eq!(app.outbox, vec![Command::Submit { text: "/resume".into() }]);
    }

    /// An argument names the session outright — a developer who did that is
    /// not asking to be shown a list.
    #[test]
    fn resume_with_an_argument_is_forwarded_untouched() {
        let mut app = app().with_sessions(sessions());
        type_str(&mut app, "/resume 0000000020-7");
        app.handle_key(press(KeyCode::Enter));
        assert!(app.resume.is_none());
        assert_eq!(app.outbox, vec![Command::Submit { text: "/resume 0000000020-7".into() }]);
    }

    #[test]
    fn esc_closes_the_session_list_without_submitting() {
        let mut app = app().with_sessions(sessions());
        type_str(&mut app, "/resume");
        app.handle_key(press(KeyCode::Enter));
        app.handle_key(press(KeyCode::Esc));
        assert!(app.resume.is_none());
        assert!(app.outbox.is_empty());
    }

    /// The list takes every key while it is open, the same way the model
    /// picker and a pending decision do.
    #[test]
    fn the_session_list_holds_the_composer_while_it_is_open() {
        let mut app = app().with_sessions(sessions());
        type_str(&mut app, "/resume");
        app.handle_key(press(KeyCode::Enter));
        type_str(&mut app, "hello");
        assert_eq!(app.input, "", "keys reach the list, not the composer");
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

    /// Where the caret is, in the composer's own terms — the rows it draws,
    /// not the draft's source lines. Same call `ui::chrome` makes to place
    /// the real terminal cursor.
    fn caret(app: &App) -> (usize, usize) {
        crate::draft::Layout::new(&app.input, app.composer_width as usize).position(app.cursor)
    }

    #[test]
    fn up_and_down_navigate_a_multiline_draft_before_falling_through_to_scroll() {
        let mut app = app();
        type_str(&mut app, "line one\nline two\nline three");
        // Cursor starts at the end (row 2, col 10 within "line three").
        // "line two" is only 8 chars, so the column clamps on the way up.
        app.handle_key(press(KeyCode::Up));
        assert_eq!(caret(&app), (1, 8), "Up should clamp to the shorter middle line's length");
        app.handle_key(press(KeyCode::Up));
        assert_eq!(caret(&app), (0, 8), "Up again should land on the same column, which the first line can also fit");
        // No line above the first — Up here must not move the cursor
        // further (it falls through to scrolling the log instead).
        app.handle_key(press(KeyCode::Up));
        assert_eq!(caret(&app), (0, 8));
    }

    /// The wrapped-row half of the same rule. A pasted paragraph is one
    /// source line and several rows; Up from its last row has to reach the
    /// row above it, not fall straight through to the log — which is what
    /// moving by source line did.
    #[test]
    fn up_and_down_navigate_the_rows_a_wrapped_draft_actually_occupies() {
        let mut app = app();
        app.composer_width = 20;
        type_str(&mut app, "one two three four five six seven eight");
        let (row, _) = caret(&app);
        assert!(row > 0, "the draft has to wrap for this test to mean anything");
        let before = app.scroll.offset;
        app.handle_key(press(KeyCode::Up));
        assert_eq!(caret(&app).0, row - 1, "Up must move a wrapped row, not skip the whole source line");
        assert_eq!(app.scroll.offset, before, "and it must not also scroll the log");
    }

    /// The reported bug: a multi-line paste arrived as if it had been
    /// typed, so the newline in it was an Enter and submitted the first
    /// line as a message of its own.
    #[test]
    fn a_multiline_paste_lands_in_the_draft_whole_instead_of_submitting() {
        let mut app = app();
        app.paste("first line\nsecond line\nthird");
        assert_eq!(app.input, "first line\nsecond line\nthird");
        assert!(app.outbox.is_empty(), "a paste is not a submission");
        assert!(app.log.is_empty());
        app.handle_key(press(KeyCode::Enter));
        assert_eq!(app.outbox, vec![Command::Submit { text: "first line\nsecond line\nthird".into() }], "and it submits as one message");
    }

    #[test]
    fn a_paste_lands_at_the_caret_and_carries_it_along() {
        let mut app = app();
        type_str(&mut app, "ac");
        app.cursor = 1;
        app.paste("b");
        assert_eq!(app.input, "abc");
        assert_eq!(app.cursor, 2);
    }

    /// Pasting a windows-clipboard block must not leave a control
    /// character on the end of every row (see `draft::sanitize`).
    #[test]
    fn a_paste_is_normalized_before_it_reaches_the_draft() {
        let mut app = app();
        app.paste("a\r\nb\tc");
        assert_eq!(app.input, "a\nb    c");
    }

    #[test]
    fn a_paste_is_dropped_while_a_decision_holds_the_composer() {
        let mut app = app();
        app.pending_approvals.push_back(PendingApproval { call_id: "c1".into(), diff: "diff".into() });
        app.paste("some text");
        assert_eq!(app.input, "", "there is no composer on screen to paste into");
    }

    #[test]
    fn alt_enter_is_a_newline_fallback_for_terminals_that_eat_shift_enter() {
        let mut app = app();
        type_str(&mut app, "a");
        app.handle_key(press_mod(KeyCode::Enter, KeyModifiers::ALT));
        type_str(&mut app, "b");
        assert_eq!(app.input, "a\nb");
        assert!(app.outbox.is_empty());
    }

    #[test]
    fn home_and_end_stay_on_the_line_the_caret_is_on() {
        let mut app = app();
        type_str(&mut app, "first\nsecond");
        app.handle_key(press(KeyCode::Home));
        assert_eq!(app.cursor, 6, "Home goes to the start of the second line, not the top of the draft");
        app.handle_key(press(KeyCode::End));
        assert_eq!(app.cursor, 12);
    }

    /// End keeps its other job on the resting screen: with nothing drafted
    /// there is no line to move within, and it puts the transcript back on
    /// the live end of the conversation.
    #[test]
    fn end_returns_an_empty_composer_to_the_bottom_of_the_log() {
        let mut app = app();
        for i in 0..20 {
            app.log.push(LogEntry::AssistantText { text: format!("line-{i}") });
        }
        app.render_width = 80;
        let total = app.total_lines();
        app.scroll.set_viewport_height(5, total);
        app.scroll.offset = 0;
        app.scroll.following = false;
        app.handle_key(press(KeyCode::End));
        assert!(app.scroll.following);
        assert_eq!(app.scroll.offset, total - 5);
    }

    #[test]
    fn a_wheel_notch_scrolls_the_log_by_three_rows() {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let mut app = app();
        for i in 0..40 {
            app.log.push(LogEntry::AssistantText { text: format!("line-{i}") });
        }
        app.render_width = 80;
        let total = app.total_lines();
        app.scroll.set_viewport_height(10, total);
        let before = app.scroll.offset;
        let wheel = |kind| MouseEvent { kind, column: 0, row: 0, modifiers: KeyModifiers::NONE };
        app.handle_mouse(wheel(MouseEventKind::ScrollUp));
        assert_eq!(app.scroll.offset, before - 3);
        app.handle_mouse(wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.scroll.offset, before);
        // Anything that isn't the wheel leaves the transcript alone.
        app.handle_mouse(wheel(MouseEventKind::Down(MouseButton::Left)));
        assert_eq!(app.scroll.offset, before);
    }

    #[test]
    fn up_arrow_scrolls_the_log_when_the_draft_is_single_line() {
        let mut app = app();
        for i in 0..20 {
            app.log.push(LogEntry::AssistantText { text: format!("line-{i}") });
        }
        app.render_width = 80;
        let total = app.total_lines();
        app.scroll.set_viewport_height(5, total);
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

    fn ctrl_c(app: &mut App) {
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
    }

    #[test]
    fn ctrl_c_with_no_active_turn_quits() {
        let mut app = app();
        ctrl_c(&mut app);
        assert!(app.should_quit);
        assert!(app.outbox.is_empty());
    }

    #[test]
    fn ctrl_c_with_an_active_turn_cancels_instead_of_quitting() {
        let mut app = app();
        type_str(&mut app, "go");
        app.handle_key(press(KeyCode::Enter)); // submits -> a turn is coming
        app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        ctrl_c(&mut app);
        assert!(!app.should_quit);
        assert_eq!(app.outbox.last(), Some(&Command::Cancel));
    }

    /// The gap between submitting and `TurnStarted` landing: a turn is
    /// coming but isn't running yet, and Ctrl+C there must still cancel
    /// rather than quit out from under the request.
    #[test]
    fn ctrl_c_between_submit_and_turn_started_cancels() {
        let mut app = app();
        type_str(&mut app, "go");
        app.handle_key(press(KeyCode::Enter));
        ctrl_c(&mut app);
        assert!(!app.should_quit);
        assert_eq!(app.outbox.last(), Some(&Command::Cancel));
    }

    #[test]
    fn ctrl_c_after_turn_ended_quits_again() {
        let mut app = app();
        type_str(&mut app, "go");
        app.handle_key(press(KeyCode::Enter));
        app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        app.apply_event(Event::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::EndTurn });
        ctrl_c(&mut app);
        assert!(app.should_quit);
    }

    /// The reported bug: "ctrl+c after /theme appears to be broken." A
    /// slash command is submitted like any other message (so `submit` logs
    /// a `UserMessage` for it) but aldwin-cli's interceptor answers it
    /// itself — no turn ever starts, and no `TurnEnded` is ever appended.
    /// The old log-scan heuristic saw only that `UserMessage`, concluded a
    /// turn was running, and sent `Cancel` to every later Ctrl+C instead of
    /// ever exiting. Written against `/theme`'s exact event sequence (a
    /// `Notice` then `ThemeChanged`), and covering `/help`'s Notice-only
    /// shape by the same path.
    #[test]
    fn ctrl_c_still_quits_after_a_locally_handled_slash_command() {
        for events in [vec![Event::Notice { message: "theme set to light".into() }, Event::ThemeChanged { theme: "light".into() }], vec![Event::Notice { message: "commands: /help …".into() }]] {
            let mut app = app();
            type_str(&mut app, "/theme light");
            app.handle_key(press(KeyCode::Enter));
            for event in events {
                app.apply_event(event);
            }
            ctrl_c(&mut app);
            assert!(app.should_quit, "a slash command handled without a turn must not leave Ctrl+C stuck cancelling forever");
            assert!(!app.outbox.contains(&Command::Cancel), "there is no turn to cancel");
        }
    }

    /// The backstop for every other way "a turn is running" could be wrong:
    /// pressing again exits regardless, so the developer is never trapped in
    /// the session.
    #[test]
    fn a_second_ctrl_c_exits_even_while_a_turn_is_running() {
        let mut app = app();
        app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        ctrl_c(&mut app);
        assert!(!app.should_quit, "the first press cancels");
        assert_eq!(app.outbox.last(), Some(&Command::Cancel));
        ctrl_c(&mut app);
        assert!(app.should_quit, "the second press within the window exits");
    }

    /// …but only as a *double* press. Two Ctrl+Cs far enough apart are two
    /// independent cancels of two different stuck turns, not an exit — a
    /// developer who cancelled a turn minutes ago and cancels another now
    /// must not have the session quit under them.
    #[test]
    fn a_much_later_ctrl_c_cancels_again_instead_of_exiting() {
        let mut app = app();
        app.apply_event(Event::TurnStarted { turn_id: TurnId(1) });
        ctrl_c(&mut app);
        for _ in 0..=DOUBLE_CTRL_C_TICKS {
            app.tick();
        }
        ctrl_c(&mut app);
        assert!(!app.should_quit);
        assert_eq!(app.outbox.iter().filter(|c| **c == Command::Cancel).count(), 2);
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
        let payload = tool_prompt("git", &["status"], Class::Read);
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
                assert_eq!(response, PromptResponse::Tool { choice: Choice::AllowProject });
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
        let payload = tool_prompt("rm", &["-rf", "/"], Class::Write);
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });
        app.handle_key(press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.pending_prompts.is_empty(), "Ctrl+C must resolve a pending permission prompt, not get stuck");
        match app.outbox.last() {
            Some(Command::PromptResponse { payload, .. }) => {
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::Tool { choice: Choice::DenyOnce });
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
        let payload = tool_prompt("git", &["status"], Class::Read);
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });
        assert!(!app.pending_prompts.is_empty());
        assert_eq!(app.decision_options()[2].label, "Always allow git reads in this project");

        app.handle_key(press(KeyCode::Char('3'))); // option 3: allow, project tier
        assert!(app.pending_prompts.is_empty());
        match app.outbox.last() {
            Some(Command::PromptResponse { payload, .. }) => {
                let response: PromptResponse = serde_json::from_value(payload.clone()).unwrap();
                assert_eq!(response, PromptResponse::Tool { choice: Choice::AllowProject });
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
        let payload_1 = tool_prompt("git", &["status"], Class::Read);
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
                assert_eq!(response, PromptResponse::Tool { choice: Choice::AllowProject });
            }
            other => panic!("expected PromptResponse, got {other:?}"),
        }

        assert_eq!(app.decision_options().last().map(|o| o.label.as_str()), Some("Don't inject"), "call-2 is a ContextFile prompt — option 3 is its decline");
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

    /// A stream that dies while the model is thinking never sends
    /// ThinkingEnd, so the turn ending has to clear the flag itself — or the
    /// spinner keeps claiming the agent is thinking after the turn is over.
    #[test]
    fn a_turn_ending_mid_thinking_clears_the_flag() {
        let mut app = app();
        app.apply_event(Event::ThinkingStart { turn_id: TurnId(1), step_id: StepId(1) });
        app.apply_event(Event::TurnEnded { turn_id: TurnId(1), reason: TurnEndReason::Error("stream closed".into()) });
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
        let info = aldwin_core::RetryInfo { provider: "anthropic".into(), status: Some(529), message: "overloaded".into(), attempt: 1 };
        app.apply_event(Event::RetryAttempt { turn_id: TurnId(1), step_id: StepId(1), info: info.clone() });
        assert!(matches!(app.log.last(), Some(LogEntry::RetryAttempt { info: i }) if *i == info));
    }

    #[test]
    fn permissions_changed_refreshes_the_status_bar() {
        let mut app = app();
        assert_eq!(app.status.access, None, "a project that has answered nothing states no rung");
        app.apply_event(Event::PermissionsChanged { payload: serde_json::Value::Null });
        // Still unset (nothing was recorded) but proves the refresh path runs
        // without panicking on an opaque payload it doesn't need to parse.
        assert_eq!(app.status.access, None);
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

    /// `/theme` round trip: the interceptor persists to `tui.yaml` and
    /// sends this event directly (see `Event::ThemeChanged`'s own doc
    /// comment in aldwin-core) — `App` just needs to switch its own
    /// `theme` field, since `ui::draw` reads it fresh on every frame with
    /// no caching to invalidate.
    #[test]
    fn theme_changed_switches_the_active_theme() {
        let mut app = app();
        assert_eq!(app.theme, crate::palette::Theme::Dark, "Dark is the default until told otherwise");
        app.apply_event(Event::ThemeChanged { theme: "light".into() });
        assert_eq!(app.theme, crate::palette::Theme::Light);
        app.apply_event(Event::ThemeChanged { theme: "dark".into() });
        assert_eq!(app.theme, crate::palette::Theme::Dark);
    }

    /// `/model` swaps the client the session runs on, so the model name the
    /// bars read has to move with it — it used to be a startup string
    /// nothing could update, which left both bars naming a model the
    /// session had already left.
    #[test]
    fn model_changed_moves_the_name_both_bars_read() {
        let mut app = app();
        assert_eq!(app.status.model_name, "claude-sonnet-5");
        app.apply_event(Event::ModelChanged { provider: Some("google".into()), model: "gemini-2.5-flash".into() });
        assert_eq!(app.status.model_name, "gemini-2.5-flash");
        assert_eq!(app.current_provider.as_deref(), Some("google"), "the picker opens on the row the session moved to");
    }

    /// An endpoint the catalogue does not know still moves the model name;
    /// it just has no row for the picker to open on.
    #[test]
    fn model_changed_without_a_catalogue_provider_still_names_the_model() {
        let mut app = app();
        app.apply_event(Event::ModelChanged { provider: None, model: "qwen3-coder".into() });
        assert_eq!(app.status.model_name, "qwen3-coder");
        assert_eq!(app.current_provider, None);
    }

    /// Mirrors `Theme::from_config`'s own "unrecognized means dark"
    /// fallback (see its doc comment) — an unexpected value reaching this
    /// event must not panic or leave `App` in some third, unnamed state.
    #[test]
    fn theme_changed_with_an_unrecognized_value_falls_back_to_dark() {
        let mut app = app();
        app.apply_event(Event::ThemeChanged { theme: "light".into() });
        assert_eq!(app.theme, crate::palette::Theme::Light);
        app.apply_event(Event::ThemeChanged { theme: "neon".into() });
        assert_eq!(app.theme, crate::palette::Theme::Dark);
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

    fn tool_prompt(program: &str, argv: &[&str], class: Class) -> serde_json::Value {
        serde_json::to_value(PromptPayload::Tool {
            program:  program.into(),
            argv:     argv.iter().map(|a| a.to_string()).collect(),
            declared: class,
        })
        .unwrap()
    }

    /// Every row states its own rule in its own sentence (ADR 0003 §1), and
    /// under ADR 0004 the rule is the program and the class — not a glob
    /// derived from the one command line that happened to trigger the
    /// prompt. `cargo test -p gateway` and `cargo build` produce the same
    /// eight sentences, which is the friction the old model could not
    /// remove.
    #[test]
    fn every_row_quotes_the_program_and_the_class_not_the_command_line() {
        let mut app = app();
        app.apply_event(Event::PromptRequested {
            call_id: "call-1".into(),
            payload: tool_prompt("cargo", &["test", "-p", "gateway"], Class::Write),
        });

        let labels: Vec<String> = app.decision_options().into_iter().map(|o| o.label).collect();
        assert_eq!(
            labels,
            vec![
                "Allow once",
                "Allow cargo writes for this session",
                "Always allow cargo writes in this project",
                "Always allow cargo writes everywhere",
                "Deny once",
                "Deny cargo writes for this session",
                "Deny cargo writes in this project",
                "Never allow cargo",
            ]
        );
    }

    /// The class is quoted too, so the same program asks two distinguishable
    /// questions. This is the distinction the old `kind:pattern` grammar
    /// could not make at all.
    #[test]
    fn a_read_prompt_and_a_write_prompt_offer_different_rules() {
        let mut reading = app();
        reading.apply_event(Event::PromptRequested {
            call_id: "call-1".into(),
            payload: tool_prompt("git", &["status"], Class::Read),
        });
        assert_eq!(reading.decision_options()[2].label, "Always allow git reads in this project");

        let mut writing = app();
        writing.apply_event(Event::PromptRequested {
            call_id: "call-2".into(),
            payload: tool_prompt("git", &["push"], Class::Write),
        });
        assert_eq!(writing.decision_options()[2].label, "Always allow git writes in this project");
    }

    /// The second prompt of ADR 0004 §4 offers the same eight rows, because
    /// the answer persists the same way — what changed is that the call is
    /// now known to be a write.
    #[test]
    fn the_write_attempt_prompt_offers_the_same_rows_at_write_class() {
        let mut app = app();
        let payload = serde_json::to_value(PromptPayload::WriteAttempt {
            program: "rm".into(),
            argv:    vec!["notes.txt".into()],
        })
        .unwrap();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload });

        let options = app.decision_options();
        assert_eq!(options.len(), 8);
        assert_eq!(options[1].label, "Allow rm writes for this session");
        assert_eq!(options[7].label, "Never allow rm");
    }


    /// `Tab` was the scope toggle before ADR 0003 moved scope onto the rows
    /// themselves. It is no longer bound in the panel, and must be dropped
    /// silently rather than falling through to some other handler.
    #[test]
    fn tab_is_no_longer_bound_in_the_decision_panel() {
        let mut app = app();
        app.apply_event(Event::PromptRequested { call_id: "call-1".into(), payload: tool_prompt("read", &["./crates/tui/src/ui.rs"], Class::Read) });
        let before: Vec<String> = app.decision_options().into_iter().map(|o| o.label).collect();
        let outbox = app.outbox.len();

        app.handle_key(press(KeyCode::Tab));

        assert_eq!(app.decision_selected, 0, "Tab must not move the cursor");
        assert_eq!(app.outbox.len(), outbox, "nor resolve anything");
        let after: Vec<String> = app.decision_options().into_iter().map(|o| o.label).collect();
        assert_eq!(before, after, "and the sentences must not change — there is no mode left to toggle");
    }

    // ── Bare `/model` opens the picker ───────────────────────────────────

    fn app_with_catalogue() -> App {
        app().with_catalogue(crate::first_run::sample_providers(), Some("bravo".into()))
    }

    /// Bare `/model` is the one submission this side reads rather than
    /// forwards — it opens the list, and nothing reaches the interceptor
    /// until a row is taken.
    #[test]
    fn bare_model_opens_the_picker_and_submits_nothing() {
        let mut app = app_with_catalogue();
        type_str(&mut app, "/model");
        app.handle_key(press(KeyCode::Enter));
        assert!(app.picker.is_some(), "the picker is open");
        assert_eq!(app.input, "", "and the draft is spent");
        assert!(app.outbox.is_empty(), "nothing is submitted until a model is picked");
        assert!(app.log.is_empty(), "and nothing is logged either");
    }

    /// Reopening the picker after a swap opens on where the session is
    /// *now*, not where it booted — `ModelChanged` carries both halves for
    /// exactly this, and marking the wrong row `· current` would be the
    /// same stale-name bug one screen over.
    #[test]
    fn the_picker_reopens_on_the_model_the_session_swapped_to() {
        let mut app = app_with_catalogue();
        app.apply_event(Event::ModelChanged { provider: Some("delta".into()), model: "delta-small".into() });
        type_str(&mut app, "/model");
        app.handle_key(press(KeyCode::Enter));

        let picker = app.picker.as_ref().expect("the picker is open");
        let current = picker.rows().into_iter().find(|r| r.current).expect("a row is marked current");
        assert_eq!(current.label, "delta", "the provider list opens marked on the row the session moved to");
        assert_eq!(picker.model, 1, "and its model list on the model it moved to");
    }

    /// A developer who names a model is not asking to be shown a list.
    #[test]
    fn model_with_an_argument_is_forwarded_untouched() {
        let mut app = app_with_catalogue();
        type_str(&mut app, "/model anthropic/claude-opus-5");
        app.handle_key(press(KeyCode::Enter));
        assert!(app.picker.is_none());
        assert_eq!(app.outbox, vec![Command::Submit { text: "/model anthropic/claude-opus-5".into() }]);
    }

    /// With no catalogue there is no list to open, so the command goes to
    /// aldwin-cli, which reports where the developer stands.
    #[test]
    fn bare_model_without_a_catalogue_is_forwarded_as_before() {
        let mut app = app();
        type_str(&mut app, "/model");
        app.handle_key(press(KeyCode::Enter));
        assert!(app.picker.is_none());
        assert_eq!(app.outbox, vec![Command::Submit { text: "/model".into() }]);
    }

    /// The picker answers by typing the command: the write, the scope it
    /// lands in and what is reported all stay aldwin-cli's, exactly as if
    /// the developer had typed it — and the transcript records the choice
    /// above the notice that answers it.
    #[test]
    fn picking_a_model_submits_the_command_a_developer_would_have_typed() {
        let mut app = app_with_catalogue();
        type_str(&mut app, "/model");
        app.handle_key(press(KeyCode::Enter)); // open
        app.handle_key(press(KeyCode::Enter)); // take `bravo`, its models open
        app.handle_key(press(KeyCode::Down));
        app.handle_key(press(KeyCode::Enter)); // take `bravo-small`
        assert!(app.picker.is_none(), "answering closes the picker");
        assert_eq!(app.outbox, vec![Command::Submit { text: "/model bravo/bravo-small".into() }]);
        assert!(matches!(app.log.last(), Some(LogEntry::UserMessage { text }) if text == "/model bravo/bravo-small"));
    }

    /// Esc from the first list closes it, writing nothing — the session
    /// keeps the model it started on.
    #[test]
    fn esc_closes_the_picker_without_submitting_anything() {
        let mut app = app_with_catalogue();
        type_str(&mut app, "/model");
        app.handle_key(press(KeyCode::Enter));
        app.handle_key(press(KeyCode::Esc));
        assert!(app.picker.is_none());
        assert!(app.outbox.is_empty());
    }

    /// Keys go to the picker, not the composer, while it is open — a
    /// keystroke that both moved a selection and typed a character would
    /// leave a draft behind the panel nobody can see.
    #[test]
    fn keys_reach_the_picker_rather_than_the_composer_while_it_is_open() {
        let mut app = app_with_catalogue();
        type_str(&mut app, "/model");
        app.handle_key(press(KeyCode::Enter));
        type_str(&mut app, "hello");
        assert_eq!(app.input, "", "nothing is typed into a composer that is not on screen");
    }

    /// A permission prompt outranks the picker: the agent is blocked on the
    /// developer, and the panel that is drawn must be the one the next key
    /// resolves.
    #[test]
    fn a_pending_decision_takes_keys_back_from_the_picker() {
        let mut app = app_with_catalogue();
        type_str(&mut app, "/model");
        app.handle_key(press(KeyCode::Enter));
        app.apply_event(Event::PromptRequested {
            call_id: "call-1".into(),
            payload: tool_prompt("read", &["./src/main.rs"], Class::Read),
        });
        app.handle_key(press(KeyCode::Down));
        assert_eq!(app.decision_selected, 1, "the decision list is what moved");
        assert!(app.picker.is_some(), "and the picker is still there once the prompt is answered");
    }
}
