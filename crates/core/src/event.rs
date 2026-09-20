use serde::{Deserialize, Serialize};
use crate::types::*;

// ── LLM-boundary events ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepOutcome {
    pub stop_reason: StopReason,
    pub usage:       UsageStats,
    pub cache:       CacheStats,
}

/// Events the LlmClient yields. Provider-agnostic; no Anthropic wire types cross this boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LlmEvent {
    TextDelta        { text: String },
    ThinkingStart,
    ThinkingEnd,
    ToolUseRequested { call: ToolCall },
    StepEnded        { outcome: StepOutcome },
    RetryAttempt     { info: RetryInfo },
}

// ── Core events (emitted upward) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnEndReason { EndTurn, Cancelled, Error(String) }

/// All events the agent emits toward the TUI / future web client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    TurnStarted { turn_id: TurnId },

    TextDelta    { turn_id: TurnId, step_id: StepId, text: String },
    ThinkingStart { turn_id: TurnId, step_id: StepId },
    ThinkingEnd   { turn_id: TurnId, step_id: StepId },

    ToolUseRequested    { turn_id: TurnId, step_id: StepId, call: ToolCall },
    ToolDispatched      { turn_id: TurnId, step_id: StepId, call_id: String },
    /// Emitted by the dispatcher via the agent's event channel; carries the rendered diff.
    ToolApprovalRequested { turn_id: TurnId, step_id: StepId, call_id: String, diff: String },
    ToolCompleted       { turn_id: TurnId, step_id: StepId, result: ToolResult },

    StepEnded    { turn_id: TurnId, step_id: StepId, outcome: StepOutcome },
    RetryAttempt { turn_id: TurnId, step_id: StepId, info: RetryInfo },

    TurnEnded    { turn_id: TurnId, reason: TurnEndReason },

    /// Permission engine needs a developer decision. Payload is opaque to
    /// core. Keyed by `call_id`, same as `ToolApprovalRequested` — a call
    /// has at most one of {approval, prompt} pending at a time, so there's
    /// no need for a separate id scheme (see `PendingReply`).
    PromptRequested  { call_id: String, payload: serde_json::Value },
    PermissionsChanged { payload: serde_json::Value },

    /// A message from outside the turn/step lifecycle — the session
    /// initialiser (mjolnir-cli) rejecting an unknown slash command or
    /// reporting a `/reload-config` result, for example. Core itself never
    /// emits this; it exists so a layer above core (which owns no other
    /// vehicle for reaching the TUI's log) has one. Not turn/step-scoped
    /// and never appended to the conversation log — this is UI-facing only.
    Notice { message: String },

    /// `Command::ClearHistory` landed and `ConversationLog` was wiped — the
    /// TUI reacts by wiping its own rendered log in step (see
    /// `mjolnir_tui::App::apply_event`), the same way `PermissionsChanged`
    /// tells it to refresh the status bar rather than carrying the new
    /// state itself.
    HistoryCleared,

    /// `Command::Resume` landed: `ConversationLog` now holds `records` and
    /// nothing else. The exact counterpart of `HistoryCleared` — the TUI
    /// rebuilds its own rendered log from these, the way it wipes its own on
    /// a clear, rather than being told separately by whoever read the file.
    ///
    /// The records travel in the event rather than the TUI reading the
    /// transcript itself: mjolnir-tui depends only on core and permissions
    /// and has no filesystem access by design, the same reason the model
    /// catalogue is handed to it rather than looked up.
    HistoryLoaded { records: Vec<LogRecord> },

    /// `/theme light|dark` — the raw config value, same "opaque to core"
    /// shape as `PermissionsChanged`'s payload: core has no opinion on what
    /// a theme is, mjolnir-tui parses it (`palette::Theme::from_config`).
    /// Core itself never emits this; same reasoning as `Notice` — mjolnir-
    /// cli's slash-command interceptor is a layer above core with no other
    /// vehicle to reach the running TUI, since it and the interceptor share
    /// one `Event` channel by construction (see mjolnir-cli's bootstrap).
    /// The interceptor persists the choice to `tui.yaml` (`Config::
    /// set_tui`) before emitting this, so a value the developer picked
    /// mid-session survives their next launch too, not just this one.
    ThemeChanged { theme: String },

    /// `/model` swapped the client the session is running on. Same "a layer
    /// above core has no other vehicle" reasoning as `Notice` and
    /// `ThemeChanged`: core is generic over `C: LlmClient` and has no idea
    /// its client is swappable, so mjolnir-cli's interceptor rebuilds the
    /// client behind the trait and announces the result here.
    ///
    /// `model` is the bare model id, the same value the session started
    /// with (`StatusInfo::model_name`); `provider` is the catalogue id of
    /// the row it belongs to, or `None` when `provider.yaml` points at an
    /// endpoint the catalogue does not know — the model picker opens on
    /// that pair, so both halves have to travel together.
    ModelChanged { provider: Option<String>, model: String },
}

// ── Commands (accepted downward) ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Submit         { text: String },
    Cancel,
    ApproveTool    { call_id: String },
    DenyTool       { call_id: String },
    PromptResponse { call_id: String, payload: serde_json::Value },
    /// `/clear` — wipes `ConversationLog` so the next turn starts from a
    /// blank slate. A no-op (with a warning) if received mid-turn, same as
    /// `Submit` mid-turn: there's no sound meaning for "forget everything"
    /// while a turn is still in flight using that same history.
    ClearHistory,

    /// `/resume` — the loaded transcript replaces `ConversationLog`, so the
    /// next turn's `messages_from_log()` sees the resumed conversation.
    /// Core acknowledges with `Event::HistoryLoaded`.
    ///
    /// It carries the records rather than a `SessionId` because core owns no
    /// filesystem dependency: mjolnir-cli's interceptor reads the file (it
    /// holds the `Config` that knows where history lives) and core is handed
    /// the result. Same division as `ClearHistory`, which core acts on
    /// without knowing what `/clear` is.
    ///
    /// Discarded with a warning mid-turn, exactly as `ClearHistory` is:
    /// there is no sound meaning for "replace the history" while a turn is
    /// in flight using it.
    Resume { records: Vec<LogRecord> },
}

// ── Log record ───────────────────────────────────────────────────────────────

/// What gets appended to the conversation log. Mirrors the event set but stripped
/// of streaming-only entries (TextDelta and ThinkingStart/End accumulate into
/// AssistantMessage before being committed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogRecord {
    TurnStarted  { turn_id: TurnId },
    UserMessage  { turn_id: TurnId, text: String },
    AssistantMessage { turn_id: TurnId, step_id: StepId, text: String },
    ToolUse      { turn_id: TurnId, step_id: StepId, call: ToolCall },
    ToolResult   { turn_id: TurnId, step_id: StepId, result: crate::types::ToolResult },
    StepBoundary { turn_id: TurnId, step_id: StepId, outcome: StepOutcome },
    TurnEnded    { turn_id: TurnId, reason: TurnEndReason },
}
