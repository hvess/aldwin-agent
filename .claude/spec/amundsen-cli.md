# amundsen-cli

Binary crate — startup sequence, session bootstrap, impl wiring, slash-command dispatch.

**Status:** active
**Scope:** crates/cli
**Owner:** Maximilian
**Last Updated:** 2026-06-10

## Design

- **Invocation:** Zero-arg binary. `amundsen` starts a session rooted at the current working directory. No runtime flags, subcommands, or environment overrides in V0 — everything driven by config files.
- **Startup Sequence:**
  1. init_global_if_empty — first launch writes annotated global config; PartiallyPresent → refuse to start.
  2. Load all config layers — refuse to start on any parse failure, schema error, unknown major, or missing env var (error to stderr, non-zero exit; TUI has not yet launched).
  3. Build additional-context string from cwd path and approved context file contents.
  4. Instantiate concrete impls: AnthropicClient (amundsen-llm), PermissionsEngine (amundsen-permissions), ToolDispatcher (amundsen-tools).
  5. Create the agent loop (amundsen-core) with LlmClient, ToolDispatcher, and additional-context.
  6. Launch TUI (amundsen-tui) with the core's event receiver and command sender.
  7. Block on TUI exit; drop channels; wait for core to drain cleanly.
- **Additional Context:** Opaque string handed to amundsen-core. Contains: absolute cwd path, then the full text of each approved context file (CLAUDE.md / AGENTS.md) from the project_context_files() snapshot, in path order. Files not in the approved list are excluded regardless of existence on disk. The core composes `<base_system_prompt>\n\n<additional_context>` and sends it verbatim.
- **Slash Commands:** Input that begins with `/` is intercepted at the CLI layer before the Submit command reaches the core. The CLI maintains a dispatch table of known slash commands. Unknown slash commands are rejected with an error message in the TUI; they do not reach the core. Known V0 commands: /reload-config.
- **Reload Config:** `/reload-config` calls config.reload_all(). On success, re-initializes the PermissionsEngine from the new snapshot and notifies the TUI. On failure, previous snapshot is retained and the failing file path is surfaced to the TUI verbatim.

## Decisions

- **Zero-arg binary in V0 — no flags, no subcommands.** — All runtime configuration belongs in config files. CLI flags duplicate the config surface and create implicit override precedence rules. V0 has one provider and one mode of operation.

- **Refuse-to-start errors go to stderr before the TUI launches.** — The TUI is not available yet. A clean stderr message with the exact failing path is more actionable than launching a partially-initialised TUI to show the same error.

- **Slash commands intercepted at the CLI layer, not forwarded to the core.** — The core's only input is Submit, Cancel, ApproveTool — it has no slash-command semantics. CLI owns the dispatch table so slash commands can trigger config, TUI, or process operations that the core has no visibility into.

- **Additional-context string includes only approved context files.** — Reading arbitrary project files without a permission grant would silently bypass the default-deny model. The project_context_files() snapshot is the authorised list.

## Steps

1. Add crates/cli — Cargo.toml depending on all six sibling crates plus clap (--help/--version only).

2. Implement startup sequence — init_global_if_empty → load config → refuse-to-start path.
   - Verify: Missing env var, malformed YAML, and PartiallyPresent each produce a distinct stderr message quoting the exact path or var name.

3. Build additional-context string — cwd path + approved context file contents from project_context_files() snapshot.
   - Why: Files present on disk but absent from the approved list must be excluded — permission check is not optional.

4. Instantiate AnthropicClient, PermissionsEngine, ToolDispatcher; wire into a new agent loop.

5. Launch TUI with event/command channels; block until TUI exits.

6. Implement slash-command interceptor — parse `/`-prefixed input before channel send; reject unknowns to TUI.

7. Implement /reload-config handler — reload_all(), re-init PermissionsEngine, notify TUI; surface failing path on error.

8. Implement clean exit — TUI exit drops command sender; core drains and shuts down; process exits 0.

## Pitfalls

- Context file read bypassing the approved list — always gate on project_context_files() snapshot, not a raw filesystem walk.
- Slash commands reaching the core as Submit commands — interceptor must run synchronously before the channel send, not as a post-send hook.
- Refuse-to-start error messages that paraphrase the failing field rather than quoting it — quote the exact path, env var name, or domain file verbatim.
- Reload re-instantiating the PermissionsEngine from stale config on failure — retain previous PermissionsEngine snapshot when reload_all() returns an error.

## Out of Scope

- CLI flags, subcommands, or env-var overrides — zero-arg in V0.
- Shell completions.
- Multiple concurrent sessions or session multiplexing.
- Additional slash commands beyond /reload-config — V0 set is minimal.
- Config editing via CLI (writing provider, MCP, or permission entries from flags).
- Daemonisation or background operation.

## References

- .claude/spec/amundsen.md — parent spec; binary crate role and dependency list.
- .claude/spec/amundsen-core.md — agent loop, additional-context contract, event/command channels.
- .claude/spec/amundsen-config.md — startup sequence, init_global_if_empty, refuse-to-start rules.
- .claude/spec/amundsen-permissions.md — PermissionsEngine init from config snapshot.
- .claude/spec/amundsen-tools.md — ToolDispatcher instantiation.
- .claude/spec/amundsen-tui.md — TUI launch, channel wiring, /reload-config surface.
