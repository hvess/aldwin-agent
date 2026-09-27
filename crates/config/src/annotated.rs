//! Annotated YAML seeded into `~/.aldwin/` on first launch. Each file must
//! parse under its domain's schema (this module's tests).
//!
//! A domain Aldwin writes to (provider, tui, connections) has a `*_HEADER`
//! comment block that `Config::with_domain_mut` prepends to every write.
//! `permissions.yaml` and `mcp.yaml` are never written after first launch.

pub const PERMISSIONS: &str = "\
# Aldwin permissions. There is one boundary, and it is the workspace: reads
# and runs need no permission, every edit is reviewed before it is written,
# and nothing Aldwin runs can write outside the workspace.
#
# The workspace is the project directory, plus any roots the PROJECT file
# (<project>/.aldwin/permissions.yaml) declares:
#
#   roots:
#     - ../proton-libs
#     - /Users/you/Documents/other-checkout
#
# A relative root is read from the project directory. A global roots list
# is not read: it would widen the workspace in every project at once.
#
# Inside the workspace, the tools read and stage edits, and a command the
# agent runs may write. Outside it, every tool refuses the path, and a
# command — like the language server and the MCP servers Aldwin starts —
# can still read but cannot write, except to temporary files and caches.
# Where this system cannot enforce that, Aldwin says so once when it starts.
#
# Keys from earlier versions of Aldwin — `allow:`, `default:` and `deny:` —
# are still accepted so an older file loads, but none of them does anything
# now. Aldwin says so once at startup if it finds one; delete them at your
# leisure.
#
# If you edit this while Aldwin is running, /reload-config picks the change up.
version: 2
";

pub const PROVIDER_HEADER: &str = "\
# Aldwin provider settings.
#   provider:                  anthropic | openai-compatible
#   model:                     the model id to use for every request
#   base_url:                  only used when provider is openai-compatible, and
#                              it is the full chat-completions URL, not a prefix
#                              — e.g. https://host/v1/chat/completions
#   api_key_env:               the NAME of an environment variable holding your
#                              API key — Aldwin never reads or stores the key
#                              itself here, only this variable's name. Export it
#                              before starting Aldwin. A provider you have
#                              connected an account to (/connect) is reached
#                              through the account first, and the key only when
#                              no account is connected.
#   extended_thinking_budget:  token budget for extended thinking. Omit to use
#                              aldwin-llm's built-in default.
";

pub const CONNECTIONS_HEADER: &str = "\
# Aldwin connected accounts, written by /connect (global only — there is no
# project-scope connections.yaml). Each entry under accounts is one
# provider's account; a model on that provider runs on it rather than on an
# API key while the entry is here.
#
# The tokens here are secrets: they act as you at that provider until they
# expire or you revoke them there. Keep this file to yourself. Deleting an
# account's entry disconnects it from the next start, or after
# /reload-config and picking the model again. Aldwin rewrites an entry
# whenever the provider rotates its token, and removes it when the provider
# revokes it.
";

pub const MCP: &str = "\
# Aldwin MCP server registry.
# Each entry under servers needs a unique name and one of:
#   kind: stdio, command: <path>, args: [...]
#   kind: http,  url: <endpoint>
# A project-scope entry with the same name replaces a global one entirely —
# fields are never merged across scopes. A stdio server runs in the same
# sandbox as the agent's commands: it can write only inside the workspace.
version: 1
servers: []
";

// A macro, not a `const`: `concat!` cannot take a `const &str`, and `TUI`
// is built from it.
macro_rules! tui_header {
    () => {
        "\
# Aldwin TUI preferences (global only — there is no project-scope tui.yaml).
#   theme:  dark (default) | light. Optional. /theme changes it and saves it
#           here. Anything other than \"light\" (including an unset or
#           omitted field) means dark.
"
    };
}

pub const TUI_HEADER: &str = tui_header!();

pub const TUI: &str = concat!(tui_header!(), "version: 1\n");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;

    #[test]
    fn every_annotated_file_parses_under_its_own_schema() {
        let permissions: PermissionsConfig = serde_yaml_ng::from_str(PERMISSIONS).unwrap();
        assert_eq!(permissions, PermissionsConfig::empty());
        assert!(!permissions.has_stale_keys());

        let mcp: McpConfig = serde_yaml_ng::from_str(MCP).unwrap();
        assert_eq!(mcp, McpConfig::empty());

        let tui: TuiConfig = serde_yaml_ng::from_str(TUI).unwrap();
        assert_eq!(tui, TuiConfig::empty());
    }

    /// `connections.yaml` is never seeded; its header only ever precedes a
    /// serialised value.
    #[test]
    fn the_connections_header_over_a_serialised_value_parses() {
        let text = format!(
            "{CONNECTIONS_HEADER}{}",
            serde_yaml_ng::to_string(&ConnectionsConfig::empty()).unwrap()
        );
        assert_eq!(
            serde_yaml_ng::from_str::<ConnectionsConfig>(&text).unwrap(),
            ConnectionsConfig::empty()
        );
    }

    #[test]
    fn a_headers_own_comment_block_matches_the_full_constants_leading_text() {
        assert!(TUI.starts_with(TUI_HEADER));
    }

    #[test]
    fn header_plus_a_freshly_serialized_empty_value_still_parses() {
        let tui_text = format!(
            "{TUI_HEADER}{}",
            serde_yaml_ng::to_string(&TuiConfig::empty()).unwrap()
        );
        assert_eq!(
            serde_yaml_ng::from_str::<TuiConfig>(&tui_text).unwrap(),
            TuiConfig::empty()
        );
    }
}
