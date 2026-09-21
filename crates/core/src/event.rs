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
    /// One fragment of thinking text. Carried, not dropped — see
    /// `ContentBlock::Thinking` and ADR 0006 for why.
    ThinkingDelta    { text: String },
    /// Closes the block opened by `ThinkingStart`, carrying the whole of it
    /// so the caller can commit one `ContentBlock::Thinking` without having
    /// to re-accumulate the deltas it already saw.
    ThinkingEnd      { text: String, signature: String },
    /// A thinking block the provider encrypted. Opaque, echoed back as-is.
    RedactedThinking { data: String },
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
    /// Thinking text as it streams. The TUI renders it in the scrim roles
    /// rather than as assistant prose; nothing else consumes it.
    ThinkingDelta { turn_id: TurnId, step_id: StepId, text: String },
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
    /// initialiser (aldwin-cli) rejecting an unknown slash command or
    /// reporting a `/reload-config` result, for example. It exists so a
    /// layer above core (which owns no other vehicle for reaching the TUI's
    /// log) has one. Not turn/step-scoped and never appended to the
    /// conversation log — this is UI-facing only.
    ///
    /// Core emits it in exactly one case, added with ADR 0006: a step that
    /// ends the turn having produced nothing the developer can see. That
    /// used to render as a blank turn and read as a hang; the floor is that
    /// a turn always says *something*, even if only that it said nothing.
    Notice { message: String },

    /// `Command::ClearHistory` landed and `ConversationLog` was wiped — the
    /// TUI wipes its own rendered log in step.
    HistoryCleared,

    /// `Command::Resume` landed: `ConversationLog` now holds `records` and
    /// nothing else. The exact counterpart of `HistoryCleared` — the TUI
    /// rebuilds its own rendered log from these, the way it wipes its own on
    /// a clear, rather than being told separately by whoever read the file.
    ///
    /// The records travel in the event rather than the TUI reading the
    /// transcript itself: aldwin-tui depends only on core and permissions
    /// and has no filesystem access by design, the same reason the model
    /// catalogue is handed to it rather than looked up.
    HistoryLoaded { records: Vec<LogRecord> },

    /// `/theme light|dark` — the raw config value, opaque to core the way
    /// `PermissionsChanged`'s payload is; aldwin-tui parses it. Core never
    /// emits this: like `Notice`, it exists because aldwin-cli's
    /// slash-command interceptor has no other vehicle to reach the TUI.
    ThemeChanged { theme: String },

    /// `/model` swapped the client the session is running on. Same "a layer
    /// above core has no other vehicle" reasoning as `Notice` and
    /// `ThemeChanged`: core is generic over `C: LlmClient` and has no idea
    /// its client is swappable, so aldwin-cli's interceptor rebuilds the
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
    /// blank slate. Refused with a `Notice` if received mid-turn, same as
    /// `Submit` mid-turn: there's no sound meaning for "forget everything"
    /// while a turn is still in flight using that same history.
    ClearHistory,

    /// `/resume` — the loaded transcript replaces `ConversationLog`, so the
    /// next turn's `messages_from_log()` sees the resumed conversation.
    /// Core acknowledges with `Event::HistoryLoaded`.
    ///
    /// It carries the records rather than a `SessionId` because core owns no
    /// filesystem dependency: aldwin-cli's interceptor reads the file (it
    /// holds the `Config` that knows where history lives) and core is handed
    /// the result. Same division as `ClearHistory`, which core acts on
    /// without knowing what `/clear` is.
    ///
    /// Refused with a `Notice` mid-turn, exactly as `ClearHistory` is:
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
    /// A completed extended-thinking block. Persisted because a resumed
    /// session that dropped it would send the provider an assistant turn
    /// whose tool call has no thinking in front of it, which is rejected —
    /// ADR 0006 §3.
    Thinking     { turn_id: TurnId, step_id: StepId, text: String, signature: String },
    /// The encrypted counterpart, kept for the same reason.
    RedactedThinking { turn_id: TurnId, step_id: StepId, data: String },
    ToolUse      { turn_id: TurnId, step_id: StepId, call: ToolCall },
    ToolResult   { turn_id: TurnId, step_id: StepId, result: crate::types::ToolResult },
    StepBoundary { turn_id: TurnId, step_id: StepId, outcome: StepOutcome },
    TurnEnded    { turn_id: TurnId, reason: TurnEndReason },
}
