//! OpenAI-compatible wire types and the SSE-to-`LlmEvent` assembler. Nothing
//! here is `pub` outside the crate — same Wire Isolation rule as `wire.rs`.
//!
//! Grounded in a live probe against Mistral's `/v1/chat/completions` (their
//! docs don't show the streamed tool-call delta shape): SSE frames are
//! untyped `data: {...}` lines terminated by a literal `data: [DONE]`, text
//! streams via `choices[0].delta.content` fragments, and a tool call can
//! arrive whole in one delta (full id/name/arguments) rather than
//! fragmented — the assembler still buffers by index defensively, since
//! OpenAI's documented behavior does fragment `arguments` across chunks for
//! other backends.

use mjolnir_core::{ContentBlock, LlmRequest, Message, Role, StopReason, ToolCall, UsageStats};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::ProviderConfig;

// ── Request ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WireRequest {
    pub model:      String,
    pub messages:   Vec<WireMessage>,
    pub max_tokens: u32,
    pub stream:     bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools:      Vec<WireTool>,
}

#[derive(Debug, Serialize)]
pub struct WireTool {
    #[serde(rename = "type")]
    pub kind:     &'static str,
    pub function: WireFunctionDef,
}

#[derive(Debug, Serialize)]
pub struct WireFunctionDef {
    pub name:        String,
    pub description: String,
    pub parameters:  serde_json::Value,
}

