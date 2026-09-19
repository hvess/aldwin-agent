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

/// A canned-response HTTP server, used by this crate's own tests and — behind
/// the `test-server` feature — by the screenshot harness, which needs the TUI
/// driven by a provider that answers the same way every run.
///
/// The feature exists so the fake never reaches the shipped binary: nothing
/// enables it except a dev-only crate. Note that Cargo unifies features across
/// one build graph, so a `cargo build --workspace` does compile it into
/// `mjolnir-cli`; the release workflow builds `-p mjolnir-cli`, where it stays
/// off.
#[cfg(any(test, feature = "test-server"))]
pub mod test_server;

pub use catalog::{identify, provider, provider_ids, Model, Provider, CURATED, PROVIDERS};
pub use client::{AnthropicClient, LlmClientInitError};
pub use client_openai::OpenAiCompatibleClient;
pub use config::{resolve, ProviderConfig};
