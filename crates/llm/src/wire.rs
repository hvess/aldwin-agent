//! Anthropic wire types and the SSE-to-`LlmEvent` assembler. Nothing here may
//! be public outside the crate (aldwin-llm.md, Wire Isolation).

use aldwin_core::{
    CacheStats, ContentBlock, LlmRequest, Message, Role, StepOutcome, StopReason, ToolCall,
    UsageStats,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::ProviderConfig;

pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Tokens added to the thinking budget in `max_tokens`, left for the answer.
const MAX_TOKENS_HEADROOM: u32 = 4096;

// ── Request ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WireRequest {
    pub model: String,
    pub system: String,
    pub max_tokens: u32,
    pub stream: bool,
    pub thinking: WireThinking,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<WireTool>,
    pub messages: Vec<WireMessage>,
}

/// Always adaptive: Claude 4.6+ models reject the older `{"type": "enabled",
/// "budget_tokens": N}` with a 400. `extended_thinking_budget` only sizes
/// `max_tokens` (`build_request`).
#[derive(Debug, Serialize)]
pub struct WireThinking {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

impl WireThinking {
    fn adaptive() -> Self {
        Self { kind: "adaptive" }
    }
}

#[derive(Debug, Serialize)]
pub struct WireTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
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
    pub role: &'static str,
    pub content: Vec<WireContentBlock>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<WireCacheControl>,
    },
    /// Echoed back verbatim on its assistant turn. Never regenerate, reorder
    /// or omit `signature`: a turn calling a tool after thinking is rejected
    /// without it.
    Thinking {
        thinking: String,
        signature: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<WireCacheControl>,
    },
    RedactedThinking {
        data: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<WireCacheControl>,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<WireCacheControl>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<WireCacheControl>,
    },
}

impl WireContentBlock {
    /// Whether the provider accepts a cache breakpoint on this block.
    fn cacheable(&self) -> bool {
        !matches!(
            self,
            WireContentBlock::Thinking { .. } | WireContentBlock::RedactedThinking { .. }
        )
    }

    fn set_cache_control(&mut self) {
        let slot = match self {
            WireContentBlock::Text { cache_control, .. } => cache_control,
            WireContentBlock::Thinking { cache_control, .. } => cache_control,
            WireContentBlock::RedactedThinking { cache_control, .. } => cache_control,
            WireContentBlock::ToolUse { cache_control, .. } => cache_control,
            WireContentBlock::ToolResult { cache_control, .. } => cache_control,
        };
        *slot = Some(WireCacheControl::ephemeral());
    }
}

/// Builds the request body, with two cache breakpoints (aldwin-llm.md): the
/// last tool (system and tools) and the last cacheable block at or before
/// `request.cache_breakpoint` (the conversation).
///
/// Deep-clones every message per step; a borrowing `Serialize` is possible
/// but the cost is unmeasured.
pub fn build_request(config: &ProviderConfig, request: &LlmRequest<'_>) -> WireRequest {
    let mut tools: Vec<WireTool> = request
        .tools
        .iter()
        .map(|t| WireTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
            cache_control: None,
        })
        .collect();
    if let Some(last) = tools.last_mut() {
        last.cache_control = Some(WireCacheControl::ephemeral());
    }

    // Indices must match `request.messages` until the breakpoint is placed;
    // empty messages are removed only afterwards.
    let mut messages: Vec<WireMessage> = request.messages.iter().map(map_message).collect();
    if let Some(break_at) = request.cache_breakpoint {
        // `cache_control` on a thinking block fails the request, and
        // `map_message` may empty a message, so search backwards for a
        // cacheable block; an earlier breakpoint only shortens the prefix.
        let end = break_at.min(messages.len().saturating_sub(1));
        if let Some(block) = messages
            .iter_mut()
            .take(end + 1)
            .rev()
            .find_map(|m| m.content.iter_mut().rev().find(|b| b.cacheable()))
        {
            block.set_cache_control();
        }
    }
    messages.retain(|m| !m.content.is_empty());

    let budget = config.thinking_budget();
    WireRequest {
        model: config.model.clone(),
        system: request.system.to_string(),
        // Saturating: the budget comes unchecked from provider.yaml.
        max_tokens: budget.saturating_add(MAX_TOKENS_HEADROOM),
        stream: true,
        thinking: WireThinking::adaptive(),
        tools,
        messages,
    }
}

