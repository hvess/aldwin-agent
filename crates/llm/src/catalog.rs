//! The provider catalogue — what the provider question (the first message
//! with nothing configured, or bare `/model`) chooses between.
//!
//! It lives here rather than in aldwin-tui or aldwin-cli because every
//! field in it is knowledge this crate already owns: which wire dialect a
//! host speaks, what its chat-completions URL is, and which environment
//! variable holds its key. aldwin-tui is handed the display half of these
//! rows (id and purpose) by the CLI rather than depending on this crate,
//! which would invert the workspace's dependency order.
//!
//! **The model lists are seeds, not a ceiling.** A provider's real catalogue
//! is a network call away and changes without us; `provider.yaml` takes any
//! model id as a plain string, and so does `/model`. What is listed here is
//! what the harness will *suggest*, and the first entry is what `/model
//! provider` writes when the developer names a provider and nothing else.
//!
//! Every entry names an environment variable. A provider that needs no key
//! at all — a local Ollama, say — has no representation here, because
//! `api_key_env` is a required field of `provider.yaml` and
//! `OpenAiCompatibleClient` refuses to start when the variable it names is
//! unset. Supporting one means relaxing that field to an `Option`, which
//! changes a persisted format and so wants its own decision record first.

use aldwin_config::ProviderKind;
use aldwin_login::Account;

/// One model on offer, in the shared 16-cell option row: the id that lands
/// in `provider.yaml`, and what picking it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    /// The model id as the provider's API takes it, written verbatim to
    /// `provider.yaml`.
    pub id: &'static str,
    /// What picking this model does, in the option row's words.
    pub purpose: &'static str,
    /// The model's context window in tokens, for the context bar. A seed
    /// like the rest of the row: a provider can change it without us, and
    /// the bar is a gauge, not an accounting.
    pub context: u32,
}

/// One provider on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider {
    /// The lowercase name in the option's 16-cell field, and the word
    /// `/model` takes before the `/`.
    pub id: &'static str,
    /// The wire dialect the provider speaks, which picks the client.
    pub kind: ProviderKind,
    /// What picking it does — the design's own row copy, which names the
    /// models and the key variable because both are what the developer
    /// needs before they can choose.
    pub purpose: &'static str,
    /// The environment variable that holds the provider's API key, written
    /// to `provider.yaml` as `api_key_env`.
    pub api_key_env: &'static str,
    /// The account a developer may connect instead of exporting a key
    /// (ADR 0012), where the provider offers one. A connected account is
    /// used before the key.
    pub account: Option<Account>,
    /// The full chat-completions URL, used verbatim — not a prefix. `None`
    /// for Anthropic, whose client has a single well-known endpoint.
    pub base_url: Option<&'static str>,
    /// The models suggested for this provider, the default first. Seeds,
    /// not a ceiling: any model id is accepted.
    pub models: &'static [Model],
}

impl Provider {
    /// What `/model provider` writes when the developer names this provider
    /// and says nothing about a model. Never empty:
    /// `every_provider_offers_a_model` pins that.
    pub fn default_model(&self) -> &'static str {
        self.models[0].id
    }
}

/// In the order the provider question lists them.
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

/// The catalogue entry for `id`, or `None` — the check `/model` runs before
/// it will write a provider name into `provider.yaml`.
pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Every provider id, in catalogue order — for a command that has to say
/// what it would have accepted.
pub fn provider_ids() -> Vec<&'static str> {
    PROVIDERS.iter().map(|p| p.id).collect()
}

/// The catalogue entry whose `kind` and `base_url` match a `provider.yaml`'s,
/// or `None` when the developer has hand-written an endpoint the catalogue
/// has never heard of.
///
/// Matching on the endpoint rather than on a name stored in the file is
/// deliberate: `provider.yaml` records what to *call*, not which row of a
/// menu was clicked, and adding a name field would create a second source
/// of truth that could disagree with the URL beside it.
pub fn identify(kind: ProviderKind, base_url: Option<&str>) -> Option<&'static Provider> {
    PROVIDERS
        .iter()
        .find(|p| p.kind == kind && p.base_url == base_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The context bar divides by this, so a zero would be a bar that never
    /// moves — or a divide by zero, depending on who reads it.
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

    /// `default_model()` indexes `[0]`, so an empty list would panic at
    /// `/model provider` rather than at compile time.
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

    /// Ids reach `/model` as the half before a `/`, and land in
    /// `provider.yaml` — a duplicate would make `provider()` silently
    /// prefer whichever came first.
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

    /// `OpenAiCompatibleClient::new` errors without a `base_url`, and
    /// `AnthropicClient` has its own endpoint and would ignore one.
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

    /// An account is offered beside a key, never instead of one: the
    /// key-less row would be a provider that cannot be reached without an
    /// account.
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

    /// Every entry names an environment variable, and never a key: there is
    /// deliberately no field a secret could travel in.
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

    /// The endpoint is what identifies a provider, so two rows sharing one
    /// would make `identify` ambiguous.
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

    /// These rows are drawn on the design system's shared option row: a
    /// `▌` and two spaces at cell 13, a 16-cell name field, then the
    /// purpose, all inside a 120-cell frame with a 3-cell right margin.
    /// Nothing here can see that layout, so the budget is restated rather
    /// than imported — a row that overflows it is a row the frame elides.
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
