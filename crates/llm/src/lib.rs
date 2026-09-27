//! Provider clients implementing core's `LlmClient`, and the provider
//! catalogue (`docs/spec/archive/aldwin-llm.md`). Wire types stay in the
//! private `wire` / `wire_openai` modules: nothing provider-shaped is public.

mod catalog;
mod client;
mod client_openai;
mod config;
mod retry;
mod transport;
mod wire;
mod wire_openai;

/// A canned-response HTTP server for this crate's tests and, behind the
/// `test-server` feature, the screenshot harness.
///
/// Only a dev-only crate may enable the feature, so the fake stays out of the
/// shipped binary. Cargo unifies features: `cargo build --workspace` compiles
/// it into `aldwin-cli`; the release build (`-p aldwin-cli`) does not.
#[cfg(any(test, feature = "test-server"))]
pub mod test_server;

pub use catalog::{identify, provider, provider_ids, Model, Provider, PROVIDERS};
pub use client::{AnthropicClient, LlmClientInitError};
pub use client_openai::OpenAiCompatibleClient;
pub use config::{Auth, ProviderConfig};
