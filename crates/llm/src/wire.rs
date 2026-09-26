//! Anthropic wire types and the SSE-to-`LlmEvent` assembler. Nothing here is
//! `pub` outside the crate — see aldwin-llm.md's Wire Isolation decision.

use aldwin_core::{
    CacheStats, ContentBlock, LlmRequest, Message, Role, StepOutcome, StopReason, ToolCall,
    UsageStats,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::ProviderConfig;

pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Headroom above `extended_thinking_budget` for the actual response
/// content, so `max_tokens` isn't sized down to exactly the thinking spend
/// with nothing left for the answer.
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

/// Adaptive thinking is the only mode current Claude models (Sonnet 5, Opus
/// 5, and the rest of the 4.6+ family this project targets) accept —
/// `{ "type": "enabled", "budget_tokens": N }` is the pre-4.6 shape and gets
/// rejected with a 400 ("thinking.type.enabled is not support for this
/// model") on all of them. No `budget_tokens` field exists on this variant;
/// `extended_thinking_budget` still sizes `max_tokens`' headroom (see
/// `build_request`) but no longer names a literal request field.
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
    /// Echoed back verbatim on the assistant turn that produced it. The
    /// signature is the provider's own stamp over the block: it is not
    /// ours to regenerate, reorder or omit, and a turn that calls a tool
    /// after thinking is rejected outright without it.
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

/// Builds the request body. Cache placement per aldwin-llm.md: one
/// breakpoint on the last tool definition (covers system + tools), one on
/// the last content block of the message at `request.cache_breakpoint`
/// (covers the conversation so far) — two in all, the V0 ceiling.
///
/// Known cost, not fixed here: `WireRequest` owns everything, so this
/// deep-clones every message once per step. A borrowing `Serialize` is
/// possible but the cost is unmeasured against real session lengths, and the
/// body is serialised and sent over the network straight afterwards.
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

    // Indices are kept aligned with `request.messages` until the breakpoint
    // is placed; a message that mapped to nothing is removed only afterwards.
    let mut messages: Vec<WireMessage> = request.messages.iter().map(map_message).collect();
    if let Some(break_at) = request.cache_breakpoint {
        // The provider refuses `cache_control` on a thinking block, and a
        // message may have been emptied by `map_message`. So: the last block
        // that can carry one, in the nearest message at or before the index
        // that has one. Moving a breakpoint earlier only shortens the cached
        // prefix; putting it on a thinking block fails the request.
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
        // Saturating: the budget is whatever a developer typed into
        // provider.yaml, and an overflow here is a panic in a debug build.
        max_tokens: budget.saturating_add(MAX_TOKENS_HEADROOM),
        stream: true,
        thinking: WireThinking::adaptive(),
        tools,
        messages,
    }
}

/// Maps one message, deciding which of its thinking blocks may go back.
///
/// Carrying thinking in history (ADR 0006) is unconditional; *sending* it is
/// not, and two cases are dropped here rather than sent to fail:
///
/// - **A block with no signature.** It came from an OpenAI-compatible
///   provider, which issues none, and `/model` can move a running session
///   from one onto this wire. The signature is how this provider verifies the
///   block is its own; an empty one is a guaranteed rejection.
/// - **Thinking with nothing after it.** The provider wants thinking back
///   when the same turn went on to call a tool. A turn that *only* thought —
///   the 14,096-token case ADR 0006 opens with — has no tool call to justify
///   it, and an assistant message made of nothing but thinking is not a reply
///   the conversation can carry. The message maps to empty and
///   `build_request` removes it; two user messages in a row are accepted.
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
    // `text`/`thinking` bodies on these two start events are always empty
    // in practice (content streams in via later deltas), so only the tag is
    // needed. `redacted_thinking` is the exception: it does not stream, so
    // its whole payload arrives here and is captured.
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

/// Parses the `{"type":"error","error":{"type":...,"message":...}}` envelope
/// Anthropic uses both for a non-streaming HTTP error body and for an
/// in-stream `error` SSE event. `None` if the body isn't that shape — the
/// caller falls back to the raw text.
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

/// One in-flight thinking block: its text as the deltas build it, and the
/// signature, which arrives as its own delta near the end.
#[derive(Default)]
struct ThinkingBuffer {
    text: String,
    signature: String,
}

/// Turns a sequence of `WireEvent`s from one HTTP attempt into
/// `aldwin_core::LlmEvent`s. Thinking is buffered like tool input and
/// closed out on `content_block_stop`, so the whole block — text and the
/// provider's signature over it — crosses the boundary as one
/// `ThinkingEnd` (ADR 0006; it was dropped here until then, which both
/// blanked reasoning-only turns and made the next request invalid). Tool
/// input is buffered and emitted as one `ToolUseRequested` on
/// `content_block_stop`; usage is folded from `message_start` +
/// `message_delta`.
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
    /// Zero, one, or (for `content_block_stop` closing a tool block) exactly
    /// one `LlmEvent` for this wire event; `Err` on a malformed payload or an
    /// upstream `error` event.
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
                // Arrives whole rather than in deltas, so it is emitted on
                // sight; there is no block to buffer.
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
                // Buffered *and* forwarded: the buffer is what gets committed
                // to history at `content_block_stop`, the event is what lets
                // the TUI show reasoning as it arrives instead of a spinner.
                WireDelta::ThinkingDelta { thinking } => {
                    if let Some(buf) = self.thinking_buffers.get_mut(&index) {
                        buf.text.push_str(&thinking);
                    }
                    vec![LlmEvent::ThinkingDelta { text: thinking }]
                }
                // Never rendered — it is the provider's stamp over the block,
                // carried only so the block can be echoed back intact.
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
    /// ADR 0006 reversed this: thinking used to be dropped at the parse site
    /// and only its brackets crossed. It is now buffered across deltas and
    /// handed over whole on stop, signature included.
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

        // The signature is carried but never surfaced as an event: it is the
        // provider's stamp, not something to render.
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
    /// The encrypted counterpart arrives whole on the start event rather than
    /// in deltas, and is passed straight through.
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
    /// The round trip ADR 0006 exists for: a thinking block that came back
    /// from the provider has to serialise into the next request intact, or
    /// the turn that follows it with a tool call is rejected.
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

    /// Audit: the turn ADR 0006 was written for — a step that only thought —
    /// became an assistant message of nothing but thinking, with the cache
    /// breakpoint on the thinking block. Both are rejected by the provider.
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
        // The breakpoint moved back onto the nearest block that can hold it.
        assert!(matches!(
            wire.messages[0].content[0],
            WireContentBlock::Text {
                cache_control: Some(_),
                ..
            }
        ));
    }

    /// Audit: `/model` can move a session from an OpenAI-compatible provider,
    /// whose reasoning has no signature, onto this wire.
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

    /// The budget is a developer-typed `u32`; the headroom sum must not
    /// overflow on one that is already at the top of the range.
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

    /// Regression test: current Claude models (Sonnet 5, Opus 5, the rest of
    /// the 4.6+ family) reject the pre-4.6 `{"type": "enabled",
    /// "budget_tokens": N}` thinking shape outright — reported live as
    /// `provider error 400: "thinking.type.enabled" is not support for this
    /// model`. The request must send adaptive thinking instead, with no
    /// `budget_tokens` field at all.
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
