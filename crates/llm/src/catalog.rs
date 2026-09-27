//! The provider catalogue the provider question and bare `/model` choose
//! from. aldwin-tui gets the display half (id, purpose) from the CLI; it must
//! not depend on this crate, which would invert the dependency order.
//!
//! Model lists are suggestions: `provider.yaml` and `/model` accept any id.
//! A key-less provider (a local Ollama) cannot be listed: `api_key_env` is
//! required, and making it optional changes a persisted format (needs an ADR).

use aldwin_config::ProviderKind;
use aldwin_login::Account;

/// One model on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    /// The API's model id, written verbatim to `provider.yaml`.
    pub id: &'static str,
    /// The option row's description.
    pub purpose: &'static str,
    /// Context window in tokens, for the context bar. Approximate: the
    /// provider may change it.
    pub context: u32,
}

/// One provider on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider {
    /// Lowercase, under 16 cells; the word before the `/` in `/model`.
    pub id: &'static str,
    /// The wire dialect, which picks the client.
    pub kind: ProviderKind,
    /// The option row's description: the models and the key variable.
    pub purpose: &'static str,
    /// The key's environment variable, written as `api_key_env`.
    pub api_key_env: &'static str,
    /// The account that may be connected instead of a key, tried first
    /// (ADR 0012).
    pub account: Option<Account>,
    /// The full chat-completions URL, posted to verbatim. `None` for
    /// Anthropic, whose client has a fixed endpoint.
    pub base_url: Option<&'static str>,
    /// Suggested models, the default first; any id is accepted.
    pub models: &'static [Model],
}

impl Provider {
    /// The model `/model provider` writes when no model is named. `models`
    /// is never empty (`every_provider_offers_a_model`).
    pub fn default_model(&self) -> &'static str {
        self.models[0].id
    }
}

/// The providers, in the order the provider question lists them.
pub static PROVIDERS: &[Provider] = &[
    Provider {
        id: "anthropic",
        kind: ProviderKind::Anthropic,
        purpose: "claude models · ANTHROPIC_API_KEY",
        api_key_env: "ANTHROPIC_API_KEY",
        account: None,
        base_url: None,
        models: &[
            Model {
                id: "claude-sonnet-5",
                purpose: "balanced; a good default",
                context: 1_000_000,
            },
            Model {
                id: "claude-opus-5",
                purpose: "slower, deeper",
                context: 1_000_000,
            },
            Model {
                id: "claude-haiku-4-5-20251001",
                purpose: "fast, cheap",
                context: 200_000,
            },
        ],
    },
    Provider {
        id: "google",
        kind: ProviderKind::OpenaiCompatible,
        purpose: "gemini models · GOOGLE_API_KEY",
        api_key_env: "GOOGLE_API_KEY",
        account: None,
        base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"),
        models: &[
            Model {
                id: "gemini-2.5-pro",
                purpose: "balanced; a good default",
                context: 1_048_576,
            },
            Model {
                id: "gemini-2.5-flash",
                purpose: "fast, cheap",
                context: 1_048_576,
            },
        ],
    },
    Provider {
        id: "openai",
        kind: ProviderKind::OpenaiCompatible,
        purpose: "gpt models · OPENAI_API_KEY",
        api_key_env: "OPENAI_API_KEY",
        account: None,
        base_url: Some("https://api.openai.com/v1/chat/completions"),
        models: &[
            Model {
                id: "gpt-5",
                purpose: "balanced; a good default",
                context: 400_000,
            },
            Model {
                id: "gpt-5-mini",
                purpose: "fast, cheap",
                context: 400_000,
            },
        ],
    },
    Provider {
        id: "xai",
        kind: ProviderKind::OpenaiCompatible,
        purpose: "grok models · XAI_API_KEY, or /connect",
        api_key_env: "XAI_API_KEY",
        account: Some(Account::Xai),
        base_url: Some("https://api.x.ai/v1/chat/completions"),
        models: &[
            Model {
                id: "grok-4.7",
                purpose: "balanced; a good default",
                context: 500_000,
            },
            Model {
                id: "grok-4.3",
                purpose: "fast, cheap",
                context: 1_000_000,
            },
        ],
    },
    Provider {
        id: "lumo",
        kind: ProviderKind::OpenaiCompatible,
        purpose: "proton lumo · LUMO_API_KEY",
        api_key_env: "LUMO_API_KEY",
        account: None,
        base_url: Some("https://lumo-api.proton.me/ai/v1/chat/completions"),
        models: &[
            Model {
                id: "lumo-max",
                purpose: "reasoning; 131k context",
                context: 131_072,
            },
            Model {
                id: "lumo-lite",
                purpose: "faster; 262k context",
                context: 262_144,
            },
        ],
    },
    Provider {
        id: "mistral",
        kind: ProviderKind::OpenaiCompatible,
        purpose: "mistral models · MISTRAL_API_KEY",
        api_key_env: "MISTRAL_API_KEY",
        account: None,
        base_url: Some("https://api.mistral.ai/v1/chat/completions"),
        models: &[
            Model {
                id: "mistral-large-latest",
                purpose: "balanced; a good default",
                context: 128_000,
            },
            Model {
                id: "mistral-small-latest",
                purpose: "fast, cheap",
                context: 128_000,
            },
        ],
    },
    Provider {
        id: "deepseek",
        kind: ProviderKind::OpenaiCompatible,
        purpose: "deepseek models · DEEPSEEK_API_KEY",
        api_key_env: "DEEPSEEK_API_KEY",
        account: None,
        base_url: Some("https://api.deepseek.com/v1/chat/completions"),
        models: &[
            Model {
                id: "deepseek-chat",
                purpose: "balanced; a good default",
                context: 128_000,
            },
            Model {
                id: "deepseek-reasoner",
                purpose: "slower, deeper",
                context: 128_000,
            },
        ],
    },
];

