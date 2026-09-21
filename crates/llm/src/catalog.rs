//! The provider catalogue — what first run's `provider` step and `/model`
//! choose between.
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
//! what the harness will *suggest*, and the first entry is what first run
//! writes when the developer picks that provider and nothing else.
//!
//! Every entry names an environment variable. A provider that needs no key
//! at all — a local Ollama, say — has no representation here, because
//! `api_key_env` is a required field of `provider.yaml` and
//! `OpenAiCompatibleClient` refuses to start when the variable it names is
//! unset. Supporting one means relaxing that field to an `Option`, which
//! changes a persisted format and so wants its own decision record first.

use aldwin_config::ProviderKind;

/// One model on offer, in the shared 16-cell option row: the id that lands
/// in `provider.yaml`, and what picking it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    pub id:      &'static str,
    pub purpose: &'static str,
}

/// One provider on offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider {
    /// The lowercase name in the option's 16-cell field, and the word
    /// `/model` takes before the `/`.
    pub id:          &'static str,
    pub kind:        ProviderKind,
    /// What picking it does — the design's own row copy, which names the
    /// models and the key variable because both are what the developer
    /// needs before they can choose.
    pub purpose:     &'static str,
    pub api_key_env: &'static str,
    /// The full chat-completions URL, used verbatim — not a prefix. `None`
    /// for Anthropic, whose client has a single well-known endpoint.
    pub base_url:    Option<&'static str>,
    pub models:      &'static [Model],
}

impl Provider {
    /// What first run writes when the developer picks this provider and
    /// says nothing about a model. Never empty: `every_provider_offers_a_model`
    /// pins that.
    pub fn default_model(&self) -> &'static str {
        self.models[0].id
    }

    pub fn offers_model(&self, id: &str) -> bool {
        self.models.iter().any(|m| m.id == id)
    }
}

/// How many of [`PROVIDERS`] first run shows before the `more` row. The
/// design's `5d` shows a short list and hangs the rest behind one row, so
/// the first question is answerable without reading a catalogue.
pub const CURATED: usize = 3;

