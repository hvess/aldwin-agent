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
    let mut body = deltas(reply);
    body.push_str(
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    );
    body.push_str("data: [DONE]\n\n");
    Canned::Sse(body)
}

/// Streams `reply`, then keeps the stream open and silent, so the app is
/// captured mid-turn. The only way a scene holds a running turn; the client's
/// `IDLE_TIMEOUT` (60 s) outlasts any capture.
///
/// # Examples
///
/// ```
/// use aldwin_review::fake::{self, Canned};
/// assert!(matches!(fake::held("Running the tests."), Canned::SseThenStall(_)));
/// ```
pub fn held(reply: &str) -> Canned {
    Canned::SseThenStall(deltas(reply))
}

/// `reply` as SSE text-delta events, with nothing that ends the step.
///
/// # Panics
///
/// Only if `serde_json` fails to encode a `String`, which it cannot.
fn deltas(reply: &str) -> String {
    split_into_deltas(reply)
        .into_iter()
        .map(|chunk| {
            let escaped = serde_json::to_string(&chunk).expect("a string is always serialisable");
            format!("data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{escaped}}},\"finish_reason\":null}}]}}\n\n")
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
///
/// # Examples
///
/// ```
/// use aldwin_review::fake::{self, Canned};
/// let reply = fake::said_then_calls(
///     "Reading the router.",
///     &[("call-read", "read", serde_json::json!({ "path": "src/router.rs" }))],
/// );
/// let Canned::Sse(body) = reply else { unreachable!() };
/// assert!(body.find("Reading").unwrap() < body.find("call-read").unwrap());
/// ```
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
