//! First-launch YAML written to `~/.mjolnir/` on a fresh install — a tour of
//! the format in the developer's editor. Each string must parse under its
//! domain's real schema (checked in this module's tests); the exact wording
//! is a separate deliverable from the structure.
//!
//! Each domain also exposes a `*_HEADER` constant: just the leading
//! `#`-comment block, textually identical to the same lines in the full
//! constant below it. `store.rs`'s `with_domain_mut` (via
//! `fsio::write_atomic_with_header`) prepends the matching header to
//! *every* write of that domain's file, not just the first-launch one —
//! `serde_yaml_ng` has no concept of a source file's original comments, so a
//! plain re-serialize permanently drops them the moment anything is next
//! persisted (e.g. one permission grant), leaving a developer who opens
//! their real, in-use `permissions.yaml` looking at a bare `version`/
//! `allow`/`deny` with no explanation of the format at all — reported
//! directly as "editing permissions.yaml doesn't really appear to make any
//! sense." The two constants are kept separate (rather than splicing the
//! header out of the full one at compile time) only because Rust's `concat!`
//! can't reference another `const &str`; keep the wording in sync by hand —
//! `tests::a_headers_own_comment_block_matches_the_full_constants_leading_
//! text` below catches drift between the two, though it can't stop it from
//! happening in the first place the way the type system would.

pub const PERMISSIONS_HEADER: &str = "\
# Mjolnir permissions — one scope layer of a default-deny grant list.
#
# Mjolnir reads up to two of these: ~/.mjolnir/permissions.yaml (applies to
# every project) and <project>/.mjolnir/permissions.yaml (this project
# only), and merges them — whichever file you're looking at right now is one
# of those two, never both. A third layer, session-only grants (picked with
# \"just this session\" at a prompt), never touches disk at all and is gone
# the moment Mjolnir exits. Precedence: session > project > global; within a
# single file, deny always wins over allow regardless of which list an entry
# is in.
#
# Entries are \"kind:pattern\" strings, e.g. \"read:./src/**\" or
# \"shell:cargo test*\". kind is a tool's own name — read, shell, and explain
# are built in; every MCP tool you approve adds its own name here too. edit
# is never listed here on purpose: Edit always prompts per call and can't be
# persisted at any tier (see mjolnir's \"Edit is never allowlistable\"
# constraint), so a hand-added \"edit:...\" entry would parse fine but has no
# effect. pattern's only wildcard is \"*\", matching any run of characters
# including \"/\" — \"**\" behaves exactly like a single \"*\" here, not the
# recursive-directory match some other tools give it.
#
# You'll rarely need to hand-edit this file: choosing \"for this project\" or
# \"always\" at a permission prompt writes the grant here for you, and the
# comments above stay right where they are (see fsio::write_atomic_with_header).
# If you do edit it directly while Mjolnir is running, run /reload-config to
# pick the change up without restarting the session.
";

pub const PERMISSIONS: &str = "\
# Mjolnir permissions — one scope layer of a default-deny grant list.
#
# Mjolnir reads up to two of these: ~/.mjolnir/permissions.yaml (applies to
# every project) and <project>/.mjolnir/permissions.yaml (this project
# only), and merges them — whichever file you're looking at right now is one
# of those two, never both. A third layer, session-only grants (picked with
# \"just this session\" at a prompt), never touches disk at all and is gone
# the moment Mjolnir exits. Precedence: session > project > global; within a
# single file, deny always wins over allow regardless of which list an entry
# is in.
#
# Entries are \"kind:pattern\" strings, e.g. \"read:./src/**\" or
# \"shell:cargo test*\". kind is a tool's own name — read, shell, and explain
# are built in; every MCP tool you approve adds its own name here too. edit
# is never listed here on purpose: Edit always prompts per call and can't be
# persisted at any tier (see mjolnir's \"Edit is never allowlistable\"
# constraint), so a hand-added \"edit:...\" entry would parse fine but has no
# effect. pattern's only wildcard is \"*\", matching any run of characters
# including \"/\" — \"**\" behaves exactly like a single \"*\" here, not the
# recursive-directory match some other tools give it.
#
# You'll rarely need to hand-edit this file: choosing \"for this project\" or
# \"always\" at a permission prompt writes the grant here for you, and the
# comments above stay right where they are (see fsio::write_atomic_with_header).
# If you do edit it directly while Mjolnir is running, run /reload-config to
# pick the change up without restarting the session.
version: 1
allow: []
deny: []
";

