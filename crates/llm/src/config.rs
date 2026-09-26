use std::sync::Arc;

use aldwin_config::ProviderKind;
use aldwin_login::Session;

/// Used when `provider.yaml` sets no `extended_thinking_budget` — extended
/// thinking is enabled by default per aldwin-llm.md, so a default has to
/// live somewhere even when the developer hasn't picked one.
const DEFAULT_THINKING_BUDGET: u32 = 10_000;

/// How the provider is reached (ADR 0012): the key in the environment
/// variable `provider.yaml` names, or the account the developer connected
/// — already read from disk and made a session by the composition root,
/// which also decides between the two, so this crate knows nothing of
/// files.
#[derive(Debug, Clone)]
pub enum Auth {
    ApiKeyEnv(String),
    Connection(Arc<Session>),
}

/// What a client is built from: the provider settings in force, with the
/// project and global `provider.yaml` already overlaid by aldwin-config
/// (`Config::effective_provider`). Distinct from `aldwin_config::ProviderConfig`,
/// the on-disk shape, so this crate knows nothing of files or scopes.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub model: String,
    pub auth: Auth,
    /// V0.5 — only used by the OpenAI-compatible adapter (`OpenAiCompatibleClient`).
    pub base_url: Option<String>,
    /// `None` takes this crate's default.
    pub extended_thinking_budget: Option<u32>,
}

impl ProviderConfig {
    /// The budget in force — the developer's, or the default.
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
