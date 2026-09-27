//! The provider a scene talks to.
//!
//! Scenes need the TUI to draw a real conversation, and a real model cannot
//! give one: it is slow, it costs money, and it says something different every
//! time, so the same scene would produce a different frame on every run. This
//! is the product's own canned-response server — `aldwin-llm`'s
//! `test_server`, behind its `test-server` feature — driving the app through
//! the whole real path: HTTP on a real socket, the streaming adapter, core's
//! event loop, the TUI.
//!
//! The wire shape is **OpenAI-compatible**, not Anthropic, and that is forced
//! rather than chosen: `base_url` is deliberately ignored for the `anthropic`
//! provider (`llm/src/client.rs:64`, quoting `provider.yaml`'s own comment),
//! and pointing the app at a local fake *is* setting `base_url`. So a scene
//! exercises the OpenAI adapter. A defect living only in the Anthropic client
//! is therefore invisible to this harness, which is worth knowing before
//! reading a clean run as coverage of both.

pub use aldwin_llm::test_server::{spawn, Canned, FakeServer};

/// An assistant reply streamed as text deltas.
///
/// Split across several deltas on purpose: one frame per delta is what the
/// TUI actually receives from a live provider, and a scene that arrived whole
/// would not exercise the incremental-render path (`e144409` fixed a
/// re-render-the-world defect that only exists while streaming).
///
/// # Panics
///
/// Only if `serde_json` fails to encode a `String`, which it cannot.
pub fn text(reply: &str) -> Canned {
    let mut body = String::new();
    for chunk in split_into_deltas(reply) {
        let escaped = serde_json::to_string(&chunk).expect("a string is always serialisable");
        body.push_str(&format!("data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{escaped}}},\"finish_reason\":null}}]}}\n\n"));
    }
    body.push_str(
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    );
    body.push_str("data: [DONE]\n\n");
    Canned::Sse(body)
}

/// Roughly 40-character pieces, split on whitespace so a delta boundary never
/// lands inside a word — which would be a shape no provider produces.
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

/// A tool call, streamed the way an OpenAI-compatible provider sends one:
/// whole, in a single delta, with the arguments as a JSON *string*.
///
/// This is what puts the TUI's plan, question and review on screen. What the
/// call then does — a read, a staged edit, a question — is the real
/// dispatcher's business, not the provider's, so a scene reaches those states
/// through the real path rather than by faking a panel.
pub fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> Canned {
    tool_calls(&[(id, name, arguments)])
}

/// Several calls in one turn — which is how a scene stages more than one
/// edit into a single changeset, or updates the plan beside a read.
///
/// # Panics
///
/// Only if `serde_json` fails to encode a `serde_json::Value` or a `String`,
/// which it cannot.
pub fn tool_calls(calls: &[(&str, &str, serde_json::Value)]) -> Canned {
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
    let mut body = String::new();
    body.push_str(&format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"tool_calls\":[{encoded}]}},\"finish_reason\":null}}]}}\n\n"
    ));
    body.push_str(
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    );
    body.push_str("data: [DONE]\n\n");
    Canned::Sse(body)
}

/// The URL a scene's `provider.yaml` points at. The server binds an ephemeral
/// port, so this is written per run and never fixed.
pub fn endpoint(server: &FakeServer) -> String {
    server.url("/v1/chat/completions")
}
