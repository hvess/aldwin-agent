//! OpenAI-compatible wire types and the SSE-to-`LlmEvent` assembler. Nothing
//! here may be public outside the crate (Wire Isolation, as `wire.rs`).
//!
//! From a live probe of Mistral's `/v1/chat/completions`: untyped `data:`
//! frames ending in `[DONE]`, text in `choices[0].delta.content`. Mistral
//! sends a tool call whole, but OpenAI fragments `arguments`, so tool calls
//! are buffered by index.

use aldwin_core::{ContentBlock, LlmRequest, Message, Role, StopReason, ToolCall, UsageStats};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::config::ProviderConfig;

// ── Request ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WireRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<WireMessage<'a>>,
    pub max_tokens: u32,
    pub stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<WireTool<'a>>,
}

#[derive(Debug, Serialize)]
pub struct WireTool<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: WireFunctionDef<'a>,
}

#[derive(Debug, Serialize)]
pub struct WireFunctionDef<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub parameters: &'a serde_json::Value,
}

#[derive(Debug, Serialize, Default)]
pub struct WireMessage<'a> {
    pub role: &'static str,
    /// Borrowed from one text block; owned only when several are joined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Cow<'a, str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<WireToolCall<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct WireToolCall<'a> {
    pub id: &'a str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: WireFunctionCall<'a>,
}

#[derive(Debug, Serialize)]
pub struct WireFunctionCall<'a> {
    pub name: &'a str,
    /// The wire takes the input as a JSON string, so it is written out.
    pub arguments: String,
}

/// Builds the request body. The wire has no cache or thinking fields, so
/// `request.cache_breakpoint` is unused. `max_tokens` is the thinking budget
/// with no headroom: here it means max output tokens, as aldwin-config
/// documents.
pub fn build_request<'a>(config: &'a ProviderConfig, request: &LlmRequest<'a>) -> WireRequest<'a> {
    let tools = request
        .tools
        .iter()
        .map(|t| WireTool {
            kind: "function",
            function: WireFunctionDef {
                name: &t.name,
                description: &t.description,
                parameters: &t.input_schema,
            },
        })
        .collect();

    let mut messages = vec![WireMessage {
        role: "system",
        content: Some(Cow::Borrowed(request.system)),
        ..Default::default()
    }];
    for m in request.messages {
        map_message_into(m, &mut messages);
    }

    WireRequest {
        model: &config.model,
        messages,
        max_tokens: config.thinking_budget(),
        stream: true,
        tools,
    }
}

/// Maps one message block by block, as OpenAI cannot mix tool results and
/// text in one message: `Text` and `ToolUse` accumulate into a message with
/// `m.role`, and each `ToolResult` becomes its own `role:"tool"` message.
/// aldwin-core's `agent.rs` keeps results and uses in separate messages, so
/// the mixed case is handled but rare.
fn map_message_into<'a>(m: &'a Message, out: &mut Vec<WireMessage<'a>>) {
    let role = role_str(&m.role);
    let mut text: Option<Cow<'a, str>> = None;
    let mut tool_calls: Vec<WireToolCall<'a>> = Vec::new();

    let flush = |text: &mut Option<Cow<'a, str>>,
                 tool_calls: &mut Vec<WireToolCall<'a>>,
                 out: &mut Vec<WireMessage<'a>>| {
        let text = text.take().filter(|t| !t.is_empty());
        if text.is_some() || !tool_calls.is_empty() {
            out.push(WireMessage {
                role,
                content: text,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(std::mem::take(tool_calls))
                },
                tool_call_id: None,
            });
        }
    };

    for block in &m.content {
        match block {
            ContentBlock::Text { text: t } => match &mut text {
                Some(joined) => joined.to_mut().push_str(t),
                None => text = Some(Cow::Borrowed(t)),
            },
            // Never sent: `reasoning` is response-only, and echoing it is a
            // 400. Core's history still keeps it (ADR 0006).
            ContentBlock::Thinking { .. } | ContentBlock::RedactedThinking { .. } => {}
            ContentBlock::ToolUse(call) => tool_calls.push(map_tool_call(call)),
            ContentBlock::ToolResult(result) => {
                flush(&mut text, &mut tool_calls, out);
                out.push(WireMessage {
                    role: "tool",
                    content: Some(Cow::Borrowed(&result.content)),
                    tool_calls: None,
                    tool_call_id: Some(&result.call_id),
                });
            }
        }
    }
    flush(&mut text, &mut tool_calls, out);
}

fn map_tool_call(call: &ToolCall) -> WireToolCall<'_> {
    WireToolCall {
        id: &call.id,
        kind: "function",
        function: WireFunctionCall {
            name: &call.name,
            arguments: call.input.to_string(),
        },
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
    pub usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize, Default)]
