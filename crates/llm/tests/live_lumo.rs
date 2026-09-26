//! Live smoke test against Proton's Lumo (`lumo-api.proton.me/ai/v1`), the
//! OpenAI-compatible endpoint the unit tests only imitate. Ignored by
//! default — it needs the network and a key:
//!
//! ```sh
//! LUMO_API_KEY=… cargo test -p aldwin-llm --test live_lumo -- --ignored --nocapture
//! ```
//!
//! Point it elsewhere with `LUMO_BASE_URL` / `LUMO_MODEL` to smoke-test any
//! other OpenAI-compatible endpoint through the same adapter.

use aldwin_core::{
    ContentBlock, LlmClient, LlmEvent, LlmRequest, Message, Role, StopReason, ToolDefinition,
};
use aldwin_llm::{Auth, OpenAiCompatibleClient, ProviderConfig};
use futures::StreamExt;
use serde_json::json;

const DEFAULT_BASE_URL: &str = "https://lumo-api.proton.me/ai/v1/chat/completions";
const DEFAULT_MODEL: &str = "lumo-lite";

fn config() -> ProviderConfig {
    ProviderConfig {
        kind: aldwin_config::ProviderKind::OpenaiCompatible,
        model: std::env::var("LUMO_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
        auth: Auth::ApiKeyEnv("LUMO_API_KEY".into()),
        base_url: Some(std::env::var("LUMO_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into())),
        extended_thinking_budget: Some(256),
    }
}

fn client() -> OpenAiCompatibleClient {
    OpenAiCompatibleClient::new(config()).expect("export LUMO_API_KEY before running this test")
}

async fn collect(client: &OpenAiCompatibleClient, request: LlmRequest<'_>) -> Vec<LlmEvent> {
    client
        .stream(request)
        .map(|e| e.unwrap_or_else(|err| panic!("live stream failed: {err}")))
        .collect::<Vec<_>>()
        .await
}

#[tokio::test]
#[ignore = "hits the live Lumo API; needs LUMO_API_KEY"]
async fn text_turn_streams_and_reports_usage() {
    let client = client();
    let messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "Say hello in three words.".into(),
        }],
    }];
    let events = collect(
        &client,
        LlmRequest {
            system: "You are terse.",
            tools: &[],
            messages: &messages,
            cache_breakpoint: None,
        },
    )
    .await;

    let text: String = events
        .iter()
        .filter_map(|e| match e {
            LlmEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let Some(LlmEvent::StepEnded { outcome }) = events.last() else {
        panic!("stream ended without StepEnded: {events:?}")
    };

    println!("text: {text:?}");
    println!("usage: {:?} stop: {:?}", outcome.usage, outcome.stop_reason);
    assert!(!text.is_empty(), "no text streamed");
    assert!(matches!(outcome.stop_reason, StopReason::EndTurn));
    // The regression this whole wiring turned on: Lumo reports usage in a
    // trailing chunk, so a zero here means StepEnded fired too early.
    assert!(
        outcome.usage.input_tokens > 0,
        "no prompt tokens on StepEnded"
    );
    assert!(
        outcome.usage.output_tokens > 0,
        "no completion tokens on StepEnded"
    );
}

#[tokio::test]
#[ignore = "hits the live Lumo API; needs LUMO_API_KEY"]
async fn tool_turn_yields_a_parsed_tool_call() {
    let client = client();
    let tools = vec![ToolDefinition {
        name: "get_weather".into(),
        description: "Get the current weather for a city.".into(),
        input_schema: json!({"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}),
    }];
    let messages = vec![Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "What is the weather in Oslo? Use the tool.".into(),
        }],
    }];
    let events = collect(
        &client,
        LlmRequest {
            system: "Use the provided tools.",
            tools: &tools,
            messages: &messages,
            cache_breakpoint: None,
        },
    )
    .await;

    let call = events
        .iter()
        .find_map(|e| match e {
            LlmEvent::ToolUseRequested { call } => Some(call),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no tool call in {events:?}"));

    println!("tool call: {} {} {}", call.id, call.name, call.input);
    assert_eq!(call.name, "get_weather");
    assert_eq!(call.input["city"], "Oslo");
    let Some(LlmEvent::StepEnded { outcome }) = events.last() else {
        panic!("stream ended without StepEnded")
    };
    assert!(matches!(outcome.stop_reason, StopReason::ToolUse));
    assert!(
        outcome.usage.input_tokens > 0,
        "no prompt tokens on StepEnded"
    );
}
