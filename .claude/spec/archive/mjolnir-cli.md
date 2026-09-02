# mjolnir-cli

Binary crate — startup sequence, session bootstrap, impl wiring, slash-command dispatch.

**Status:** archived — implemented, tested, audited
**Scope:** crates/cli
**Owner:** Maximilian
**Last Updated:** 2026-06-10

**Completed:** 2026-08-29 — `20a8d39`, plus an audit fix (`f023d4a`) that
wired up context-file approval (was implemented in mjolnir-permissions
and mjolnir-tui but never actually invoked from here — the session
initializer now does exactly what this spec's Design section says: tests
each candidate file before composing additional-context). No known gaps
against this spec. Manually verified against the real binary: --help/
--version, first-launch init, refuse-to-start on a missing env var, and
the full startup path through TUI launch — but not yet a full live session
against the real Anthropic API (no API key in this environment).

**Post-archive addition (2026-08-29):** A live run's developer had no
discoverable way to end a session short of `Ctrl+C` (itself undiscoverable
until fixed in mjolnir-tui the same day) and asked for a slash command.
Added `/exit` to the dispatch table — `Intercepted::Quit`
makes `run_interceptor` return instead of looping again, which drops its
`forward` and `events` sender clones; the core's command channel then
closes, the core drains and drops its own `events` sender, and the TUI's
event channel closes the same way it already does on `None` — no new
`Event` variant, no core changes. This was explicitly out of scope before
("Additional slash commands beyond /reload-config — V0 set is minimal"),
not a gap against the original spec.

Also added `/help`, listing all three commands (`/help`, `/exit`,
`/reload-config`) from a single `HELP_TEXT` constant kept in sync by hand
with the `match` in `intercept` — three commands doesn't earn a
data-driven dispatch table yet. The unknown-command Notice now points at
`/help` too.

**Post-archive addition (2026-08-29, `/clear`):** Part of mjolnir-tui's
same-day live-feedback batch (see its own spec). Unlike every other known
command, `/clear` is translated and forwarded (`Intercepted::Forward(Command::ClearHistory)`)
rather than handled locally — core owns `ConversationLog`, so only core
can actually wipe it; `intercept` returns `Forward` instead of sending a
`Notice` and returning `Handled` the way `/help`/`/reload-config` do. This
doesn't reopen the Decisions section's "core has no slash-command
semantics" — `ClearHistory` is a generic core operation core would accept
from any caller, the same way `Submit`/`Cancel` are; the CLI layer still
owns 100% of the `/`-prefix parsing and dispatch table, it's just that this
one entry's action lives in core rather than in this crate. `HELP_TEXT`
updated to include it.

**Post-archive addition (2026-09-02, `/theme`):** Follow-up to mjolnir-tui's
same-day light-theme addition (see its own spec) — the developer asked for
a way to switch themes from inside the harness rather than hand-editing
`tui.yaml`. `/theme light|dark` is closer to `/reload-config` than to
`/clear`: it never touches core at all. `handle_theme` reads/writes
`tui.yaml` directly (`Config::global_tui`/`set_tui`, already existed) and
sends `Event::ThemeChanged { theme }` straight into the same channel the
TUI reads from — this doesn't reopen "core has no slash-command semantics"
below any more than `PermissionsChanged` did; it's a config write plus a
UI-facing signal, not a core operation. `/theme` with no argument reports
the current value rather than erroring (mirrors `/help`'s "tell the
developer where they stand" instinct, since this process has no other way
to see what a *running* TUI is currently showing than what's already on
disk — the two should always agree, since this command is the only thing
that changes either). An invalid value (anything but `light`/`dark`,
case-insensitive) is rejected with a `Notice` and neither persists nor
emits `ThemeChanged` — confirmed by test, not just by the validation read.
`HELP_TEXT` updated to include it. See mjolnir-tui.md's matching Progress
note for the `App`/`ui.rs` side (switches live, no restart, since `App::
theme` is read fresh on every draw) and mjolnir-core.md's for the new
`Event` variant.

## Design

- **Invocation:** Zero-arg binary. `mjolnir` starts a session rooted at the current working directory. No runtime flags, subcommands, or environment overrides in V0 — everything driven by config files.
- **Startup Sequence:**
  1. init_global_if_empty — first launch writes annotated global config; PartiallyPresent → refuse to start.
  2. Load all config layers — refuse to start on any parse failure, schema error, unknown major, or missing env var (error to stderr, non-zero exit; TUI has not yet launched).
  3. Build additional-context string from cwd path and approved context file contents.
  4. Instantiate concrete impls: AnthropicClient (mjolnir-llm), PermissionsEngine (mjolnir-permissions), ToolDispatcher (mjolnir-tools).
  5. Create the agent loop (mjolnir-core) with LlmClient, ToolDispatcher, and additional-context.
  6. Launch TUI (mjolnir-tui) with the core's event receiver and command sender.
  7. Block on TUI exit; drop channels; wait for core to drain cleanly.
- **Additional Context:** Opaque string handed to mjolnir-core. Contains: absolute cwd path, then the full text of each approved context file (CLAUDE.md / AGENTS.md) from the project_context_files() snapshot, in path order. Files not in the approved list are excluded regardless of existence on disk. The core composes `<base_system_prompt>\n\n<additional_context>` and sends it verbatim.
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

- .claude/spec/mjolnir.md — parent spec; binary crate role and dependency list.
- .claude/spec/mjolnir-core.md — agent loop, additional-context contract, event/command channels.
- .claude/spec/mjolnir-config.md — startup sequence, init_global_if_empty, refuse-to-start rules.
- .claude/spec/mjolnir-permissions.md — PermissionsEngine init from config snapshot.
- .claude/spec/mjolnir-tools.md — ToolDispatcher instantiation.
- .claude/spec/mjolnir-tui.md — TUI launch, channel wiring, /reload-config surface.