/// Maps one message, dropping thinking the provider would reject (history
/// keeps it, ADR 0006):
///
/// - **No signature:** from an OpenAI-compatible provider, reachable via
///   `/model` mid-session; always rejected.
/// - **Thinking only** (no text or tool block in the message): the message
///   maps to empty and `build_request` removes it; two user messages in a
///   row are accepted.
fn map_message(m: &Message) -> WireMessage {
    let said_or_did_something = m.content.iter().any(|b| {
        matches!(
            b,
            ContentBlock::Text { .. } | ContentBlock::ToolUse(_) | ContentBlock::ToolResult(_)
        )
    });
    let content = m
        .content
        .iter()
        .filter(|b| match b {
            ContentBlock::Thinking { signature, .. } => {
                said_or_did_something && !signature.is_empty()
            }
            ContentBlock::RedactedThinking { .. } => said_or_did_something,
            _ => true,
        })
        .map(map_content_block)
        .collect();
    WireMessage {
        role: role_str(&m.role),
        content,
    }
}

fn role_str(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn map_content_block(b: &ContentBlock) -> WireContentBlock {
    match b {
        ContentBlock::Text { text } => WireContentBlock::Text {
            text: text.clone(),
            cache_control: None,
        },
        ContentBlock::Thinking { text, signature } => WireContentBlock::Thinking {
            thinking: text.clone(),
            signature: signature.clone(),
            cache_control: None,
        },
        ContentBlock::RedactedThinking { data } => WireContentBlock::RedactedThinking {
            data: data.clone(),
            cache_control: None,
        },
        ContentBlock::ToolUse(call) => WireContentBlock::ToolUse {
            id: call.id.clone(),
            name: call.name.clone(),
            input: call.input.clone(),
            cache_control: None,
        },
        ContentBlock::ToolResult(result) => WireContentBlock::ToolResult {
            tool_use_id: result.call_id.clone(),
            content: result.content.clone(),
            is_error: result.is_error,
            cache_control: None,
        },
    }
}

// ── SSE events ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    MessageStart {
        message: WireMessageStart,
    },
    ContentBlockStart {
        index: usize,
        content_block: WireContentBlockStart,
    },
    ContentBlockDelta {
        index: usize,
        delta: WireDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: WireMessageDelta,
        #[serde(default)]
        usage: Option<WireDeltaUsage>,
    },
    MessageStop,
    Error {
        error: WireApiError,
    },
    Ping,
    /// Unknown event types are ignored, not a parse failure.
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
    pub input_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