/// Ordered: the curated rows first, then everything the `more` row reveals.
pub static PROVIDERS: &[Provider] = &[
    Provider {
        id:          "anthropic",
        kind:        ProviderKind::Anthropic,
        purpose:     "claude models · ANTHROPIC_API_KEY",
        api_key_env: "ANTHROPIC_API_KEY",
        base_url:    None,
        models:      &[
            Model { id: "claude-sonnet-5", purpose: "balanced; a good default" },
            Model { id: "claude-opus-5", purpose: "slower, deeper" },
            Model { id: "claude-haiku-4-5-20251001", purpose: "fast, cheap" },
        ],
    },
    Provider {
        id:          "google",
        kind:        ProviderKind::OpenaiCompatible,
        purpose:     "gemini models · GOOGLE_API_KEY",
        api_key_env: "GOOGLE_API_KEY",
        base_url:    Some("https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"),
        models:      &[
            Model { id: "gemini-2.5-pro", purpose: "balanced; a good default" },
            Model { id: "gemini-2.5-flash", purpose: "fast, cheap" },
        ],
    },
    Provider {
        id:          "openai",
        kind:        ProviderKind::OpenaiCompatible,
        purpose:     "gpt models · OPENAI_API_KEY",
        api_key_env: "OPENAI_API_KEY",
        base_url:    Some("https://api.openai.com/v1/chat/completions"),
        models:      &[
            Model { id: "gpt-5", purpose: "balanced; a good default" },
            Model { id: "gpt-5-mini", purpose: "fast, cheap" },
        ],
    },
    // Everything below here is behind the `more` row.
    Provider {
        id:          "lumo",
        kind:        ProviderKind::OpenaiCompatible,
        purpose:     "proton lumo · LUMO_API_KEY",
        api_key_env: "LUMO_API_KEY",
        base_url:    Some("https://lumo-api.proton.me/ai/v1/chat/completions"),
        models:      &[
            Model { id: "lumo-max", purpose: "reasoning; 131k context" },
            Model { id: "lumo-lite", purpose: "faster; 262k context" },
        ],
    },
    Provider {
        id:          "mistral",
        kind:        ProviderKind::OpenaiCompatible,
        purpose:     "mistral models · MISTRAL_API_KEY",
        api_key_env: "MISTRAL_API_KEY",
        base_url:    Some("https://api.mistral.ai/v1/chat/completions"),
        models:      &[
            Model { id: "mistral-large-latest", purpose: "balanced; a good default" },
            Model { id: "mistral-small-latest", purpose: "fast, cheap" },
        ],
    },
    Provider {
        id:          "deepseek",
        kind:        ProviderKind::OpenaiCompatible,
        purpose:     "deepseek models · DEEPSEEK_API_KEY",
        api_key_env: "DEEPSEEK_API_KEY",
        base_url:    Some("https://api.deepseek.com/v1/chat/completions"),
        models:      &[
            Model { id: "deepseek-chat", purpose: "balanced; a good default" },
            Model { id: "deepseek-reasoner", purpose: "slower, deeper" },
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

/// The catalogue entry whose `kind`, `base_url` and `api_key_env` match a
/// `provider.yaml` already on disk, or `None` when the developer has
/// hand-written an endpoint the catalogue has never heard of.
///
/// Matching on the endpoint rather than on a name stored in the file is
/// deliberate: `provider.yaml` records what to *call*, not which row of a
/// menu was clicked, and adding a name field would create a second source
/// of truth that could disagree with the URL beside it.
pub fn identify(config: &aldwin_config::ProviderConfig) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.kind == config.provider && p.base_url == config.base_url.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `default_model()` indexes `[0]`, so an empty list would panic on
    /// first run rather than at compile time.
    #[test]
    fn every_provider_offers_a_model() {
        for p in PROVIDERS {
            assert!(!p.models.is_empty(), "{} offers no model, so first run has nothing to write", p.id);
        }
    }

    /// The curated rows are a prefix of the catalogue, not a separate list —
    /// first run shows `PROVIDERS[..CURATED]` and then everything.
    #[test]
    fn the_curated_rows_are_a_prefix_and_leave_something_behind_more() {
        assert!(CURATED < PROVIDERS.len(), "the `more` row must reveal something");
        assert_eq!(PROVIDERS[0].id, "anthropic", "the curated list opens on the provider the harness was built against");
    }

    /// Ids reach `/model` as the half before a `/`, and land in
    /// `provider.yaml` — a duplicate would make `provider()` silently
    /// prefer whichever came first.
    #[test]
    fn provider_ids_are_unique_and_lowercase() {
        let mut seen = std::collections::BTreeSet::new();
        for p in PROVIDERS {
            assert!(seen.insert(p.id), "duplicate provider id {}", p.id);
            assert_eq!(p.id, p.id.to_ascii_lowercase(), "{} must be lowercase", p.id);
            assert!(!p.id.contains('/'), "{} would split ambiguously in /model", p.id);
        }
    }

    /// `OpenAiCompatibleClient::new` errors without a `base_url`, and
    /// `AnthropicClient` has its own endpoint and would ignore one.
    #[test]
    fn only_the_openai_compatible_entries_carry_an_endpoint() {
        for p in PROVIDERS {
            match p.kind {
                ProviderKind::Anthropic => assert!(p.base_url.is_none(), "{} needs no base_url", p.id),
                ProviderKind::OpenaiCompatible => {
                    let url = p.base_url.expect("an openai-compatible provider needs an endpoint");
                    assert!(
                        url.ends_with("/chat/completions"),
                        "{url} must be the full chat-completions URL, not a prefix — the client posts to it verbatim"
                    );
                }
            }
        }
    }

    /// Every entry names an environment variable, and never a key: there is
    /// deliberately no field a secret could travel in.
    #[test]
    fn every_provider_names_a_key_variable() {
        for p in PROVIDERS {
            assert!(!p.api_key_env.trim().is_empty(), "{} must name a key variable", p.id);
            assert_eq!(p.api_key_env, p.api_key_env.to_ascii_uppercase(), "{} names a variable, not a value", p.id);
        }
    }

    /// The endpoint is what identifies a provider, so two rows sharing one
    /// would make `identify` ambiguous.
    #[test]
    fn no_two_providers_share_an_endpoint() {
        let mut seen: Vec<(ProviderKind, Option<&str>)> = Vec::new();
        for p in PROVIDERS {
            let key = (p.kind, p.base_url);
            assert!(!seen.contains(&key), "{} is indistinguishable from an earlier row on disk", p.id);
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
            assert!(p.id.chars().count() < NAME_FIELD, "{} does not fit the 16-cell name field", p.id);
            let row = 3 + NAME_FIELD + p.purpose.chars().count();
            assert!(row <= BODY_COLUMN, "{}'s row is {row} cells, past the {BODY_COLUMN} the body column has", p.id);
        }
    }

    /// First run draws the whole catalogue once `more` is taken, on a frame
    /// that is 36 rows and does not scroll. Everything but the provider
    /// options is fixed: 3 top bar, 4 wordmark block, 3 gap, 2 for the
    /// provider prose and its blank row, 3 gap, 6 for the access step, 3
    /// footer — 24 rows, leaving 12.
    #[test]
    fn the_whole_catalogue_fits_the_expanded_first_run_screen() {
        assert!(PROVIDERS.len() <= 12, "{} providers would be clipped by the 36-row frame", PROVIDERS.len());
    }

    #[test]
    fn identify_recovers_the_catalogue_row_from_a_written_config() {
        let anthropic = provider("anthropic").unwrap();
        let written = aldwin_config::ProviderConfig {
            version:                  aldwin_config::PROVIDER_VERSION,
            provider:                 anthropic.kind,
            model:                    anthropic.default_model().into(),
            base_url:                 anthropic.base_url.map(String::from),
            api_key_env:              anthropic.api_key_env.into(),
            extended_thinking_budget: None,
        };
        assert_eq!(identify(&written).map(|p| p.id), Some("anthropic"));
    }

    #[test]
    fn identify_returns_none_for_a_hand_written_endpoint() {
        let written = aldwin_config::ProviderConfig {
            version:                  aldwin_config::PROVIDER_VERSION,
            provider:                 ProviderKind::OpenaiCompatible,
            model:                    "qwen3-coder".into(),
            base_url:                 Some("http://localhost:8000/v1/chat/completions".into()),
            api_key_env:              "VLLM_API_KEY".into(),
            extended_thinking_budget: None,
        };
        assert_eq!(identify(&written), None, "a developer's own endpoint must not be reported as a catalogue provider");
    }
}