pub struct WireChoice {
    #[serde(default)]
    pub delta: WireDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct WireDelta {
    #[serde(default)]
    pub content: Option<String>,
    /// Thinking text from reasoning backends such as Lumo.
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<WireToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub struct WireToolCallDelta {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<WireFunctionDelta>,
}

#[derive(Debug, Deserialize, Default)]
pub struct WireFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WireUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

/// The message in an error body: Mistral's observed `{"detail": "..."}`
/// (an invalid key) or its documented `{"message": "..."}`. `None` for any
/// other shape.
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
    #[error("malformed tool arguments JSON for {name:?}: {source}")]
    ToolInput {
        name: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Default)]
struct ToolBuffer {
    id: Option<String>,
    name: Option<String>,
    json: String,
}

/// Turns one HTTP attempt's `WireChunk`s into `LlmEvent`s.
///
/// Feed every chunk to [`Assembler::handle`], then always call
/// [`Assembler::finish`] when the stream stops: `StepEnded` comes from
/// either, and skipping `finish` loses a step still waiting for usage.
///
/// Tool calls are buffered by `tool_calls[].index` and flushed in index order
/// on a tool turn; any other `finish_reason` is `EndTurn`. Usage is a
/// complete total, taken from whichever chunk carries it.
#[derive(Default)]
pub struct Assembler {
    tool_buffers: BTreeMap<usize, ToolBuffer>,
    usage: Option<WireUsage>,
    /// Set when `finish_reason` came before usage ([`Assembler::end_step`]).
    pending_stop: Option<StopReason>,
    /// A `ThinkingStart` has been emitted without its `ThinkingEnd`.
    in_reasoning: bool,
    /// Reasoning since `ThinkingStart`, handed over on `ThinkingEnd`.
    reasoning_buf: String,
}

impl Assembler {
    pub fn handle(&mut self, chunk: WireChunk) -> Result<Vec<aldwin_core::LlmEvent>, WireError> {
        use aldwin_core::LlmEvent;

        let mut events = Vec::new();

        if let Some(usage) = chunk.usage {
            self.usage = Some(usage);
            // Lumo sends usage in a choice-less chunk after `finish_reason`.
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

        // Reasoning opens a thinking block; the first non-reasoning delta
        // closes it with the whole text (ADR 0006). This wire has no
        // signature: leave it empty, never invent one.
        if let Some(fragment) = reasoning {
            if !self.in_reasoning {
                events.push(LlmEvent::ThinkingStart);
                self.in_reasoning = true;
            }
            self.reasoning_buf.push_str(&fragment);
            events.push(LlmEvent::ThinkingDelta { text: fragment });
        }
        if self.in_reasoning
            && (text.is_some() || !tool_calls.is_empty() || choice.finish_reason.is_some())
        {
            events.push(LlmEvent::ThinkingEnd {
                text: std::mem::take(&mut self.reasoning_buf),
                signature: String::new(),
            });
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
            // `"stop"` with calls buffered is a tool turn: some backends never
            // send `"tool_calls"`, and the calls would be lost silently.
            Some(reason)
                if reason == "tool_calls"
                    || (reason == "stop" && !self.tool_buffers.is_empty()) =>
            {
                for (_, buf) in std::mem::take(&mut self.tool_buffers) {
                    let input = if buf.json.is_empty() {
                        serde_json::Value::Object(Default::default())
                    } else {
                        serde_json::from_str(&buf.json).map_err(|source| WireError::ToolInput {
                            name: buf.name.clone().unwrap_or_default(),
                            source,
                        })?
                    };
                    events.push(LlmEvent::ToolUseRequested {
                        call: ToolCall {
                            id: buf.id.unwrap_or_default(),
                            name: buf.name.unwrap_or_default(),
                            input,
                        },
                    });
                }
                self.end_step(StopReason::ToolUse, &mut events);
            }
            Some(_) => self.end_step(StopReason::EndTurn, &mut events),
            None => {}
        }

        Ok(events)
    }

    /// Emits `StepEnded` if usage is known (Mistral sends it with
    /// `finish_reason`), else holds it for trailing usage or the stream's
    /// end; emitting early reports zero tokens for such backends.
    fn end_step(&mut self, stop_reason: StopReason, events: &mut Vec<aldwin_core::LlmEvent>) {
        if self.usage.is_some() {
            events.push(self.step_ended(stop_reason));
        } else {
            self.pending_stop = Some(stop_reason);
        }
    }

    /// Called however the stream ends. A held-back `StepEnded` is a complete
    /// step without usage: it flushes with zero usage, not as a failure.
    pub fn finish(&mut self) -> Option<aldwin_core::LlmEvent> {
        self.pending_stop.take().map(|stop| self.step_ended(stop))
    }

    fn step_ended(&self, stop_reason: StopReason) -> aldwin_core::LlmEvent {
        use aldwin_core::{CacheStats, LlmEvent, StepOutcome};
        let (input_tokens, output_tokens) = self
            .usage
            .as_ref()
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or_default();
        LlmEvent::StepEnded {
            outcome: StepOutcome {
                stop_reason,
                usage: UsageStats {
                    input_tokens,
                    output_tokens,
                },
                cache: CacheStats {
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Auth;
    use aldwin_config::ProviderKind;
    use aldwin_core::{ContentBlock, LlmEvent, Role, ToolDefinition, ToolResult};
    use serde_json::json;

    fn chunk(json_str: &str) -> WireChunk {
        serde_json::from_str(json_str).unwrap()
    }

    #[test]
    fn text_delta_passes_through() {
        let mut a = Assembler::default();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#,
            ))
            .unwrap();
        assert!(matches!(&out[..], [LlmEvent::TextDelta { text }] if text == "hi"));
    }

    #[test]
    fn empty_content_delta_emits_nothing() {
        let mut a = Assembler::default();
        let out = a.handle(chunk(r#"{"choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#)).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn tool_call_arriving_whole_in_one_delta_emits_one_tool_use_requested() {
        let mut a = Assembler::default();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"get_weather","arguments":"{\"city\": \"Paris\"}"},"index":0}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
            ))
            .unwrap();
        let [LlmEvent::ToolUseRequested { call }, LlmEvent::StepEnded { outcome }] = &out[..]
        else {
            panic!("expected ToolUseRequested then StepEnded, got {out:?}")
        };
        assert_eq!(call.id, "c1");
        assert_eq!(call.name, "get_weather");
        assert_eq!(call.input, json!({"city": "Paris"}));
        assert!(matches!(outcome.stop_reason, StopReason::ToolUse));
    }

    #[test]
    fn tool_call_arguments_fragmented_across_deltas_still_assembles() {
        let mut a = Assembler::default();
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
        let [LlmEvent::ToolUseRequested { call }, LlmEvent::StepEnded { .. }] = &out[..] else {
            panic!("expected two events, got {out:?}")
        };
        assert_eq!(call.id, "c1");
        assert_eq!(call.name, "read");
        assert_eq!(call.input, json!({"path": "f.rs"}));
    }

    #[test]
    fn empty_tool_arguments_become_an_empty_object() {
        let mut a = Assembler::default();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","function":{"name":"noop"},"index":0}]},"finish_reason":"tool_calls"}]}"#,
            ))
            .unwrap();
        let [LlmEvent::ToolUseRequested { call }, ..] = &out[..] else {
            panic!("expected at least one event")
        };
        assert_eq!(call.input, json!({}));
    }

