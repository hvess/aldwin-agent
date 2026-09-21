//! The provider clients implementing core's `LlmClient` trait, and the
//! provider catalogue. See `.claude/spec/archive/aldwin-llm.md`. Every wire
//! type stays in the private `wire` / `wire_openai` modules — nothing
//! provider-shaped is part of this crate's public surface.

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
/// `aldwin-cli`; the release workflow builds `-p aldwin-cli`, where it stays
/// off.
#[cfg(any(test, feature = "test-server"))]
pub mod test_server;

pub use catalog::{identify, provider, provider_ids, Model, Provider, CURATED, PROVIDERS};
pub use client::{AnthropicClient, LlmClientInitError};
pub use client_openai::OpenAiCompatibleClient;
pub use config::{resolve, ProviderConfig};