#[derive(Debug, Serialize, Default)]
pub struct WireMessage {
    pub role:    &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<WireToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WireToolCall {
    pub id:       String,
    #[serde(rename = "type")]
    pub kind:     &'static str,
    pub function: WireFunctionCall,
}

#[derive(Debug, Serialize)]
pub struct WireFunctionCall {
    pub name:      String,
    pub arguments: String,
}

/// Builds the request body. No cache/thinking fields: OpenAI-compatible has
/// neither concept, so `request.cache_breakpoints` is deliberately unused
/// here — not an oversight, this provider has nothing to place a breakpoint
/// on. `max_tokens` reuses `config.extended_thinking_budget` verbatim (no
/// Anthropic-style headroom math): for this provider the field is just "max
/// output tokens," which is exactly what mjolnir-config's doc comment
/// already promises it can be used for.
pub fn build_request(config: &ProviderConfig, request: &LlmRequest<'_>) -> WireRequest {
    let tools = request
        .tools
        .iter()
        .map(|t| WireTool {
            kind:     "function",
            function: WireFunctionDef { name: t.name.clone(), description: t.description.clone(), parameters: t.input_schema.clone() },
        })
        .collect();

    let mut messages = vec![WireMessage { role: "system", content: Some(request.system.to_string()), ..Default::default() }];
    for m in request.messages {
        map_message_into(m, &mut messages);
    }

    WireRequest { model: config.model.clone(), messages, max_tokens: config.extended_thinking_budget, stream: true, tools }
}

/// OpenAI's message shape doesn't allow mixed tool-result + text content in
/// one message the way Anthropic's content-block array does, so this walks
/// blocks rather than messages: consecutive `Text` blocks and any `ToolUse`
/// blocks accumulate into one buffered message (role from `m.role`), and
/// each `ToolResult` flushes as its own separate `role:"tool"` message. In
/// practice (see mjolnir-core's agent.rs) tool results always live in their
/// own `Role::User` message and tool uses in their own `Role::Assistant`
/// message, so this produces exactly one OpenAI message per core message in
/// the common case — the per-block walk just also handles the mixed case
/// correctly without assuming it can't happen.
fn map_message_into(m: &Message, out: &mut Vec<WireMessage>) {
    let role = role_str(&m.role);
    let mut text = String::new();
    let mut tool_calls: Vec<WireToolCall> = Vec::new();

    let flush = |text: &mut String, tool_calls: &mut Vec<WireToolCall>, out: &mut Vec<WireMessage>| {
        if !text.is_empty() || !tool_calls.is_empty() {
            out.push(WireMessage {
                role,
                content: if text.is_empty() { None } else { Some(std::mem::take(text)) },
                tool_calls: if tool_calls.is_empty() { None } else { Some(std::mem::take(tool_calls)) },
                tool_call_id: None,
            });
        }
    };

    for block in &m.content {
        match block {
            ContentBlock::Text { text: t } => text.push_str(t),
            ContentBlock::ToolUse(call) => tool_calls.push(map_tool_call(call)),
            ContentBlock::ToolResult(result) => {
                flush(&mut text, &mut tool_calls, out);
                out.push(WireMessage {
                    role:         "tool",
                    content:      Some(result.content.clone()),
                    tool_calls:   None,
                    tool_call_id: Some(result.call_id.clone()),
                });
            }
        }
    }
    flush(&mut text, &mut tool_calls, out);
}

fn map_tool_call(call: &ToolCall) -> WireToolCall {
    WireToolCall {
        id:       call.id.clone(),
        kind:     "function",
        function: WireFunctionCall { name: call.name.clone(), arguments: call.input.to_string() },
    }
}

fn role_str(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

// ── SSE chunks ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct WireChunk {
    #[serde(default)]
    pub choices: Vec<WireChoice>,
    #[serde(default)]
    pub usage:   Option<WireUsage>,
}

#[derive(Debug, Deserialize, Default)]
pub struct WireChoice {
    #[serde(default)]
    pub delta:         WireDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct WireDelta {
    #[serde(default)]
    pub content:    Option<String>,
    /// Lumo (and other reasoning backends) stream thinking text here, in the
    /// same deltas as content. Core's event vocabulary has no thinking *text*
    /// — only ThinkingStart/ThinkingEnd, matching how `wire.rs` drops
    /// Anthropic's ThinkingDelta — so this is used for its presence, and its
    /// text is deliberately discarded.
    #[serde(default)]
    pub reasoning:  Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<WireToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub struct WireToolCallDelta {
    pub index:    usize,
    #[serde(default)]
    pub id:       Option<String>,
    #[serde(default)]
    pub function: Option<WireFunctionDelta>,
}

#[derive(Debug, Deserialize, Default)]
pub struct WireFunctionDelta {
    #[serde(default)]
    pub name:      Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WireUsage {
    #[serde(default)]
    pub prompt_tokens:     u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

/// Parses an error body. Mistral has returned two different shapes: an
/// observed `{"detail": "..."}` (e.g. an invalid API key) and the documented
/// `{"message": "..."}`. Tries `detail` first, then `message`, else `None`
/// and the caller falls back to the raw text.
pub fn parse_error_body(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Detail {
        detail: String,
    }
    #[derive(Deserialize)]
    struct Msg {
        message: String,
    }
    serde_json::from_str::<Detail>(text)
        .map(|d| d.detail)
        .or_else(|_| serde_json::from_str::<Msg>(text).map(|m| m.message))
        .ok()
}

// ── Assembler ────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("malformed SSE payload: {0}")]
    Frame(#[from] serde_json::Error),
    #[error("malformed tool arguments JSON for {name:?}: {source}")]
    ToolInput { name: String, #[source] source: serde_json::Error },
}

#[derive(Default)]
struct ToolBuffer {
    id:   Option<String>,
    name: Option<String>,
    json: String,
}

/// Turns a sequence of `WireChunk`s from one HTTP attempt into
/// `mjolnir_core::LlmEvent`s.
///
/// One assembler drives exactly one attempt, and the contract spans two
/// methods: feed every chunk to [`Assembler::handle`], then call
/// [`Assembler::finish`] once the stream stops for any reason. StepEnded can
/// come out of either — `handle` emits it as soon as usage is known, and
/// `finish` releases one that was still waiting for usage that never came.
/// Skipping `finish` silently loses the end of such a turn.
///
/// Tool-call deltas are buffered by
/// `tool_calls[].index` and flushed once, in index order, when
/// `finish_reason` is `"tool_calls"`; any other terminal `finish_reason`
/// (`"stop"`, `"length"`, ...) maps to `StopReason::EndTurn` — same
/// "everything unmapped falls back to EndTurn" philosophy as the Anthropic
/// assembler. Usage is taken verbatim from whichever chunk carries it: it's
/// already a complete total here, unlike Anthropic's start+delta fold.
#[derive(Default)]
pub struct Assembler {
    tool_buffers: HashMap<usize, ToolBuffer>,
    usage:        Option<WireUsage>,
    /// Set when `finish_reason` arrived before any usage did — see
    /// [`Assembler::end_step`].
    pending_stop: Option<StopReason>,
    /// Whether a ThinkingStart has been emitted without its ThinkingEnd.
    in_reasoning: bool,
}

impl Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle(&mut self, chunk: WireChunk) -> Result<Vec<mjolnir_core::LlmEvent>, WireError> {
        use mjolnir_core::LlmEvent;

        let mut events = Vec::new();

        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
            // Proton's Lumo puts usage in a trailing, choice-less chunk
            // *after* the one carrying `finish_reason`; a StepEnded held back
            // by that ordering can now be emitted with real token counts.
            if let Some(stop) = self.pending_stop.take() {
                events.push(self.step_ended(stop));
            }
        }

        let Some(choice) = chunk.choices.into_iter().next() else {
            return Ok(events);
        };

        let reasoning = choice.delta.reasoning.filter(|r| !r.is_empty());
        let text = choice.delta.content.filter(|t| !t.is_empty());
        let tool_calls = choice.delta.tool_calls.unwrap_or_default();

        // Thinking is bracketed, not transcribed: the first reasoning
        // fragment opens it and the first non-reasoning thing — text, a tool
        // call, or the finish_reason — closes it.
        if reasoning.is_some() && !self.in_reasoning {
            events.push(LlmEvent::ThinkingStart);
            self.in_reasoning = true;
        }
        if self.in_reasoning && (text.is_some() || !tool_calls.is_empty() || choice.finish_reason.is_some()) {
            events.push(LlmEvent::ThinkingEnd);
            self.in_reasoning = false;
        }

        if let Some(text) = text {
            events.push(LlmEvent::TextDelta { text });
        }
        for tc in tool_calls {
            let buf = self.tool_buffers.entry(tc.index).or_default();
            if let Some(id) = tc.id {
                buf.id = Some(id);
            }
            if let Some(function) = tc.function {
                if let Some(name) = function.name {
                    buf.name = Some(name);
                }
                if let Some(args) = function.arguments {
                    buf.json.push_str(&args);
                }
            }
        }

        match choice.finish_reason.as_deref() {
            Some("tool_calls") => {
                let mut indices: Vec<usize> = self.tool_buffers.keys().copied().collect();
                indices.sort_unstable();
                for index in indices {
                    let buf = self.tool_buffers.remove(&index).expect("index came from this map's own keys");
                    let input = if buf.json.is_empty() {
                        serde_json::Value::Object(Default::default())
                    } else {
                        serde_json::from_str(&buf.json)
                            .map_err(|source| WireError::ToolInput { name: buf.name.clone().unwrap_or_default(), source })?
                    };
                    events.push(LlmEvent::ToolUseRequested {
                        call: ToolCall { id: buf.id.unwrap_or_default(), name: buf.name.unwrap_or_default(), input },
                    });
                }
                self.end_step(StopReason::ToolUse, &mut events);
            }
            Some(_) => self.end_step(StopReason::EndTurn, &mut events),
            None => {}
        }

        Ok(events)
    }

    /// Emits StepEnded now if usage is already known (Mistral puts it in the
    /// same chunk as `finish_reason`), otherwise holds the stop reason until
    /// a trailing usage chunk arrives or the stream ends. Holding it is what
    /// makes token counts land for backends that report usage last; without
    /// it every step from such a backend reports zero.
    fn end_step(&mut self, stop_reason: StopReason, events: &mut Vec<mjolnir_core::LlmEvent>) {
        if self.usage.is_some() {
            events.push(self.step_ended(stop_reason));
        } else {
            self.pending_stop = Some(stop_reason);
        }
    }

    /// Called when the stream ends for any reason — `[DONE]`, a closed
    /// connection, an idle timeout, a framing error. A held-back StepEnded is
    /// a *complete* turn whose usage chunk never came, so it flushes (with
    /// zero usage) rather than surfacing as a stream failure.
    pub fn finish(&mut self) -> Option<mjolnir_core::LlmEvent> {
        self.pending_stop.take().map(|stop| self.step_ended(stop))
    }

    fn step_ended(&self, stop_reason: StopReason) -> mjolnir_core::LlmEvent {
        use mjolnir_core::{CacheStats, LlmEvent, StepOutcome};
        let (input_tokens, output_tokens) =
            self.usage.as_ref().map(|u| (u.prompt_tokens, u.completion_tokens)).unwrap_or_default();
        LlmEvent::StepEnded {
            outcome: StepOutcome {
                stop_reason,
                usage: UsageStats { input_tokens, output_tokens },
                cache: CacheStats { cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mjolnir_core::{ContentBlock, LlmEvent, Role, ToolDefinition, ToolResult};
    use serde_json::json;

    fn chunk(json_str: &str) -> WireChunk {
        serde_json::from_str(json_str).unwrap()
    }

    #[test]
    fn text_delta_passes_through() {
        let mut a = Assembler::new();
        let out = a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#)).unwrap();
        assert!(matches!(&out[..], [LlmEvent::TextDelta { text }] if text == "hi"));
    }

    #[test]
    fn empty_content_delta_emits_nothing() {
        let mut a = Assembler::new();
        let out = a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#)).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn tool_call_arriving_whole_in_one_delta_emits_one_tool_use_requested() {
        let mut a = Assembler::new();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"get_weather","arguments":"{\"city\": \"Paris\"}"},"index":0}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
            ))
            .unwrap();
        let [LlmEvent::ToolUseRequested { call }, LlmEvent::StepEnded { outcome }] = &out[..] else {
            panic!("expected ToolUseRequested then StepEnded, got {out:?}")
        };
        assert_eq!(call.id, "c1");
        assert_eq!(call.name, "get_weather");
        assert_eq!(call.input, json!({"city": "Paris"}));
        assert!(matches!(outcome.stop_reason, StopReason::ToolUse));
    }

    #[test]
    fn tool_call_arguments_fragmented_across_deltas_still_assembles() {
        let mut a = Assembler::new();
        assert!(a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","function":{"name":"read","arguments":"{\"path\""},"index":0}]},"finish_reason":null}]}"#
            ))
            .unwrap()
            .is_empty());
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"function":{"arguments":":\"f.rs\"}"},"index":0}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
            ))
            .unwrap();
        let [LlmEvent::ToolUseRequested { call }, LlmEvent::StepEnded { .. }] = &out[..] else { panic!("expected two events, got {out:?}") };
        assert_eq!(call.id, "c1");
        assert_eq!(call.name, "read");
        assert_eq!(call.input, json!({"path": "f.rs"}));
    }

    #[test]
    fn empty_tool_arguments_become_an_empty_object() {
        let mut a = Assembler::new();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","function":{"name":"noop"},"index":0}]},"finish_reason":"tool_calls"}]}"#,
            ))
            .unwrap();
        let [LlmEvent::ToolUseRequested { call }, ..] = &out[..] else { panic!("expected at least one event") };
        assert_eq!(call.input, json!({}));
    }

    #[test]
    fn malformed_tool_json_is_a_structured_error_not_a_panic() {
        let mut a = Assembler::new();
        a.handle(chunk(
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","function":{"name":"read","arguments":"{not json"},"index":0}]},"finish_reason":null}]}"#,
        ))
        .unwrap();
        let err = a
            .handle(chunk(r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#))
            .unwrap_err();
        assert!(matches!(err, WireError::ToolInput { .. }));
    }

    #[test]
    fn stop_finish_reason_maps_to_end_turn() {
        let mut a = Assembler::new();
        let out = a
            .handle(chunk(r#"{"choices":[{"index":0,"delta":{"content":""},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#))
            .unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else { panic!("expected StepEnded") };
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
        assert_eq!(outcome.usage.input_tokens, 10);
        assert_eq!(outcome.usage.output_tokens, 5);
    }

    #[test]
    fn an_unmapped_finish_reason_falls_back_to_end_turn() {
        let mut a = Assembler::new();
        // No usage anywhere in this stream, so StepEnded waits for the end of
        // it — the mapping is what's under test, not the timing.
        assert!(a.handle(chunk(r#"{"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}"#)).unwrap().is_empty());
        let Some(LlmEvent::StepEnded { outcome }) = a.finish() else { panic!("expected StepEnded") };
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
    }

    /// Proton's Lumo shape, captured live: `finish_reason` lands in one
    /// chunk and `usage` in a later, choice-less one. Emitting StepEnded at
    /// the first would report zero tokens for every turn.
    #[test]
    fn usage_arriving_after_finish_reason_still_reaches_step_ended() {
        let mut a = Assembler::new();
        assert_eq!(a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"content":"hi"}}]}"#)).unwrap().len(), 1);
        assert!(a
            .handle(chunk(r#"{"choices":[{"index":0,"delta":{"role":null,"content":null},"finish_reason":"stop"}]}"#))
            .unwrap()
            .is_empty());
        let out = a
            .handle(chunk(r#"{"choices":[],"usage":{"prompt_tokens":77,"completion_tokens":7,"total_tokens":84}}"#))
            .unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else { panic!("expected StepEnded, got {out:?}") };
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
        assert_eq!(outcome.usage.input_tokens, 77);
        assert_eq!(outcome.usage.output_tokens, 7);
        assert!(a.finish().is_none(), "the step was already ended; nothing left to flush");
    }

    #[test]
    fn tool_use_step_deferred_for_usage_keeps_its_stop_reason() {
        let mut a = Assembler::new();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","function":{"name":"read","arguments":"{}"},"index":0}]},"finish_reason":"tool_calls"}]}"#,
            ))
            .unwrap();
        assert!(matches!(&out[..], [LlmEvent::ToolUseRequested { .. }]), "StepEnded should be held back, got {out:?}");
        let out = a.handle(chunk(r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#)).unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else { panic!("expected StepEnded, got {out:?}") };
        assert!(matches!(outcome.stop_reason, StopReason::ToolUse));
        assert_eq!(outcome.usage.input_tokens, 3);
    }

    /// `lumo-max` streams thinking as `delta.reasoning` alongside content.
    /// Core has no thinking-text event, so the fragments bracket into
    /// ThinkingStart/ThinkingEnd and the text itself is dropped.
    #[test]
    fn reasoning_deltas_bracket_into_thinking_start_and_end() {
        let mut a = Assembler::new();
        let out = a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"content":null,"reasoning":"17*"}}]}"#)).unwrap();
        assert!(matches!(&out[..], [LlmEvent::ThinkingStart]), "got {out:?}");
        let out = a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"content":null,"reasoning":"23"}}]}"#)).unwrap();
        assert!(out.is_empty(), "thinking opens once, got {out:?}");
        let out = a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"content":"391","reasoning":null}}]}"#)).unwrap();
        let [LlmEvent::ThinkingEnd, LlmEvent::TextDelta { text }] = &out[..] else { panic!("got {out:?}") };
        assert_eq!(text, "391");
        let out = a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"content":"!"}}]}"#)).unwrap();
        assert!(matches!(&out[..], [LlmEvent::TextDelta { .. }]), "thinking closes once, got {out:?}");
    }

    #[test]
    fn a_finish_reason_closes_an_open_thinking_bracket() {
        let mut a = Assembler::new();
        a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"reasoning":"hmm"}}]}"#)).unwrap();
        let out = a
            .handle(chunk(r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#))
            .unwrap();
        assert!(matches!(&out[..], [LlmEvent::ThinkingEnd, LlmEvent::StepEnded { .. }]), "got {out:?}");
    }

    #[test]
    fn parse_error_body_handles_the_detail_shape() {
        assert_eq!(parse_error_body(r#"{"detail":"Invalid API Key"}"#), Some("Invalid API Key".to_string()));
    }

    #[test]
    fn parse_error_body_handles_the_message_shape() {
        assert_eq!(parse_error_body(r#"{"message":"bad model"}"#), Some("bad model".to_string()));
        assert_eq!(parse_error_body("not json at all"), None);
    }

    #[test]
    fn build_request_maps_tool_result_to_its_own_tool_message() {
        let config = crate::config::ProviderConfig {
            kind: mjolnir_config::ProviderKind::OpenaiCompatible,
            model: "mistral-small-latest".into(),
            api_key_env: "X".into(),
            base_url: Some("https://api.mistral.ai/v1/chat/completions".into()),
            extended_thinking_budget: 4096,
        };
        let messages = vec![Message {
            role:    Role::User,
            content: vec![ContentBlock::ToolResult(ToolResult { call_id: "t1".into(), content: "ok".into(), is_error: false })],
        }];
        let request = LlmRequest { model: "unused", system: "sys", tools: &[], messages: &messages, cache_breakpoints: &[] };

        let wire = build_request(&config, &request);
        assert_eq!(wire.messages[0].role, "system");
        assert_eq!(wire.messages[1].role, "tool");
        assert_eq!(wire.messages[1].tool_call_id.as_deref(), Some("t1"));
        assert_eq!(wire.messages[1].content.as_deref(), Some("ok"));
    }

    #[test]
    fn build_request_maps_mixed_text_and_tool_use_into_one_assistant_message() {
        let config = crate::config::ProviderConfig {
            kind: mjolnir_config::ProviderKind::OpenaiCompatible,
            model: "m".into(),
            api_key_env: "X".into(),
            base_url: Some("https://x".into()),
            extended_thinking_budget: 4096,
        };
        let messages = vec![Message {
            role:    Role::Assistant,
            content: vec![
                ContentBlock::Text { text: "checking...".into() },
                ContentBlock::ToolUse(ToolCall { id: "c1".into(), name: "read".into(), input: json!({"path": "f.rs"}) }),
            ],
        }];
        let request = LlmRequest { model: "unused", system: "sys", tools: &[], messages: &messages, cache_breakpoints: &[] };

        let wire = build_request(&config, &request);
        assert_eq!(wire.messages.len(), 2);
        let assistant = &wire.messages[1];
        assert_eq!(assistant.role, "assistant");
        assert_eq!(assistant.content.as_deref(), Some("checking..."));
        let tool_calls = assistant.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "c1");
        assert_eq!(tool_calls[0].function.name, "read");
    }

    #[test]
    fn build_request_uses_extended_thinking_budget_as_max_tokens() {
        let config = crate::config::ProviderConfig {
            kind: mjolnir_config::ProviderKind::OpenaiCompatible,
            model: "m".into(),
            api_key_env: "X".into(),
            base_url: Some("https://x".into()),
            extended_thinking_budget: 4096,
        };
        let request = LlmRequest { model: "unused", system: "sys", tools: &[], messages: &[], cache_breakpoints: &[] };
        let wire = build_request(&config, &request);
        assert_eq!(wire.max_tokens, 4096);
    }

    #[test]
    fn build_request_maps_tools_into_function_shape() {
        let config = crate::config::ProviderConfig {
            kind: mjolnir_config::ProviderKind::OpenaiCompatible,
            model: "m".into(),
            api_key_env: "X".into(),
            base_url: Some("https://x".into()),
            extended_thinking_budget: 4096,
        };
        let tools = vec![ToolDefinition { name: "read".into(), description: "reads a file".into(), input_schema: json!({"type":"object"}) }];
        let request = LlmRequest { model: "unused", system: "sys", tools: &tools, messages: &[], cache_breakpoints: &[] };
        let wire = build_request(&config, &request);
        assert_eq!(wire.tools.len(), 1);
        assert_eq!(wire.tools[0].kind, "function");
        assert_eq!(wire.tools[0].function.name, "read");
    }
}