pub const PROVIDER_HEADER: &str = "\
# Mjolnir provider settings.
#   provider:                  anthropic | openai-compatible
#   model:                     the model id to use for every request
#   base_url:                  only used when provider is openai-compatible
#   api_key_env:               the NAME of an environment variable holding your
#                              API key — Mjolnir never reads or stores the key
#                              itself here, only this variable's name. Export it
#                              before starting Mjolnir.
#   extended_thinking_budget:  token budget for extended thinking. Omit to use
#                              mjolnir-llm's built-in default.
";

pub const PROVIDER: &str = "\
# Mjolnir provider settings.
#   provider:                  anthropic | openai-compatible
#   model:                     the model id to use for every request
#   base_url:                  only used when provider is openai-compatible
#   api_key_env:               the NAME of an environment variable holding your
#                              API key — Mjolnir never reads or stores the key
#                              itself here, only this variable's name. Export it
#                              before starting Mjolnir.
#   extended_thinking_budget:  token budget for extended thinking. Omit to use
#                              mjolnir-llm's built-in default.
version: 1
provider: anthropic
model: claude-sonnet-5
api_key_env: ANTHROPIC_API_KEY
";

pub const MCP_HEADER: &str = "\
# Mjolnir MCP server registry.
# Each entry under servers needs a unique name and one of:
#   kind: stdio, command: <path>, args: [...]
#   kind: http,  url: <endpoint>
# A project-scope entry with the same name replaces a global one entirely —
# fields are never merged across scopes.
";

pub const MCP: &str = "\
# Mjolnir MCP server registry.
# Each entry under servers needs a unique name and one of:
#   kind: stdio, command: <path>, args: [...]
#   kind: http,  url: <endpoint>
# A project-scope entry with the same name replaces a global one entirely —
# fields are never merged across scopes.
version: 1
servers: []
";

pub const TUI_HEADER: &str = "\
# Mjolnir TUI preferences (global only — there is no project-scope tui.yaml).
# theme, layout, and keybinds are all optional; omit whatever you don't want
# to override.
";

pub const TUI: &str = "\
# Mjolnir TUI preferences (global only — there is no project-scope tui.yaml).
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

    /// `store.rs`'s `with_domain_mut` re-serializes a domain's value fresh
    /// on every write and prepends the matching `*_HEADER` in front of that
    /// (see `fsio::write_atomic_with_header`) — this is what that actually
    /// produces for an empty/default value, so it must still parse. Guards
    /// the real failure mode this module's own doc comment warns about: the
    /// `HEADER` and full constants are two separate literals kept in sync by
    /// hand, so a header-only edit that isn't mirrored into the full
    /// constant (or vice versa) wouldn't be caught by
    /// `every_annotated_file_parses_under_its_own_schema` above, since that
    /// test only ever exercises the full constants.
    #[test]
    fn a_headers_own_comment_block_matches_the_full_constants_leading_text() {
        for (header, full) in [(PERMISSIONS_HEADER, PERMISSIONS), (PROVIDER_HEADER, PROVIDER), (MCP_HEADER, MCP), (TUI_HEADER, TUI)] {
            assert!(full.starts_with(header), "header text has drifted from the full constant's own leading comment block:\nheader: {header:?}\nfull:   {full:?}");
        }
    }

    #[test]
    fn header_plus_a_freshly_serialized_empty_value_still_parses() {
        let permissions_text = format!("{PERMISSIONS_HEADER}{}", serde_yaml_ng::to_string(&PermissionsConfig::empty()).unwrap());
        assert_eq!(serde_yaml_ng::from_str::<PermissionsConfig>(&permissions_text).unwrap(), PermissionsConfig::empty());

        let mcp_text = format!("{MCP_HEADER}{}", serde_yaml_ng::to_string(&McpConfig::empty()).unwrap());
        assert_eq!(serde_yaml_ng::from_str::<McpConfig>(&mcp_text).unwrap(), McpConfig::empty());

        let tui_text = format!("{TUI_HEADER}{}", serde_yaml_ng::to_string(&TuiConfig::empty()).unwrap());
        assert_eq!(serde_yaml_ng::from_str::<TuiConfig>(&tui_text).unwrap(), TuiConfig::empty());
    }
}
