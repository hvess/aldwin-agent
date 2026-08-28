//! First-launch YAML written to `~/.amundsen/` on a fresh install — a tour of
//! the format in the developer's editor. Each string must parse under its
//! domain's real schema (checked in this module's tests); the exact wording
//! is a separate deliverable from the structure.

pub const PERMISSIONS: &str = "\
# Amundsen permissions — default-deny grants for this machine.
# Entries are \"kind:pattern\" strings, e.g. \"read:**\" or \"shell:git *\".
# allow and deny are kept as two separate lists on purpose: deny always wins,
# regardless of the order entries were added in.
version: 1
allow: []
deny: []
";

pub const PROVIDER: &str = "\
# Amundsen provider settings.
#   provider:                  anthropic | openai-compatible
#   model:                     the model id to use for every request
#   base_url:                  only used when provider is openai-compatible
#   api_key_env:               the NAME of an environment variable holding your
#                              API key — Amundsen never reads or stores the key
#                              itself here, only this variable's name. Export it
#                              before starting Amundsen.
#   extended_thinking_budget:  token budget for extended thinking. Omit to use
#                              amundsen-llm's built-in default.
version: 1
provider: anthropic
model: claude-sonnet-5
api_key_env: ANTHROPIC_API_KEY
";

pub const MCP: &str = "\
# Amundsen MCP server registry.
# Each entry under servers needs a unique name and one of:
#   kind: stdio, command: <path>, args: [...]
#   kind: http,  url: <endpoint>
# A project-scope entry with the same name replaces a global one entirely —
# fields are never merged across scopes.
version: 1
servers: []
";

pub const TUI: &str = "\
# Amundsen TUI preferences (global only — there is no project-scope tui.yaml).
# theme, layout, and keybinds are all optional; omit whatever you don't want
# to override.
version: 1
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;

    #[test]
    fn every_annotated_file_parses_under_its_own_schema() {
        let permissions: PermissionsConfig = serde_yaml_ng::from_str(PERMISSIONS).unwrap();
        assert_eq!(permissions, PermissionsConfig::empty());

        let provider: ProviderConfig = serde_yaml_ng::from_str(PROVIDER).unwrap();
        assert!(provider.has_valid_api_key_env());

        let mcp: McpConfig = serde_yaml_ng::from_str(MCP).unwrap();
        assert_eq!(mcp, McpConfig::empty());

        let tui: TuiConfig = serde_yaml_ng::from_str(TUI).unwrap();
        assert_eq!(tui, TuiConfig::empty());
    }
}
