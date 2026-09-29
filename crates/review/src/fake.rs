//! The deterministic provider a scene talks to: `aldwin-llm`'s `test_server`
//! (feature `test-server`), reached over the app's real HTTP and streaming path.
//!
//! The wire shape is OpenAI-compatible because `AnthropicClient::new` never
//! reads `base_url` (`llm/src/client.rs`), so a defect only in the Anthropic
//! client is invisible to every scene.

use serde_json::Value;

pub use aldwin_llm::test_server::{spawn, Canned, FakeServer};

/// An assistant reply streamed as several text deltas.
///
/// Must stay split: a whole reply would skip the TUI's incremental-render path.
///
/// # Panics
///
/// Only if `serde_json` fails to encode a `String`, which it cannot.
pub fn text(reply: &str) -> Canned {
    Canned::Sse(deltas(reply) + STOP)
}

/// The end of a reply that called no tool.
const STOP: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

/// Reasoning, then a reply, as a provider that thinks streams them: the
/// wire's `reasoning` deltas open a thinking block the first text closes
/// (ADR 0006, 0015).
///
/// # Panics
///
/// Only if `serde_json` fails to encode a `String`, which it cannot.
pub fn thought_then_text(reasoning: &str, reply: &str) -> Canned {
    Canned::Sse(field_deltas("reasoning", reasoning) + &deltas(reply) + STOP)
}

/// Streams `reply`, then keeps the stream open and silent, so the app is
/// captured mid-turn. The only way a scene holds a running turn; the client's
/// `IDLE_TIMEOUT` (60 s) outlasts any capture.
pub fn held(reply: &str) -> Canned {
    Canned::SseThenStall(deltas(reply))
}

/// `reply` as SSE text-delta events, with nothing that ends the step.
fn deltas(reply: &str) -> String {
    field_deltas("content", reply)
}

/// `text` as SSE deltas of the delta object's `field`.
///
/// # Panics
///
/// Only if `serde_json` fails to encode a `String`, which it cannot.
fn field_deltas(field: &str, text: &str) -> String {
    split_into_deltas(text)
        .into_iter()
        .map(|chunk| {
            let escaped = serde_json::to_string(&chunk).expect("a string is always serialisable");
            format!("data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"{field}\":{escaped}}},\"finish_reason\":null}}]}}\n\n")
        })
        .collect()
}

/// Roughly 40-byte pieces, split after whitespace so no delta splits a word.
fn split_into_deltas(reply: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for word in reply.split_inclusive(char::is_whitespace) {
        if current.len() + word.len() > 40 && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// A tool call as an OpenAI-compatible provider streams it: whole, in one
/// delta, arguments as a JSON string. The real dispatcher then runs it.
pub fn tool_call(id: &str, name: &str, arguments: Value) -> Canned {
    tool_calls(&[(id, name, arguments)])
}

/// Several tool calls in one reply, e.g. edits staged into one changeset.
pub fn tool_calls(calls: &[(&str, &str, Value)]) -> Canned {
    said_then_calls("", calls)
}

/// Prose, then tool calls, in one reply.
///
/// # Panics
///
/// Only if `serde_json` fails to encode a `serde_json::Value` or a `String`,
/// which it cannot.
pub fn said_then_calls(text: &str, calls: &[(&str, &str, Value)]) -> Canned {
    let encoded = calls
        .iter()
        .enumerate()
        .map(|(i, (id, name, arguments))| {
            let args = serde_json::to_string(arguments).expect("scene arguments are serialisable");
            let args = serde_json::to_string(&args).expect("a string is always serialisable");
            format!("{{\"index\":{i},\"id\":\"{id}\",\"type\":\"function\",\"function\":{{\"name\":\"{name}\",\"arguments\":{args}}}}}")
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut body = deltas(text);
    body.push_str(&format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"tool_calls\":[{encoded}]}},\"finish_reason\":null}}]}}\n\n"
    ));
    body.push_str(
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    );
    body.push_str("data: [DONE]\n\n");
    Canned::Sse(body)
}

/// The URL a scene's `provider.yaml` points at; the port is ephemeral, so it
/// is written per run.
pub fn endpoint(server: &FakeServer) -> String {
    server.url("/v1/chat/completions")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn body(reply: Canned) -> String {
        match reply {
            Canned::Sse(body) | Canned::SseThenStall(body) => body,
            other => panic!("not a stream: {other:?}"),
        }
    }

    /// The app reads a reply in stream order, so each part must come in the
    /// order a provider sends it.
    #[test]
    fn a_reply_streams_its_parts_in_order() {
        let thought = body(thought_then_text("Weighing it up.", "Done."));
        assert!(thought.find("\"reasoning\"").unwrap() < thought.find("Done.").unwrap());
        assert!(thought.ends_with("data: [DONE]\n\n"));

        let calls = body(said_then_calls(
            "Reading the router.",
            &[("call-read", "read", json!({ "path": "src/router.rs" }))],
        ));
        assert!(calls.find("Reading").unwrap() < calls.find("call-read").unwrap());

        let stalled = held("Running the tests.");
        assert!(matches!(stalled, Canned::SseThenStall(ref b) if !b.contains("[DONE]")));
    }
}
