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
# Mjolnir permissions — one scope layer. Nothing runs that a rule here, or a
# prompt you answered, has not allowed.
#
# Mjolnir reads up to two of these: ~/.mjolnir/permissions.yaml (applies in
# every project) and <project>/.mjolnir/permissions.yaml (this project only).
# Whichever file you are looking at is one of those two, never both. Two more
# layers never touch disk: session grants, gone when Mjolnir exits, and a
# single turn\'s \"allow once\", gone immediately.
#
#   default:  what runs when no entry below covers the call.
#               ask    every call asks, every time
#               read   reads run; writes and edits ask
#               write  reads and writes run; edits ask
#
#             Where this file and the other one disagree, the narrower file
#             wins outright — a project may be opened up without loosening
#             every project, or locked down without touching the global file.
#
#   allow:    programs that may run, and the class they may run at.
#   deny:     programs that may not. A deny is a lock: nothing narrower can
#             override it — not the other file, not a session, not a single
#             turn. Undoing one is an edit to this file, made deliberately,
#             outside the moment that wanted it.
#
# An entry is a program and a class:
#
#   allow:
#     - git: read      # any git call that reads
#     - cargo: write   # any cargo call at all — write includes read
#     - rg             # every class, the widest grant there is
#   deny:
#     - curl           # locked entirely
#     - npm: write     # npm may still read
#
# The class belongs to the CALL, not the program: `git status` is a read and
# `git push` is a write, and they are the same binary. The agent declares a
# class for each call, and a call it declares a read is executed with your
# source tree read-only and the network unreachable — so a declaration that
# was wrong costs you a prompt, not a tree. There is no entry for the whole
# command line: argv is run directly, never through a shell, so `&&`, `|`
# and `$(...)` are ordinary characters and cannot chain a second command
# onto an approved first one.
#
# `edit` is not a class you can write here. Editing a file always shows you
# the diff and waits, under every setting in this file, with no way to turn
# it off. An `edit` entry is a load error rather than a rule that quietly
# does nothing.
#
# You will rarely hand-edit this: answering a permission prompt writes the
# rule for you, and these comments stay where they are. If you do edit it
# while Mjolnir is running, /reload-config picks the change up.
";

pub const PERMISSIONS: &str = "\
# Mjolnir permissions — one scope layer. Nothing runs that a rule here, or a
# prompt you answered, has not allowed.
#
# Mjolnir reads up to two of these: ~/.mjolnir/permissions.yaml (applies in
# every project) and <project>/.mjolnir/permissions.yaml (this project only).
# Whichever file you are looking at is one of those two, never both. Two more
# layers never touch disk: session grants, gone when Mjolnir exits, and a
# single turn\'s \"allow once\", gone immediately.
#
#   default:  what runs when no entry below covers the call.
#               ask    every call asks, every time
#               read   reads run; writes and edits ask
#               write  reads and writes run; edits ask
#
#             Where this file and the other one disagree, the narrower file
#             wins outright — a project may be opened up without loosening
#             every project, or locked down without touching the global file.
#
#   allow:    programs that may run, and the class they may run at.
#   deny:     programs that may not. A deny is a lock: nothing narrower can
#             override it — not the other file, not a session, not a single
#             turn. Undoing one is an edit to this file, made deliberately,
#             outside the moment that wanted it.
#
# An entry is a program and a class:
#
#   allow:
#     - git: read      # any git call that reads
#     - cargo: write   # any cargo call at all — write includes read
#     - rg             # every class, the widest grant there is
#   deny:
#     - curl           # locked entirely
#     - npm: write     # npm may still read
#
# The class belongs to the CALL, not the program: `git status` is a read and
# `git push` is a write, and they are the same binary. The agent declares a
# class for each call, and a call it declares a read is executed with your
# source tree read-only and the network unreachable — so a declaration that
# was wrong costs you a prompt, not a tree. There is no entry for the whole
# command line: argv is run directly, never through a shell, so `&&`, `|`
# and `$(...)` are ordinary characters and cannot chain a second command
# onto an approved first one.
#
# `edit` is not a class you can write here. Editing a file always shows you
# the diff and waits, under every setting in this file, with no way to turn
# it off. An `edit` entry is a load error rather than a rule that quietly
# does nothing.
#
# You will rarely hand-edit this: answering a permission prompt writes the
# rule for you, and these comments stay where they are. If you do edit it
# while Mjolnir is running, /reload-config picks the change up.
version: 2
default: ask
allow: []
deny: []
";

pub const PROVIDER_HEADER: &str = "\
# Mjolnir provider settings.
#   provider:                  anthropic | openai-compatible
#   model:                     the model id to use for every request
#   base_url:                  only used when provider is openai-compatible, and
#                              it is the full chat-completions URL, not a prefix
#                              — e.g. https://host/v1/chat/completions
#   api_key_env:               the NAME of an environment variable holding your
#                              API key — Mjolnir never reads or stores the key
#                              itself here, only this variable's name. Export it
#                              before starting Mjolnir.
#   extended_thinking_budget:  token budget for extended thinking. Omit to use
#                              mjolnir-llm's built-in default.
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
#   theme:  dark (default) | light. Read once at session start; changing it
#           takes effect on the next launch, not live. Anything other than
#           \"light\" (including an unset/omitted field) means dark.
";

pub const TUI: &str = "\
# Mjolnir TUI preferences (global only — there is no project-scope tui.yaml).
# theme, layout, and keybinds are all optional; omit whatever you don't want
# to override.
#   theme:  dark (default) | light. Read once at session start; changing it
#           takes effect on the next launch, not live. Anything other than
#           \"light\" (including an unset/omitted field) means dark.
version: 1
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::*;

    #[test]
    fn every_annotated_file_parses_under_its_own_schema() {
        let permissions: PermissionsConfig = serde_yaml_ng::from_str(PERMISSIONS).unwrap();
        // The first-launch file states `ask` outright rather than leaving the
        // field absent: it is a teaching file, and the one rung that grants
        // nothing is the one worth showing a developer written down.
        assert_eq!(permissions, PermissionsConfig { default: Some(crate::domain::Rung::Ask), ..PermissionsConfig::empty() });

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
        for (header, full) in [(PERMISSIONS_HEADER, PERMISSIONS), (MCP_HEADER, MCP), (TUI_HEADER, TUI)] {
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
