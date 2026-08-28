use serde::{Deserialize, Serialize};
use crate::types::*;

// ── LLM-boundary events ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Permission engine needs a developer decision. Payload is opaque to core.
    PromptRequested  { id: PromptId, payload: serde_json::Value },
    PermissionsChanged { payload: serde_json::Value },
}

// ── Commands (accepted downward) ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Command {
    Submit         { text: String },
    Cancel,
    ApproveTool    { call_id: String },
    DenyTool       { call_id: String },
    PromptResponse { id: PromptId, payload: serde_json::Value },
}

// ── Log record ───────────────────────────────────────────────────────────────

/// What gets appended to the conversation log. Mirrors the event set but stripped
/// of streaming-only entries (TextDelta and ThinkingStart/End accumulate into
/// AssistantMessage before being committed).
#[derive(Debug, Clone, Serialize, Deserialize)]
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
