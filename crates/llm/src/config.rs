use std::sync::Arc;

use aldwin_config::ProviderKind;
use aldwin_login::Session;

/// Used when `provider.yaml` sets no `extended_thinking_budget`; thinking is
/// on by default (aldwin-llm.md).
const DEFAULT_THINKING_BUDGET: u32 = 10_000;

/// How the provider is reached (ADR 0012): an API key from the environment,
/// or a connected account's session. The composition root reads the files
/// and chooses; this crate knows nothing of files.
#[derive(Debug, Clone)]
pub enum Auth {
    /// The name of the environment variable that holds the API key.
    ApiKeyEnv(String),
    /// The connected account's session, tried before a key.
    Connection(Arc<Session>),
}

/// The provider settings a client is built from, already overlaid by
/// `Config::effective_provider`. Not `aldwin_config::ProviderConfig`, the
/// on-disk shape: this crate knows nothing of files or scopes.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// The wire dialect, which picks the client.
    pub kind: ProviderKind,
    /// The model id sent with every request.
    pub model: String,
    /// A key or a connected account.
    pub auth: Auth,
    /// Read only by `OpenAiCompatibleClient`.
    pub base_url: Option<String>,
    /// `None` takes this crate's default.
    pub extended_thinking_budget: Option<u32>,
}

impl ProviderConfig {
    /// The configured budget, or `DEFAULT_THINKING_BUDGET`.
    pub(crate) fn thinking_budget(&self) -> u32 {
        self.extended_thinking_budget
            .unwrap_or(DEFAULT_THINKING_BUDGET)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_left_unset_takes_the_built_in_default() {
        let config = ProviderConfig {
            kind: ProviderKind::Anthropic,
            model: "m".into(),
            auth: Auth::ApiKeyEnv("K".into()),
            base_url: None,
            extended_thinking_budget: None,
        };
        assert_eq!(config.thinking_budget(), DEFAULT_THINKING_BUDGET);
        let set = ProviderConfig {
            extended_thinking_budget: Some(1_000),
            ..config
        };
        assert_eq!(set.thinking_budget(), 1_000);
    }
}