/// The catalogue entry for `id`; `/model` checks it before writing a
/// provider.
pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Every provider id, in catalogue order.
pub fn provider_ids() -> Vec<&'static str> {
    PROVIDERS.iter().map(|p| p.id).collect()
}

/// The catalogue entry matching a `provider.yaml`'s `kind` and `base_url`;
/// `None` for a hand-written endpoint.
///
/// Do not store a provider name in `provider.yaml` instead: it would be a
/// second source of truth that could disagree with the URL.
pub fn identify(kind: ProviderKind, base_url: Option<&str>) -> Option<&'static Provider> {
    PROVIDERS
        .iter()
        .find(|p| p.kind == kind && p.base_url == base_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The context bar divides by `context`.
    #[test]
    fn every_model_states_a_context_window() {
        for p in PROVIDERS {
            for m in p.models {
                assert!(
                    m.context >= 8_000,
                    "{}/{} has an implausible context window {}",
                    p.id,
                    m.id,
                    m.context
                );
            }
        }
    }

    /// `default_model()` indexes `[0]`.
    #[test]
    fn every_provider_offers_a_model() {
        for p in PROVIDERS {
            assert!(
                !p.models.is_empty(),
                "{} offers no model, so /model has nothing to write",
                p.id
            );
        }
    }

    /// `/model` splits on `/`, and `provider()` takes the first match.
    #[test]
    fn provider_ids_are_unique_and_lowercase() {
        let mut seen = std::collections::BTreeSet::new();
        for p in PROVIDERS {
            assert!(seen.insert(p.id), "duplicate provider id {}", p.id);
            assert_eq!(
                p.id,
                p.id.to_ascii_lowercase(),
                "{} must be lowercase",
                p.id
            );
            assert!(
                !p.id.contains('/'),
                "{} would split ambiguously in /model",
                p.id
            );
        }
    }

    /// `OpenAiCompatibleClient::new` requires `base_url`; `AnthropicClient`
    /// ignores it.
    #[test]
    fn only_the_openai_compatible_entries_carry_an_endpoint() {
        for p in PROVIDERS {
            match p.kind {
                ProviderKind::Anthropic => {
                    assert!(p.base_url.is_none(), "{} needs no base_url", p.id)
                }
                ProviderKind::OpenaiCompatible => {
                    let url = p
                        .base_url
                        .expect("an openai-compatible provider needs an endpoint");
                    assert!(
                        url.ends_with("/chat/completions"),
                        "{url} must be the full chat-completions URL, not a prefix — the client posts to it verbatim"
                    );
                }
            }
        }
    }

    /// An account is offered beside a key, never instead of one (the key is
    /// pinned by `every_provider_names_a_key_variable`).
    #[test]
    fn an_account_is_offered_beside_a_key_and_xai_offers_one() {
        for p in PROVIDERS {
            if let Some(account) = p.account {
                assert_eq!(account.id(), p.id, "the account is keyed by the row's id");
                assert_eq!(p.kind, ProviderKind::OpenaiCompatible);
            }
        }
        assert_eq!(provider("xai").unwrap().account, Some(Account::Xai));
    }

    /// A variable name, never a key: no field may carry a secret.
    #[test]
    fn every_provider_names_a_key_variable() {
        for p in PROVIDERS {
            assert!(
                !p.api_key_env.trim().is_empty(),
                "{} must name a key variable",
                p.id
            );
            assert_eq!(
                p.api_key_env,
                p.api_key_env.to_ascii_uppercase(),
                "{} names a variable, not a value",
                p.id
            );
        }
    }

    /// `identify` matches on the endpoint.
    #[test]
    fn no_two_providers_share_an_endpoint() {
        let mut seen: Vec<(ProviderKind, Option<&str>)> = Vec::new();
        for p in PROVIDERS {
            let key = (p.kind, p.base_url);
            assert!(
                !seen.contains(&key),
                "{} is indistinguishable from an earlier row on disk",
                p.id
            );
            seen.push(key);
        }
    }

    /// The design's option row: a `▌` and two spaces at cell 13, a 16-cell
    /// name field, the purpose, in a 120-cell frame with a 3-cell right
    /// margin. Restated here, as this crate cannot import the TUI's layout.
    #[test]
    fn every_row_fits_the_option_row_it_is_drawn_on() {
        const NAME_FIELD: usize = 16;
        const BODY_COLUMN: usize = 120 - 13 - 3;
        for p in PROVIDERS {
            assert!(
                p.id.chars().count() < NAME_FIELD,
                "{} does not fit the 16-cell name field",
                p.id
            );
            let row = 3 + NAME_FIELD + p.purpose.chars().count();
            assert!(
                row <= BODY_COLUMN,
                "{}'s row is {row} cells, past the {BODY_COLUMN} the body column has",
                p.id
            );
        }
    }

    #[test]
    fn identify_recovers_the_catalogue_row_from_a_written_config() {
        let google = provider("google").unwrap();
        assert_eq!(
            identify(google.kind, google.base_url).map(|p| p.id),
            Some("google")
        );
    }

    #[test]
    fn identify_returns_none_for_a_hand_written_endpoint() {
        assert_eq!(
            identify(
                ProviderKind::OpenaiCompatible,
                Some("http://localhost:8000/v1/chat/completions")
            ),
            None,
            "a developer's own endpoint must not be reported as a catalogue provider"
        );
    }
}
