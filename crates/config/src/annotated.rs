//! First-launch YAML written to `~/.aldwin/` on a fresh install — a tour of
//! the format in the developer's editor. Each string must parse under its
//! domain's real schema (checked in this module's tests); the exact wording
//! is a separate deliverable from the structure.
//!
//! Each domain also exposes a `*_HEADER` constant: just the leading
//! `#`-comment block. `store.rs`'s `with_domain_mut` prepends it to *every*
//! write of that domain's file, because a plain re-serialise drops the
//! comments the moment anything is next persisted — reported as "editing
//! permissions.yaml doesn't really appear to make any sense."
//!
//! The header text lives in a macro rather than a `const` only because
//! `concat!` cannot take a `const &str`; the macro is what lets the full
//! constant be built from the header instead of restating it.

macro_rules! permissions_header {
    () => {
"\
# Aldwin permissions — one scope layer. Reads and runs need no permission;
# every edit is reviewed before it is written; this file is where you lock a
# program out, and where you widen what the tools may reach.
#
# Aldwin reads up to two of these: ~/.aldwin/permissions.yaml (applies in
# every project) and <project>/.aldwin/permissions.yaml (this project only).
# Whichever file you are looking at is one of those two, never both.
#
#   deny:     programs that may not run. A deny is a lock: nothing narrower
#             can override it — not the other file, not the agent, not a
#             single turn. A locked call is refused outright and the refusal
#             names this file. Undoing one is an edit here, made
#             deliberately, outside the moment that wanted it.
#   roots:    extra directories the tools may be pointed at, beyond this
#             project. A relative root is read from the project directory.
#
#               roots:
#                 - ../proton-libs
#                 - /Users/you/Documents/other-checkout
#
#             Project files only; a global roots list would widen reach in
#             every project at once. Without one, every tool — run included —
#             is confined to this project, and a path outside it is refused
#             rather than quietly reached.
#
# A deny entry is a program, optionally qualified by the class of call:
#
#   deny:
#     - curl           # locked entirely
#     - npm: write     # npm may still read
#
# The class belongs to the CALL, not the program: `git status` is a read and
# `git push` is a write, and they are the same binary. The agent declares a
# class for each call, and a call it declares a read is executed with your
# source tree read-only and the network unreachable — so a declaration that
# was wrong fails, and the agent is told to declare it again as what it is.
# There is no entry for the whole command line: argv is run directly, never
# through a shell, so `&&`, `|` and `$(...)` are ordinary characters and
# cannot chain a second command onto a first.
#
# Editing is not governed here at all. Every edit the agent stages in a turn
# is shown to you as one review, and nothing is written until you approve it
# — under every setting in this file, with no way to turn it off.
#
# Two keys from an earlier version of Aldwin — `allow:` and `default:` — are
# still accepted so an older file loads, but nothing reads them. Aldwin says
# so once at startup if it finds them; delete them at your leisure.
#
# If you edit this while Aldwin is running, /reload-config picks the change up.
"
    };
}

pub const PERMISSIONS_HEADER: &str = permissions_header!();

pub const PERMISSIONS: &str = concat!(permissions_header!(), "version: 2\ndeny: []\n");

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
#                              before starting Aldwin.
#   extended_thinking_budget:  token budget for extended thinking. Omit to use
#                              aldwin-llm's built-in default.
";

macro_rules! mcp_header {
    () => {
"\
# Aldwin MCP server registry.
# Each entry under servers needs a unique name and one of:
#   kind: stdio, command: <path>, args: [...]
#   kind: http,  url: <endpoint>
# A project-scope entry with the same name replaces a global one entirely —
# fields are never merged across scopes.
"
    };
}

pub const MCP_HEADER: &str = mcp_header!();

pub const MCP: &str = concat!(mcp_header!(), "version: 1\nservers: []\n");

macro_rules! tui_header {
    () => {
"\
# Aldwin TUI preferences (global only — there is no project-scope tui.yaml).
# theme, layout, and keybinds are all optional; omit whatever you don't want
# to override.
#   theme:  dark (default) | light. Read once at session start; changing it
#           takes effect on the next launch, not live. Anything other than
#           \"light\" (including an unset/omitted field) means dark.
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
        // The first-launch file states `ask` outright rather than leaving the
        // field absent: it is a teaching file, and the one rung that grants
        // nothing is the one worth showing a developer written down.
        assert_eq!(permissions, PermissionsConfig::empty(), "a fresh file states neither a rung nor an allow list (ADR 0009)");

        let mcp: McpConfig = serde_yaml_ng::from_str(MCP).unwrap();
        assert_eq!(mcp, McpConfig::empty());

        let tui: TuiConfig = serde_yaml_ng::from_str(TUI).unwrap();
        assert_eq!(tui, TuiConfig::empty());
    }

    /// The full constants are built from the headers, so this cannot drift;
    /// it pins that whoever changes how they are built keeps it that way.
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