#[derive(Debug, Deserialize)]
pub struct WireDeltaUsage {
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireContentBlockStart {
    // `text`/`thinking` start with an empty body (deltas follow), so only the
    // tag is read. `redacted_thinking` does not stream: its payload is here.
    Text,
    Thinking,
    RedactedThinking {
        #[serde(default)]
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireDelta {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        #[serde(default)]
        thinking: String,
    },
    SignatureDelta {
        #[serde(default)]
        signature: String,
    },
    InputJsonDelta {
        #[serde(default)]
        partial_json: String,
    },
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

/// The message in Anthropic's `{"type":"error","error":{"type":...,"message":...}}`
/// envelope, used by HTTP error bodies and in-stream `error` events. `None`
/// for any other shape.
pub fn parse_error_body(text: &str) -> Option<String> {
    serde_json::from_str::<WireErrorBody>(text)
        .ok()
        .map(|b| b.error.message)
}

// ── Assembler ────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("malformed tool input JSON for {name:?}: {source}")]
    ToolInput {
        name: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{message}")]
    Api { message: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    RedactedThinking,
    ToolUse,
    Other,
}

struct ToolBuffer {
    id: String,
    name: String,
    json: String,
}

/// One in-flight thinking block; the signature arrives as its own delta
/// near the end.
#[derive(Default)]
struct ThinkingBuffer {
    text: String,
    signature: String,
}

/// Turns one HTTP attempt's `WireEvent`s into `LlmEvent`s. Thinking and tool
/// input are buffered until `content_block_stop`: a thinking block crosses
/// whole, with its signature, as one `ThinkingEnd` (ADR 0006).
#[derive(Default)]
pub struct Assembler {
    blocks: HashMap<usize, BlockKind>,
    tool_buffers: HashMap<usize, ToolBuffer>,
    thinking_buffers: HashMap<usize, ThinkingBuffer>,
    input_tokens: u32,
    cache_creation: u32,
    cache_read: u32,
    output_tokens: u32,
    stop_reason: Option<String>,
}

impl Assembler {
    /// The `LlmEvent`s (at most one) for this wire event; `Err` on malformed
    /// tool input or an upstream `error` event.
    pub fn handle(&mut self, event: WireEvent) -> Result<Vec<aldwin_core::LlmEvent>, WireError> {
        use aldwin_core::LlmEvent;

        Ok(match event {
            WireEvent::MessageStart { message } => {
                self.input_tokens = message.usage.input_tokens;
                self.cache_creation = message.usage.cache_creation_input_tokens;
                self.cache_read = message.usage.cache_read_input_tokens;
                vec![]
            }
            WireEvent::ContentBlockStart {
                index,
                content_block,
            } => match content_block {
                WireContentBlockStart::Text => {
                    self.blocks.insert(index, BlockKind::Text);
                    vec![]
                }
                WireContentBlockStart::Thinking => {
                    self.blocks.insert(index, BlockKind::Thinking);
                    self.thinking_buffers
                        .insert(index, ThinkingBuffer::default());
                    vec![LlmEvent::ThinkingStart]
                }
                // Arrives whole, not in deltas: nothing to buffer.
                WireContentBlockStart::RedactedThinking { data } => {
                    self.blocks.insert(index, BlockKind::RedactedThinking);
                    vec![LlmEvent::RedactedThinking { data }]
                }
                WireContentBlockStart::ToolUse { id, name } => {
                    self.blocks.insert(index, BlockKind::ToolUse);
                    self.tool_buffers.insert(
                        index,
                        ToolBuffer {
                            id,
                            name,
                            json: String::new(),
                        },
                    );
                    vec![]
                }
                WireContentBlockStart::Other => {
                    self.blocks.insert(index, BlockKind::Other);
                    vec![]
                }
            },
            WireEvent::ContentBlockDelta { index, delta } => match delta {
                WireDelta::TextDelta { text }
                    if self.blocks.get(&index) == Some(&BlockKind::Text) =>
                {
                    vec![LlmEvent::TextDelta { text }]
                }
                WireDelta::InputJsonDelta { partial_json } => {
                    if let Some(buf) = self.tool_buffers.get_mut(&index) {
                        buf.json.push_str(&partial_json);
                    }
                    vec![]
                }
                // Buffered for history at `content_block_stop`, and forwarded
                // so the TUI shows reasoning live.
                WireDelta::ThinkingDelta { thinking } => {
                    if let Some(buf) = self.thinking_buffers.get_mut(&index) {
                        buf.text.push_str(&thinking);
                    }
                    vec![LlmEvent::ThinkingDelta { text: thinking }]
                }
                // Never rendered; carried only to echo the block back intact.
                WireDelta::SignatureDelta { signature } => {
                    if let Some(buf) = self.thinking_buffers.get_mut(&index) {
                        buf.signature.push_str(&signature);
                    }
                    vec![]
                }
                _ => vec![],
            },
            WireEvent::ContentBlockStop { index } => match self.blocks.remove(&index) {
                Some(BlockKind::Thinking) => {
                    let buf = self.thinking_buffers.remove(&index).unwrap_or_default();
                    vec![LlmEvent::ThinkingEnd {
                        text: buf.text,
                        signature: buf.signature,
                    }]
                }
                Some(BlockKind::ToolUse) => {
                    let buf = self
                        .tool_buffers
                        .remove(&index)
                        .expect("ToolUse block always has a buffer");
                    let input = if buf.json.is_empty() {
                        serde_json::Value::Object(Default::default())
                    } else {
                        serde_json::from_str(&buf.json).map_err(|source| WireError::ToolInput {
                            name: buf.name.clone(),
                            source,
                        })?
                    };
                    vec![LlmEvent::ToolUseRequested {
                        call: ToolCall {
                            id: buf.id,
                            name: buf.name,
                            input,
                        },
                    }]
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
                        usage: UsageStats {
                            input_tokens: self.input_tokens,
                            output_tokens: self.output_tokens,
                        },
                        cache: CacheStats {
                            cache_creation_input_tokens: self.cache_creation,
                            cache_read_input_tokens: self.cache_read,
                        },
                    },
                }]
            }
            WireEvent::Error { error } => {
                return Err(WireError::Api {
                    message: error.message,
                })
            }
            WireEvent::Ping | WireEvent::Other => vec![],
        })
    }

    /// Every stop reason but `tool_use` (`max_tokens`, `refusal`, ...) maps
    /// to `EndTurn`: no tool call is pending, the one distinction core's
    /// `StopReason` carries.
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
    use crate::config::Auth;
    use aldwin_core::{ContentBlock, LlmEvent, Role, ToolDefinition, ToolResult};
    use serde_json::json;

    fn ev(json_str: &str) -> WireEvent {
        serde_json::from_str(json_str).unwrap()
    }

    #[test]
    fn text_delta_passes_through() {
        let mut a = Assembler::default();
        a.handle(ev(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        ))
        .unwrap();
        let out = a.handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#)).unwrap();
        assert!(matches!(&out[..], [LlmEvent::TextDelta { text }] if text == "hi"));
    }

    #[test]
    /// ADR 0006: thinking crosses whole on stop, signature included.
    fn thinking_buffers_across_deltas_and_closes_with_its_signature() {
        let mut a = Assembler::default();
        let start = a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#)).unwrap();
        assert!(matches!(&start[..], [LlmEvent::ThinkingStart]));

        let delta = a
            .handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"reasoning..."}}"#))
            .unwrap();
        assert!(
            matches!(&delta[..], [LlmEvent::ThinkingDelta { text }] if text == "reasoning..."),
            "got {delta:?}"
        );

        let more = a
            .handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" and more"}}"#))
            .unwrap();
        assert!(matches!(&more[..], [LlmEvent::ThinkingDelta { .. }]));

        // The signature is carried, never emitted as an event.
        let sig = a.handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#)).unwrap();
        assert!(sig.is_empty());

        let stop = a
            .handle(ev(r#"{"type":"content_block_stop","index":0}"#))
            .unwrap();
        let [LlmEvent::ThinkingEnd { text, signature }] = &stop[..] else {
            panic!("got {stop:?}")
        };
        assert_eq!(text, "reasoning... and more");
        assert_eq!(signature, "abc");
    }

    #[test]
    /// Arrives whole on the start event and passes straight through.
    fn redacted_thinking_is_carried_opaquely() {
        let mut a = Assembler::default();
        let out = a
            .handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"EncRypTed=="}}"#))
            .unwrap();
        let [LlmEvent::RedactedThinking { data }] = &out[..] else {
            panic!("got {out:?}")
        };
        assert_eq!(data, "EncRypTed==");
    }

    #[test]
    /// ADR 0006: a returned thinking block must serialise back intact, or a
    /// following tool-call turn is rejected.
    fn a_thinking_block_serialises_back_with_its_signature() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Thinking {
                    text: "step one".into(),
                    signature: "sig-1".into(),
                },
                ContentBlock::Text {
                    text: "done".into(),
                },
            ],
        }];
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &messages,
            cache_breakpoint: None,
        };
        let wire = build_request(&anthropic(), &request);
        let json = serde_json::to_value(&wire).unwrap();
        let block = &json["messages"][0]["content"][0];
        assert_eq!(block["type"], "thinking");
        assert_eq!(block["thinking"], "step one");
        assert_eq!(block["signature"], "sig-1");
    }

    #[test]
    fn tool_input_buffers_across_deltas_and_emits_once_on_stop() {
        let mut a = Assembler::default();
        a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"read"}}"#)).unwrap();
        assert!(a
            .handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\""}}"#))
            .unwrap()
            .is_empty());
        assert!(a
            .handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":":\"f.rs\"}"}}"#))
            .unwrap()
            .is_empty());

        let out = a
            .handle(ev(r#"{"type":"content_block_stop","index":0}"#))
            .unwrap();
        let [LlmEvent::ToolUseRequested { call }] = &out[..] else {
            panic!("expected one ToolUseRequested, got {out:?}")
        };
        assert_eq!(call.id, "t1");
        assert_eq!(call.name, "read");
        assert_eq!(call.input, json!({"path": "f.rs"}));
    }

    #[test]
    fn empty_tool_input_becomes_an_empty_object() {
        let mut a = Assembler::default();
        a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"noop"}}"#)).unwrap();
        let out = a
            .handle(ev(r#"{"type":"content_block_stop","index":0}"#))
            .unwrap();
        let [LlmEvent::ToolUseRequested { call }] = &out[..] else {
            panic!("expected one event")
        };
        assert_eq!(call.input, json!({}));
    }

    #[test]
    fn malformed_tool_json_is_a_structured_error_not_a_panic() {
        let mut a = Assembler::default();
        a.handle(ev(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"read"}}"#)).unwrap();
        a.handle(ev(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{not json"}}"#)).unwrap();
        let err = a
            .handle(ev(r#"{"type":"content_block_stop","index":0}"#))
            .unwrap_err();
        assert!(matches!(err, WireError::ToolInput { .. }));
    }

    #[test]
    fn usage_folds_from_message_start_and_message_delta() {
        let mut a = Assembler::default();
        a.handle(ev(r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"cache_creation_input_tokens":2,"cache_read_input_tokens":3}}}"#))
            .unwrap();
        a.handle(ev(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#)).unwrap();
        let out = a.handle(ev(r#"{"type":"message_stop"}"#)).unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else {
            panic!("expected StepEnded")
        };
        assert_eq!(outcome.usage.input_tokens, 10);
        assert_eq!(outcome.usage.output_tokens, 42);
        assert_eq!(outcome.cache.cache_creation_input_tokens, 2);
        assert_eq!(outcome.cache.cache_read_input_tokens, 3);
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
    }

    #[test]
    fn tool_use_stop_reason_maps_through() {
        let mut a = Assembler::default();
        a.handle(ev(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":1}}}"#,
        ))
        .unwrap();
        a.handle(ev(
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        ))
        .unwrap();
        let out = a.handle(ev(r#"{"type":"message_stop"}"#)).unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else {
            panic!("expected StepEnded")
        };
        assert!(matches!(outcome.stop_reason, StopReason::ToolUse));
    }

    #[test]
    fn an_unmapped_stop_reason_falls_back_to_end_turn() {
        let mut a = Assembler::default();
        a.handle(ev(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":1}}}"#,
        ))
        .unwrap();
        a.handle(ev(
            r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#,
        ))
        .unwrap();
        let out = a.handle(ev(r#"{"type":"message_stop"}"#)).unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else {
            panic!("expected StepEnded")
        };
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
    }

    #[test]
    fn error_event_is_a_wire_error_not_a_step_ended() {
        let mut a = Assembler::default();
        let err = a
            .handle(ev(
                r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            ))
            .unwrap_err();
        assert!(matches!(err, WireError::Api { message } if message == "Overloaded"));
    }

    #[test]
    fn ping_and_unknown_event_types_are_ignored() {
        let mut a = Assembler::default();
        assert!(a.handle(ev(r#"{"type":"ping"}"#)).unwrap().is_empty());
        assert!(a
            .handle(ev(r#"{"type":"some_future_event"}"#))
            .unwrap()
            .is_empty());
    }

    fn anthropic() -> crate::config::ProviderConfig {
        crate::config::ProviderConfig {
            kind: aldwin_config::ProviderKind::Anthropic,
            model: "claude-sonnet-5".into(),
            auth: Auth::ApiKeyEnv("X".into()),
            base_url: None,
            extended_thinking_budget: Some(1000),
        }
    }

    fn user(text: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    /// Regression (ADR 0006): a thinking-only step was sent as a thinking-only
    /// message with the cache breakpoint on it; the provider rejects both.
    #[test]
    fn a_thinking_only_turn_is_not_sent_and_never_carries_the_breakpoint() {
        let messages = vec![
            user("first"),
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Thinking {
                    text: "hmm".into(),
                    signature: "sig".into(),
                }],
            },
            user("continue"),
        ];
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &messages,
            cache_breakpoint: Some(1),
        };
        let wire = build_request(&anthropic(), &request);

        assert_eq!(
            wire.messages.len(),
            2,
            "the thinking-only message is dropped"
        );
        assert!(wire.messages.iter().all(|m| m.role == "user"));
        let json = serde_json::to_value(&wire).unwrap();
        assert!(!json.to_string().contains("\"thinking\":\"hmm\""));
        // The breakpoint moved back to the nearest cacheable block.
        assert!(matches!(
            wire.messages[0].content[0],
            WireContentBlock::Text {
                cache_control: Some(_),
                ..
            }
        ));
    }

    /// `/model` can move a session onto this wire from an OpenAI-compatible
    /// provider, whose reasoning has no signature.
    #[test]
    fn unsigned_thinking_from_another_provider_is_not_sent() {
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Thinking {
                    text: "unsigned".into(),
                    signature: String::new(),
                },
                ContentBlock::Text {
                    text: "answer".into(),
                },
            ],
        }];
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &messages,
            cache_breakpoint: Some(0),
        };
        let wire = build_request(&anthropic(), &request);

        assert_eq!(wire.messages[0].content.len(), 1);
        assert!(matches!(
            wire.messages[0].content[0],
            WireContentBlock::Text {
                cache_control: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn build_request_places_cache_control_on_last_tool_and_the_given_message_index() {
        let tools = vec![
            ToolDefinition {
                name: "a".into(),
                description: "".into(),
                input_schema: json!({}),
            },
            ToolDefinition {
                name: "b".into(),
                description: "".into(),
                input_schema: json!({}),
            },
        ];
        let messages = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
            },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolResult(ToolResult {
                    call_id: "t1".into(),
                    content: "ok".into(),
                    is_error: false,
                })],
            },
        ];
        let request = LlmRequest {
            system: "sys",
            tools: &tools,
            messages: &messages,
            cache_breakpoint: Some(1),
        };

        let wire = build_request(&anthropic(), &request);
        assert!(wire.tools[0].cache_control.is_none());
        assert!(wire.tools[1].cache_control.is_some());
        let last_block = wire.messages[1].content.last().unwrap();
        assert!(matches!(
            last_block,
            WireContentBlock::ToolResult {
                cache_control: Some(_),
                ..
            }
        ));
        // Only the two expected breakpoints exist anywhere in the request.
        assert!(matches!(
            &wire.messages[0].content[0],
            WireContentBlock::Text {
                cache_control: None,
                ..
            }
        ));
    }

    #[test]
    fn parse_error_body_extracts_the_message() {
        let text =
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad model"}}"#;
        assert_eq!(parse_error_body(text), Some("bad model".to_string()));
        assert_eq!(parse_error_body("not json at all"), None);
    }

    #[test]
    fn build_request_max_tokens_gives_headroom_above_the_thinking_budget() {
        let config = crate::config::ProviderConfig {
            kind: aldwin_config::ProviderKind::Anthropic,
            model: "m".into(),
            auth: Auth::ApiKeyEnv("X".into()),
            base_url: None,
            extended_thinking_budget: Some(8000),
        };
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        let wire = build_request(&config, &request);
        assert!(wire.max_tokens > config.thinking_budget());
    }

    /// The headroom sum must not overflow on a budget of `u32::MAX`.
    #[test]
    fn build_request_max_tokens_saturates_rather_than_overflowing() {
        let config = crate::config::ProviderConfig {
            extended_thinking_budget: Some(u32::MAX),
            ..anthropic()
        };
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        assert_eq!(build_request(&config, &request).max_tokens, u32::MAX);
    }

    /// Regression: Claude 4.6+ models answer `{"type": "enabled",
    /// "budget_tokens": N}` with a 400; only adaptive, with no
    /// `budget_tokens`, is accepted.
    #[test]
    fn build_request_sends_adaptive_thinking_with_no_budget_tokens_field() {
        let config = crate::config::ProviderConfig {
            kind: aldwin_config::ProviderKind::Anthropic,
            model: "claude-sonnet-5".into(),
            auth: Auth::ApiKeyEnv("X".into()),
            base_url: None,
            extended_thinking_budget: Some(8000),
        };
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        let wire = build_request(&config, &request);
        assert_eq!(wire.thinking.kind, "adaptive");

        let body = serde_json::to_value(&wire).unwrap();
        assert_eq!(body["thinking"], json!({"type": "adaptive"}));
    }
}
