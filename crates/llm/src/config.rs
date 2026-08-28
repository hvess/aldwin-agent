use amundsen_config::ProviderKind;

/// Used when neither project nor global provider.yaml sets
/// `extended_thinking_budget` — extended thinking is enabled by default per
/// amundsen-llm.md, so a default has to live somewhere even when the
/// developer hasn't picked one.
pub const DEFAULT_THINKING_BUDGET: u32 = 10_000;

/// amundsen-llm's own resolved view of provider config — distinct from
/// `amundsen_config::ProviderConfig` (the raw on-disk domain shape). Built by
/// [`resolve`]; never round-trips back to YAML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub kind:                     ProviderKind,
    pub model:                    String,
    pub api_key_env:              String,
    /// V0.5 — only used by the (not yet implemented) OpenAI-compatible adapter.
    pub base_url:                 Option<String>,
    pub extended_thinking_budget: u32,
}

/// Flat project-over-global overlay: a project-scope provider.yaml, when
/// present, wins wholesale for the required fields (`provider`/`model`/
/// `api_key_env` — amundsen-config's schema makes a project file all-or-
/// nothing for those, since they're not `Option`), but the two optional
/// fields overlay individually, falling back to global's value.
pub fn resolve(project: Option<&amundsen_config::ProviderConfig>, global: &amundsen_config::ProviderConfig) -> ProviderConfig {
    let required_source = project.unwrap_or(global);
    ProviderConfig {
        kind:        required_source.provider,
        model:       required_source.model.clone(),
        api_key_env: required_source.api_key_env.clone(),
        base_url:    project.and_then(|p| p.base_url.clone()).or_else(|| global.base_url.clone()),
        extended_thinking_budget: project
            .and_then(|p| p.extended_thinking_budget)
            .or(global.extended_thinking_budget)
            .unwrap_or(DEFAULT_THINKING_BUDGET),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amundsen_config::ProviderConfig as RawProviderConfig;

    fn raw(model: &str, base_url: Option<&str>, budget: Option<u32>) -> RawProviderConfig {
        RawProviderConfig {
            version: 1,
            provider: ProviderKind::Anthropic,
            model: model.into(),
            base_url: base_url.map(String::from),
            api_key_env: "ANTHROPIC_API_KEY".into(),
            extended_thinking_budget: budget,
        }
    }

    #[test]
    fn no_project_scope_uses_global_wholesale() {
        let global = raw("claude-sonnet-5", Some("https://x"), Some(5_000));
        let resolved = resolve(None, &global);
        assert_eq!(resolved.model, "claude-sonnet-5");
        assert_eq!(resolved.base_url.as_deref(), Some("https://x"));
        assert_eq!(resolved.extended_thinking_budget, 5_000);
    }

    #[test]
    fn project_scope_required_fields_win_wholesale() {
        let global = raw("global-model", None, None);
        let project = raw("project-model", None, None);
        let resolved = resolve(Some(&project), &global);
        assert_eq!(resolved.model, "project-model");
    }

    #[test]
    fn optional_fields_overlay_individually_from_global() {
        // Project sets nothing optional; both should fall back to global.
        let global = raw("g", Some("https://global"), Some(20_000));
        let project = raw("p", None, None);
        let resolved = resolve(Some(&project), &global);
        assert_eq!(resolved.base_url.as_deref(), Some("https://global"));
        assert_eq!(resolved.extended_thinking_budget, 20_000);
    }

    #[test]
    fn optional_fields_set_at_project_scope_override_global() {
        let global = raw("g", Some("https://global"), Some(20_000));
        let project = raw("p", Some("https://project"), Some(1_000));
        let resolved = resolve(Some(&project), &global);
        assert_eq!(resolved.base_url.as_deref(), Some("https://project"));
        assert_eq!(resolved.extended_thinking_budget, 1_000);
    }

    #[test]
    fn missing_budget_everywhere_falls_back_to_the_built_in_default() {
        let global = raw("g", None, None);
        let resolved = resolve(None, &global);
        assert_eq!(resolved.extended_thinking_budget, DEFAULT_THINKING_BUDGET);
    }
}
