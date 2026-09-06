//! V0 Anthropic client implementing core's `LlmClient` trait. See
//! `.claude/spec/mjolnir-llm.md`. All Anthropic wire types stay in the
//! private `wire` module — only `AnthropicClient`, `ProviderConfig`, and
//! core's own `LlmError` are part of this crate's public surface.

mod catalog;
mod client;
mod client_openai;
mod config;
mod retry;
mod wire;
mod wire_openai;

#[cfg(test)]
mod test_server;

pub use catalog::{identify, provider, provider_ids, Model, Provider, CURATED, PROVIDERS};
pub use client::{AnthropicClient, LlmClientInitError};
pub use client_openai::OpenAiCompatibleClient;
pub use config::{resolve, ProviderConfig};