    #[test]
    fn malformed_tool_json_is_a_structured_error_not_a_panic() {
        let mut a = Assembler::default();
        a.handle(chunk(
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","function":{"name":"read","arguments":"{not json"},"index":0}]},"finish_reason":null}]}"#,
        ))
        .unwrap();
        let err = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
            ))
            .unwrap_err();
        assert!(matches!(err, WireError::ToolInput { .. }));
    }

    /// Regression: a tool turn ended with `"stop"`, not `"tool_calls"`, lost
    /// its calls.
    #[test]
    fn tool_calls_still_flush_when_the_turn_ends_with_stop() {
        let mut a = Assembler::default();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","function":{"name":"read","arguments":"{}"},"index":0}]},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#,
            ))
            .unwrap();
        assert!(matches!(&out[0], LlmEvent::ToolUseRequested { call } if call.name == "read"));
        assert!(
            matches!(&out[1], LlmEvent::StepEnded { outcome } if outcome.stop_reason == StopReason::ToolUse)
        );
    }

    #[test]
    fn stop_finish_reason_maps_to_end_turn() {
        let mut a = Assembler::default();
        let out = a
            .handle(chunk(r#"{"choices":[{"index":0,"delta":{"content":""},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#))
            .unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else {
            panic!("expected StepEnded")
        };
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
        assert_eq!(outcome.usage.input_tokens, 10);
        assert_eq!(outcome.usage.output_tokens, 5);
    }

    #[test]
    fn an_unmapped_finish_reason_falls_back_to_end_turn() {
        let mut a = Assembler::default();
        // No usage, so `StepEnded` comes from `finish`.
        assert!(a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}"#
            ))
            .unwrap()
            .is_empty());
        let Some(LlmEvent::StepEnded { outcome }) = a.finish() else {
            panic!("expected StepEnded")
        };
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
    }

    /// Lumo, captured live: `usage` comes in a choice-less chunk after
    /// `finish_reason`; emitting `StepEnded` early reports zero tokens.
    #[test]
    fn usage_arriving_after_finish_reason_still_reaches_step_ended() {
        let mut a = Assembler::default();
        assert_eq!(
            a.handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"content":"hi"}}]}"#
            ))
            .unwrap()
            .len(),
            1
        );
        assert!(a
            .handle(chunk(r#"{"choices":[{"index":0,"delta":{"role":null,"content":null},"finish_reason":"stop"}]}"#))
            .unwrap()
            .is_empty());
        let out = a
            .handle(chunk(r#"{"choices":[],"usage":{"prompt_tokens":77,"completion_tokens":7,"total_tokens":84}}"#))
            .unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else {
            panic!("expected StepEnded, got {out:?}")
        };
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
        assert_eq!(outcome.usage.input_tokens, 77);
        assert_eq!(outcome.usage.output_tokens, 7);
        assert!(
            a.finish().is_none(),
            "the step was already ended; nothing left to flush"
        );
    }

    #[test]
    fn tool_use_step_deferred_for_usage_keeps_its_stop_reason() {
        let mut a = Assembler::default();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"id":"c1","function":{"name":"read","arguments":"{}"},"index":0}]},"finish_reason":"tool_calls"}]}"#,
            ))
            .unwrap();
        assert!(
            matches!(&out[..], [LlmEvent::ToolUseRequested { .. }]),
            "StepEnded should be held back, got {out:?}"
        );
        let out = a
            .handle(chunk(
                r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#,
            ))
            .unwrap();
        let [LlmEvent::StepEnded { outcome }] = &out[..] else {
            panic!("expected StepEnded, got {out:?}")
        };
        assert!(matches!(outcome.stop_reason, StopReason::ToolUse));
        assert_eq!(outcome.usage.input_tokens, 3);
    }

    /// `lumo-max` streams `delta.reasoning`; it closes with the whole text
    /// (ADR 0006) and an empty signature.
    #[test]
    fn reasoning_deltas_bracket_and_carry_their_text() {
        let mut a = Assembler::default();
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"content":null,"reasoning":"17*"}}]}"#,
            ))
            .unwrap();
        assert!(
            matches!(
                &out[..],
                [LlmEvent::ThinkingStart, LlmEvent::ThinkingDelta { .. }]
            ),
            "got {out:?}"
        );
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"content":null,"reasoning":"23"}}]}"#,
            ))
            .unwrap();
        assert!(
            matches!(&out[..], [LlmEvent::ThinkingDelta { .. }]),
            "thinking opens once, got {out:?}"
        );
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"content":"391","reasoning":null}}]}"#,
            ))
            .unwrap();
        let [LlmEvent::ThinkingEnd {
            text: thought,
            signature,
        }, LlmEvent::TextDelta { text }] = &out[..]
        else {
            panic!("got {out:?}")
        };
        assert_eq!(
            thought, "17*23",
            "the whole block, not just the last fragment"
        );
        assert!(signature.is_empty(), "this wire issues no signature");
        assert_eq!(text, "391");
        let out = a
            .handle(chunk(
                r#"{"choices":[{"index":0,"delta":{"content":"!"}}]}"#,
            ))
            .unwrap();
        assert!(
            matches!(&out[..], [LlmEvent::TextDelta { .. }]),
            "thinking closes once, got {out:?}"
        );
    }

    #[test]
    /// This wire has no request-side reasoning field: carried thinking must
    /// not be sent.
    fn thinking_blocks_are_not_sent_back_on_this_wire() {
        let messages = [Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Thinking {
                    text: "private".into(),
                    signature: String::new(),
                },
                ContentBlock::Text {
                    text: "visible".into(),
                },
            ],
        }];
        let mut out = Vec::new();
        map_message_into(&messages[0], &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content.as_deref(), Some("visible"));
    }

    #[test]
    fn a_finish_reason_closes_an_open_thinking_bracket() {
        let mut a = Assembler::default();
        a.handle(chunk(
            r#"{"choices":[{"index":0,"delta":{"reasoning":"hmm"}}]}"#,
        ))
        .unwrap();
        let out = a
            .handle(chunk(r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#))
            .unwrap();
        assert!(
            matches!(
                &out[..],
                [LlmEvent::ThinkingEnd { .. }, LlmEvent::StepEnded { .. }]
            ),
            "got {out:?}"
        );
    }

    #[test]
    fn parse_error_body_handles_the_detail_shape() {
        assert_eq!(
            parse_error_body(r#"{"detail":"Invalid API Key"}"#),
            Some("Invalid API Key".to_string())
        );
    }

    #[test]
    fn parse_error_body_handles_the_message_shape() {
        assert_eq!(
            parse_error_body(r#"{"message":"bad model"}"#),
            Some("bad model".to_string())
        );
        assert_eq!(parse_error_body("not json at all"), None);
    }

    #[test]
    fn build_request_maps_tool_result_to_its_own_tool_message() {
        let config = crate::config::ProviderConfig {
            kind: aldwin_config::ProviderKind::OpenaiCompatible,
            model: "mistral-small-latest".into(),
            auth: Auth::ApiKeyEnv("X".into()),
            base_url: Some("https://api.mistral.ai/v1/chat/completions".into()),
            extended_thinking_budget: Some(4096),
        };
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult(ToolResult {
                call_id: "t1".into(),
                content: "ok".into(),
                is_error: false,
            })],
        }];
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &messages,
            cache_breakpoint: None,
        };

        let wire = build_request(&config, &request);
        assert_eq!(wire.messages[0].role, "system");
        assert_eq!(wire.messages[1].role, "tool");
        assert_eq!(wire.messages[1].tool_call_id, Some("t1"));
        assert_eq!(wire.messages[1].content.as_deref(), Some("ok"));
    }

    #[test]
    fn build_request_maps_mixed_text_and_tool_use_into_one_assistant_message() {
        let config = crate::config::ProviderConfig {
            kind: aldwin_config::ProviderKind::OpenaiCompatible,
            model: "m".into(),
            auth: Auth::ApiKeyEnv("X".into()),
            base_url: Some("https://x".into()),
            extended_thinking_budget: Some(4096),
        };
        let messages = vec![Message {
            role: Role::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "checking...".into(),
                },
                ContentBlock::ToolUse(ToolCall {
                    id: "c1".into(),
                    name: "read".into(),
                    input: json!({"path": "f.rs"}),
                }),
            ],
        }];
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &messages,
            cache_breakpoint: None,
        };

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
    fn several_text_blocks_join_into_one_content_and_one_is_only_borrowed() {
        let config = ProviderConfig {
            kind: ProviderKind::OpenaiCompatible,
            model: "m".into(),
            auth: Auth::ApiKeyEnv("X".into()),
            base_url: Some("https://x".into()),
            extended_thinking_budget: Some(4096),
        };
        let text = |t: &str| ContentBlock::Text { text: t.into() };
        let messages = vec![
            Message {
                role: Role::Assistant,
                content: vec![text("one, "), text("two")],
            },
            Message {
                role: Role::User,
                content: vec![text("alone")],
            },
        ];
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &messages,
            cache_breakpoint: None,
        };
        let wire = build_request(&config, &request);
        assert_eq!(wire.messages[1].content.as_deref(), Some("one, two"));
        assert!(matches!(
            wire.messages[2].content,
            Some(Cow::Borrowed("alone"))
        ));
    }

    #[test]
    fn build_request_uses_extended_thinking_budget_as_max_tokens() {
        let config = crate::config::ProviderConfig {
            kind: aldwin_config::ProviderKind::OpenaiCompatible,
            model: "m".into(),
            auth: Auth::ApiKeyEnv("X".into()),
            base_url: Some("https://x".into()),
            extended_thinking_budget: Some(4096),
        };
        let request = LlmRequest {
            system: "sys",
            tools: &[],
            messages: &[],
            cache_breakpoint: None,
        };
        let wire = build_request(&config, &request);
        assert_eq!(wire.max_tokens, 4096);
    }

    #[test]
    fn build_request_maps_tools_into_function_shape() {
        let config = crate::config::ProviderConfig {
            kind: aldwin_config::ProviderKind::OpenaiCompatible,
            model: "m".into(),
            auth: Auth::ApiKeyEnv("X".into()),
            base_url: Some("https://x".into()),
            extended_thinking_budget: Some(4096),
        };
        let tools = vec![ToolDefinition {
            name: "read".into(),
            description: "reads a file".into(),
            input_schema: json!({"type":"object"}),
        }];
        let request = LlmRequest {
            system: "sys",
            tools: &tools,
            messages: &[],
            cache_breakpoint: None,
        };
        let wire = build_request(&config, &request);
        assert_eq!(wire.tools.len(), 1);
        assert_eq!(wire.tools[0].kind, "function");
        assert_eq!(wire.tools[0].function.name, "read");
    }
}
