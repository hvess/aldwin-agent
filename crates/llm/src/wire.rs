//! Anthropic wire types and the SSE-to-`LlmEvent` assembler. Nothing here is
//! `pub` outside the crate — see amundsen-llm.md's Wire Isolation decision.

use amundsen_core::{CacheStats, ContentBlock, LlmRequest, Message, Role, StepOutcome, StopReason, ToolCall, UsageStats};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::ProviderConfig;

pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic requires `max_tokens > thinking.budget_tokens`; headroom above
/// the configured thinking budget for the actual response content.
const MAX_TOKENS_HEADROOM: u32 = 4096;

// ── Request ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WireRequest {
    pub model:      String,
    pub system:     String,
    pub max_tokens: u32,
    pub stream:     bool,
    pub thinking:   WireThinking,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools:      Vec<WireTool>,
    pub messages:   Vec<WireMessage>,
}

#[derive(Debug, Serialize)]
pub struct WireThinking {
    #[serde(rename = "type")]
    pub kind:         &'static str,
    pub budget_tokens: u32,
}

#[derive(Debug, Serialize)]
pub struct WireTool {
    pub name:          String,
    pub description:   String,
    pub input_schema:  serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<WireCacheControl>,
}

#[derive(Debug, Serialize)]
pub struct WireCacheControl {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

impl WireCacheControl {
    fn ephemeral() -> Self {
        Self { kind: "ephemeral" }
    }
}

#[derive(Debug, Serialize)]
pub struct WireMessage {
    pub role:    &'static str,
    pub content: Vec<WireContentBlock>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireContentBlock {
    Text {
        text:                          String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control:                 Option<WireCacheControl>,
    },
    ToolUse {
        id:                            String,
        name:                          String,
        input:                         serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control:                 Option<WireCacheControl>,
    },
    ToolResult {
        tool_use_id:                   String,
        content:                       String,
        is_error:                      bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control:                 Option<WireCacheControl>,
    },
}

impl WireContentBlock {
    fn set_cache_control(&mut self) {
        let slot = match self {
            WireContentBlock::Text { cache_control, .. } => cache_control,
            WireContentBlock::ToolUse { cache_control, .. } => cache_control,
            WireContentBlock::ToolResult { cache_control, .. } => cache_control,
        };
        *slot = Some(WireCacheControl::ephemeral());
    }
}

/// Builds the request body. Cache placement per amundsen-llm.md: one
/// breakpoint on the last tool definition (covers system + tools), one on
/// the last content block of the message at `request.cache_breakpoints`'
/// highest index (covers the last completed turn) — at most two total, the
/// V0 ceiling, even if core ever supplied more than one breakpoint index.
pub fn build_request(config: &ProviderConfig, request: &LlmRequest<'_>) -> WireRequest {
    let mut tools: Vec<WireTool> = request
        .tools
        .iter()
        .map(|t| WireTool { name: t.name.clone(), description: t.description.clone(), input_schema: t.input_schema.clone(), cache_control: None })
        .collect();
    if let Some(last) = tools.last_mut() {
        last.cache_control = Some(WireCacheControl::ephemeral());
    }

    let mut messages: Vec<WireMessage> = request.messages.iter().map(map_message).collect();
    if let Some(&break_at) = request.cache_breakpoints.last() {
        if let Some(block) = messages.get_mut(break_at).and_then(|m| m.content.last_mut()) {
            block.set_cache_control();
        }
    }

    let budget = config.extended_thinking_budget;
    WireRequest {
        model: config.model.clone(),
        system: request.system.to_string(),
        max_tokens: budget + MAX_TOKENS_HEADROOM,
        stream: true,
        thinking: WireThinking { kind: "enabled", budget_tokens: budget },
        tools,
        messages,
    }
}

fn map_message(m: &Message) -> WireMessage {
    WireMessage { role: role_str(&m.role), content: m.content.iter().map(map_content_block).collect() }
}

fn role_str(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn map_content_block(b: &ContentBlock) -> WireContentBlock {
    match b {
        ContentBlock::Text { text } => WireContentBlock::Text { text: text.clone(), cache_control: None },
        ContentBlock::ToolUse(call) => {
            WireContentBlock::ToolUse { id: call.id.clone(), name: call.name.clone(), input: call.input.clone(), cache_control: None }
        }
        ContentBlock::ToolResult(result) => WireContentBlock::ToolResult {
            tool_use_id: result.call_id.clone(),
            content:     result.content.clone(),
            is_error:    result.is_error,
            cache_control: None,
        },
    }
}

// ── SSE events ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    MessageStart { message: WireMessageStart },
    ContentBlockStart { index: usize, content_block: WireContentBlockStart },
    ContentBlockDelta { index: usize, delta: WireDelta },
    ContentBlockStop { index: usize },
    MessageDelta { delta: WireMessageDelta, #[serde(default)] usage: Option<WireDeltaUsage> },
    MessageStop,
    Error { error: WireApiError },
    Ping,
    /// Anthropic may add event types over time; ignored rather than
    /// treated as a parse failure.
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct WireMessageStart {
    pub usage: WireUsage,
}

#[derive(Debug, Deserialize)]
pub struct WireUsage {
    #[serde(default)]
    pub input_tokens:              u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens:   u32,
}

#[derive(Debug, Deserialize)]
pub struct WireDeltaUsage {
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireContentBlockStart {
    // `text`/`thinking` bodies on these two start events are always empty
    // in practice (content streams in via later deltas) — the variant tag
    // alone is what block-kind tracking needs, so the payload isn't parsed.
    Text,
    Thinking,
    ToolUse { id: String, name: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireDelta {
    TextDelta { text: String },
    // Dropped at the parse site by construction — see Assembler::handle.
    ThinkingDelta,
    SignatureDelta,
    InputJsonDelta { #[serde(default)] partial_json: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct WireMessageDelta {
    #[serde(default)]
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WireApiError {
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct WireErrorBody {
    error: WireApiError,
}

/// Parses the `{"type":"error","error":{"type":...,"message":...}}` envelope
/// Anthropic uses both for a non-streaming HTTP error body and for an
/// in-stream `error` SSE event. `None` if the body isn't that shape — the
/// caller falls back to the raw text.
pub fn parse_error_body(text: &str) -> Option<String> {
    serde_json::from_str::<WireErrorBody>(text).ok().map(|b| b.error.message)
}

// ── Assembler ────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("malformed SSE payload: {0}")]
    Frame(#[from] serde_json::Error),
    #[error("malformed tool input JSON for {name:?}: {source}")]
    ToolInput { name: String, #[source] source: serde_json::Error },
    #[error("{message}")]
    Api { message: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    ToolUse,
    Other,
}

struct ToolBuffer {
    id:   String,
    name: String,
    json: String,
}

/// Turns a sequence of `WireEvent`s from one HTTP attempt into
/// `amundsen_core::LlmEvent`s. Per amundsen-llm.md: thinking content is
/// dropped at the parse site (only start/end markers cross the boundary);
/// tool input is buffered and emitted as one `ToolUseRequested` on
/// `content_block_stop`; usage is folded from `message_start` +
/// `message_delta`.
#[derive(Default)]
pub struct Assembler {
    blocks:         HashMap<usize, BlockKind>,
    tool_buffers:   HashMap<usize, ToolBuffer>,
    input_tokens:   u32,
    cache_creation: u32,
    cache_read:     u32,
    output_tokens:  u32,
    stop_reason:    Option<String>,
}

impl Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zero, one, or (for `content_block_stop` closing a tool block) exactly
    /// one `LlmEvent` for this wire event; `Err` on a malformed payload or an
    /// upstream `error` event.
    pub fn handle(&mut self, event: WireEvent) -> Result<Vec<amundsen_core::LlmEvent>, WireError> {
        use amundsen_core::LlmEvent;

        Ok(match event {
            WireEvent::MessageStart { message } => {
                self.input_tokens = message.usage.input_tokens;
                self.cache_creation = message.usage.cache_creation_input_tokens;
                self.cache_read = message.usage.cache_read_input_tokens;
                vec![]
            }
            WireEvent::ContentBlockStart { index, content_block } => match content_block {
                WireContentBlockStart::Text => {
                    self.blocks.insert(index, BlockKind::Text);
                    vec![]
                }
                WireContentBlockStart::Thinking => {
                    self.blocks.insert(index, BlockKind::Thinking);
                    vec![LlmEvent::ThinkingStart]
                }
                WireContentBlockStart::ToolUse { id, name } => {
                    self.blocks.insert(index, BlockKind::ToolUse);
                    self.tool_buffers.insert(index, ToolBuffer { id, name, json: String::new() });
                    vec![]
                }
                WireContentBlockStart::Other => {
                    self.blocks.insert(index, BlockKind::Other);
                    vec![]
                }
            },
            WireEvent::ContentBlockDelta { index, delta } => match delta {
                WireDelta::TextDelta { text } if self.blocks.get(&index) == Some(&BlockKind::Text) => {
                    vec![LlmEvent::TextDelta { text }]
                }
                WireDelta::InputJsonDelta { partial_json } => {
                    if let Some(buf) = self.tool_buffers.get_mut(&index) {
                        buf.json.push_str(&partial_json);
                    }
                    vec![]
                }
                // ThinkingDelta / SignatureDelta dropped here at the parse
                // site by construction — no arm buffers or re-emits them.
                _ => vec![],
            },
            WireEvent::ContentBlockStop { index } => match self.blocks.remove(&index) {
                Some(BlockKind::Thinking) => vec![LlmEvent::ThinkingEnd],
                Some(BlockKind::ToolUse) => {
                    let buf = self.tool_buffers.remove(&index).expect("ToolUse block always has a buffer");
                    let input = if buf.json.is_empty() {
                        serde_json::Value::Object(Default::default())
                    } else {
                        serde_json::from_str(&buf.json).map_err(|source| WireError::ToolInput { name: buf.name.clone(), source })?
                    };
                    vec![LlmEvent::ToolUseRequested { call: ToolCall { id: buf.id, name: buf.name, input } }]
                }
                _ => vec![],
            },
            WireEvent::MessageDelta { delta, usage } => {
                self.stop_reason = delta.stop_reason;
                if let Some(u) = usage {
                    self.output_tokens = u.output_tokens;
                }
                vec![]
            }
            WireEvent::MessageStop => {
                vec![LlmEvent::StepEnded {
                    outcome: StepOutcome {
                        stop_reason: self.resolve_stop_reason(),
                        usage: UsageStats { input_tokens: self.input_tokens, output_tokens: self.output_tokens },
                        cache: CacheStats {
                            cache_creation_input_tokens: self.cache_creation,
                            cache_read_input_tokens:     self.cache_read,
                        },
                    },
                }]
            }
            WireEvent::Error { error } => return Err(WireError::Api { message: error.message }),
            WireEvent::Ping | WireEvent::Other => vec![],
        })
    }

    /// Core's `StopReason` only has `EndTurn`/`ToolUse` — every other
    /// Anthropic stop reason (`max_tokens`, `stop_sequence`, `pause_turn`,
    /// `refusal`, ...) maps to `EndTurn`: the turn is over and no tool call
    /// is pending either way, which is the only distinction core's enum can
    /// represent.
    fn resolve_stop_reason(&self) -> StopReason {
        match self.stop_reason.as_deref() {
            Some("tool_use") => StopReason::ToolUse,
            _ => StopReason::EndTurn,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amundsen_core::{ContentBlock, LlmEvent, Role, ToolDefinition, ToolResult};
    use serde_json::json;

    fn ev(json_str: &str) -> WireEvent {
        serde_json::from_str(json_str).unwrap()
    }

    #[test]
    fn text_delta_passes_through() {
        let mut a = Assembler::new();
        a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#)).unwrap();
        let out = a.handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#)).unwrap();
        assert!(matches!(&out[..], [LlmEvent::TextDelta { text }] if text == "hi"));
    }

    #[test]
    fn thinking_content_is_dropped_only_markers_cross() {
        let mut a = Assembler::new();
        let start = a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#)).unwrap();
        assert!(matches!(&start[..], [LlmEvent::ThinkingStart]));

        let delta = a
            .handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reasoning..."}}"#))
            .unwrap();
        assert!(delta.is_empty());

        let sig = a.handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#)).unwrap();
        assert!(sig.is_empty());

        let stop = a.handle(ev(r#"{"type":"content_block_stop","index":0}"#)).unwrap();
        assert!(matches!(&stop[..], [LlmEvent::ThinkingEnd]));
    }

    #[test]
    fn tool_input_buffers_across_deltas_and_emits_once_on_stop() {
        let mut a = Assembler::new();
        a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"read"}}"#)).unwrap();
        assert!(a
            .handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\""}}"#))
            .unwrap()
            .is_empty());
        assert!(a
            .handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":":\"f.rs\"}"}}"#))
            .unwrap()
            .is_empty());

        let out = a.handle(ev(r#"{"type":"content_block_stop","index":0}"#)).unwrap();
        let [LlmEvent::ToolUseRequested { call }] = &out[..] else { panic!("expected one ToolUseRequested, got {out:?}") };
        assert_eq!(call.id, "t1");
        assert_eq!(call.name, "read");
        assert_eq!(call.input, json!({"path": "f.rs"}));
    }

    #[test]
    fn empty_tool_input_becomes_an_empty_object() {
        let mut a = Assembler::new();
        a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"noop"}}"#)).unwrap();
        let out = a.handle(ev(r#"{"type":"content_block_stop","index":0}"#)).unwrap();
        let [LlmEvent::ToolUseRequested { call }] = &out[..] else { panic!("expected one event") };
        assert_eq!(call.input, json!({}));
    }

    #[test]
    fn malformed_tool_json_is_a_structured_error_not_a_panic() {
        let mut a = Assembler::new();
        a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"read"}}"#)).unwrap();
        a.handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{not json"}}"#)).unwrap();
        let err = a.handle(ev(r#"{"type":"content_block_stop","index":0}"#)).unwrap_err();
        assert!(matches!(err, WireError::ToolInput { .. }));
    }

    #[test]
    fn usage_folds_from_message_start_and_message_delta() {
        let mut a = Assembler::new();
        a.handle(ev(r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"cache_creation_input_tokens":2,"cache_read_input_tokens":3}}}"#))
            .unwrap();
        a.handle(ev(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#)).unwrap();
        let out = a.handle(ev(r#"{"type":"message_stop"}"#)).unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else { panic!("expected StepEnded") };
        assert_eq!(outcome.usage.input_tokens, 10);
        assert_eq!(outcome.usage.output_tokens, 42);
        assert_eq!(outcome.cache.cache_creation_input_tokens, 2);
        assert_eq!(outcome.cache.cache_read_input_tokens, 3);
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
    }

    #[test]
    fn tool_use_stop_reason_maps_through() {
        let mut a = Assembler::new();
        a.handle(ev(r#"{"type":"message_start","message":{"usage":{"input_tokens":1}}}"#)).unwrap();
        a.handle(ev(r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#)).unwrap();
        let out = a.handle(ev(r#"{"type":"message_stop"}"#)).unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else { panic!("expected StepEnded") };
        assert!(matches!(outcome.stop_reason, StopReason::ToolUse));
    }

    #[test]
    fn an_unmapped_stop_reason_falls_back_to_end_turn() {
        let mut a = Assembler::new();
        a.handle(ev(r#"{"type":"message_start","message":{"usage":{"input_tokens":1}}}"#)).unwrap();
        a.handle(ev(r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#)).unwrap();
        let out = a.handle(ev(r#"{"type":"message_stop"}"#)).unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else { panic!("expected StepEnded") };
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
    }

    #[test]
    fn error_event_is_a_wire_error_not_a_step_ended() {
        let mut a = Assembler::new();
        let err = a.handle(ev(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#)).unwrap_err();
        assert!(matches!(err, WireError::Api { message } if message == "Overloaded"));
    }

    #[test]
    fn ping_and_unknown_event_types_are_ignored() {
        let mut a = Assembler::new();
        assert!(a.handle(ev(r#"{"type":"ping"}"#)).unwrap().is_empty());
        assert!(a.handle(ev(r#"{"type":"some_future_event"}"#)).unwrap().is_empty());
    }

    #[test]
    fn build_request_places_cache_control_on_last_tool_and_the_given_message_index() {
        let config = crate::config::ProviderConfig {
            kind: amundsen_config::ProviderKind::Anthropic,
            model: "claude-sonnet-5".into(),
            api_key_env: "X".into(),
            base_url: None,
            extended_thinking_budget: 1000,
        };
        let tools = vec![
            ToolDefinition { name: "a".into(), description: "".into(), input_schema: json!({}) },
            ToolDefinition { name: "b".into(), description: "".into(), input_schema: json!({}) },
        ];
        let messages = vec![
            Message { role: Role::User, content: vec![ContentBlock::Text { text: "hi".into() }] },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolResult(ToolResult { call_id: "t1".into(), content: "ok".into(), is_error: false })],
            },
        ];
        let request = LlmRequest { model: "unused", system: "sys", tools: &tools, messages: &messages, cache_breakpoints: &[1] };

        let wire = build_request(&config, &request);
        assert!(wire.tools[0].cache_control.is_none());
        assert!(wire.tools[1].cache_control.is_some());
        let last_block = wire.messages[1].content.last().unwrap();
        assert!(matches!(last_block, WireContentBlock::ToolResult { cache_control: Some(_), .. }));
        // Only the two expected breakpoints exist anywhere in the request.
        assert!(matches!(&wire.messages[0].content[0], WireContentBlock::Text { cache_control: None, .. }));
    }

    #[test]
    fn parse_error_body_extracts_the_message() {
        let text = r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad model"}}"#;
        assert_eq!(parse_error_body(text), Some("bad model".to_string()));
        assert_eq!(parse_error_body("not json at all"), None);
    }

    #[test]
    fn build_request_max_tokens_exceeds_the_thinking_budget() {
        let config = crate::config::ProviderConfig {
            kind: amundsen_config::ProviderKind::Anthropic,
            model: "m".into(),
            api_key_env: "X".into(),
            base_url: None,
            extended_thinking_budget: 8000,
        };
        let request = LlmRequest { model: "unused", system: "sys", tools: &[], messages: &[], cache_breakpoints: &[] };
        let wire = build_request(&config, &request);
        assert!(wire.max_tokens > wire.thinking.budget_tokens);
    }
}
