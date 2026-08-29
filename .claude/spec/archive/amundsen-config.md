# amundsen-config

Per-domain YAML at project and global scope; typed accessors; refuse-to-start on bad config.

**Status:** archived — implemented, tested, audited
**Scope:** amundsen-config crate only. On-disk file layout, per-domain typed accessors, first-launch init, in-session reload. Excludes the runtime permission engine, the agent loop, and provider HTTP work.
**Owner:** Maximilian
**Last Updated:** 2026-05-16

**Completed:** 2026-08-29. Implemented in full — `b9d81f8`, plus a small
additive `extended_thinking_budget` field on `ProviderConfig` (`5c868ce`,
needed by amundsen-llm). No known gaps against this spec.

## Why

Amundsen's persistent state — permission grants, context-file decisions, provider settings, MCP server entries, TUI preferences — lives on disk in per-domain YAML across two scopes. This crate owns the format, the read/write surface, and first-launch init. It does not know what permissions mean, how the agent loop uses provider config, or how MCP servers are spawned. Keeping the persistence layer as a leaf crate (`depends_on: []`) means policy crates can test without real YAML and the format can evolve without touching typed consumers.

## Vocabulary

- **Domain:** A coherent group of config that has its own file. Five domains in V0: permissions, context_files, provider, mcp, tui.
- **Scope:** Persistence tier on disk — project (`<project>/.amundsen/`) or global (`~/.amundsen/`). Not every domain exists at every scope: context_files is project-only, tui is global-only. First launch therefore writes four global files (permissions, provider, mcp, tui) — not five.
- **Layer:** A (scope, domain) pair — one YAML file. Layers are exposed as raw per-layer snapshots; merging is the consumer's job.
- **Annotated Config:** The first-launch YAML written to `~/.amundsen/` on a fresh install. Empty allow/deny, commented placeholders, inline comments explaining each field — a tour of the format in the developer's editor.

## Design

- **File Layout:** Project: `<project>/.amundsen/<domain>.yaml`. Global: `~/.amundsen/<domain>.yaml`. One file per domain per scope. XDG paths intentionally avoided — matches the `.claude/`, `.gemini/` convention.
- **Version Field:** Every YAML ships with `version: 1` as a top-level field from V0. Loader reads the version first; unknown majors fail loudly. Each domain versions independently.
- **Permissions Format:** Two top-level lists, `allow:` and `deny:`, each containing grant entries keyed by `kind:pattern`. Loader returns both lists separately — deny-wins is a structural property of the file shape, not a runtime sort.
- **Provider Format:** Fields for the active provider — `provider:` (anthropic | openai-compatible), `model:`, `base_url:` (OpenAI-compat only), `api_key_env:` naming the env var that holds the key. A raw `api_key:` field is rejected by the schema. Missing or empty `api_key_env` field → refuse to start. Resolving the env var is the LLM crate's responsibility, not config's.
- **MCP Format:** A `servers:` list, each with name, transport (stdio command + args, or HTTP endpoint), optional env vars. Project scope shadows global by server name — replaces wholesale rather than merging fields.
- **TUI Format:** Global-only. Theme, layout, keybind overrides. Field set owned by amundsen-tui.
- **Context Files Format:** Project-only. List of approved absolute paths for CLAUDE.md / AGENTS.md. Path-keyed only, no content hash (see amundsen-permissions decision).
- **First Launch:** On a fresh install with no `~/.amundsen/`, the crate creates it and writes all four global domain files annotated. Project scope is left untouched until something persists to it. First-launch is idempotent — runs only when the directory does not exist. Directory present but missing a file → refuse to start (see pitfall).
- **Atomic Writes:** Every write via tempfile + fsync + rename within the same directory. A crash mid-write leaves either the old or the new file, never partial YAML.
- **Reload:** Read once at startup. Hand-edits are silent until `/reload-config`, which re-reads every layer and fires change events through consumer channels. No file watcher in V0. On reload parse failure the previous in-memory snapshot is retained.

## Interfaces

- **Read:** Domain-typed accessors per scope: `project_permissions()`, `global_permissions()`, `project_provider()`, `global_provider()`, `project_mcp()`, `global_mcp()`, `global_tui()`, `project_context_files()`. Each returns a typed snapshot of one (scope, domain) layer. Permissions returns the raw allow and deny lists separately. No merged view — that is the consumer's job.
- **Write:** Domain-typed mutators per scope: `add_grant`, `remove_grant`, `set_provider`, `add_mcp_server`, etc. Each persists atomically and invalidates the in-memory snapshot. Session-scope writes are not accepted — the session layer lives in amundsen-permissions and never touches disk. All writes round-trip through the same schema validation as the loader.
- **Reload All:** Re-reads every existing layer. Called by the cli crate's `/reload-config` handler. On failure returns an error and retains the previous snapshot.
- **Init Global If Empty:** Creates `~/.amundsen/` and writes the four annotated files if the directory does not exist. Returns Created | AlreadyPresent | PartiallyPresent. PartiallyPresent is refuse-to-start.

## Decisions

- **amundsen-config is the general project-wide config crate, not a permissions-persistence crate.** — Provider, MCP, TUI, permissions, and context-files all flow through one persistence boundary. Narrower scoping would force a second config crate and proliferate format ownership.

- **One file per domain per scope; domain-typed accessors on the API.** — Small diffs when only one domain changes; parse failures isolate to one domain; typed API stays stable per-domain as the YAML evolves. A single combined file per scope was rejected — its only win is atomic-per-scope write, not worth the cost.

- **amundsen-config exposes raw per-scope layers; consumers do any cross-scope merging.** — Different domains want different merge semantics — permissions wants per-grant attribution, provider wants flat overlay, MCP wants name-keyed union. A generic merge would degenerate to the lowest common denominator or grow per-domain branches.

- **Global lives at `~/.amundsen/`, not `~/.config/amundsen/`.** — Matches the `.claude/`, `.gemini/` convention. Tool-rooted style is uniform across platforms; XDG nesting is Linux-specific and Amundsen is cross-platform.

- **permissions.yaml uses two top-level lists (allow / deny); deny applied over allow at load.** — Deny-wins is a structural property of the file shape. A single ordered list with an `effect:` field would let a later allow silently overwrite an earlier deny — the exact failure mode the permissions spec called out.

- **provider.yaml stores api_key_env only; raw keys never persisted.** — Dotfiles get committed by accident; raw keys would be the worst kind of leak. Env-var-reference is what Claude Code and OpenCode do. OS keychain integration reachable later behind the same `api_key_env:` indirection.

- **Read once at startup; in-session reload via `/reload-config`; no file watcher in V0.** — A file watcher would force the permissions engine to invalidate snapshots on every disk event for a dependency Amundsen does not otherwise need. V0 trades that complexity for one slash command.

- **First launch writes all four global domain files annotated; init is idempotent.** — The annotated YAML is the developer's tour of the format. Writing all four upfront beats drip-feeding the format domain by domain. Partial directory is refuse-to-start, not auto-filled.

- **Every YAML file ships with version 1 from V0; loader refuses unknown majors.** — Three lines per file and one constant in code saves a "missing field = pre-v1" hack the day a migration is needed. Each domain versions independently.

- **Malformed file / schema failure / unknown major / missing api_key_env field → refuse to start.** — A broken config deserves the developer's attention, not a soft-fail that lets them keep working with silent gaps. Mid-session reload failure is different — previous snapshot is retained so a hand-edit gone wrong does not collapse the session.

## Pitfalls

- Deny-wins drifting from structural property to runtime sort — never collapse allow + deny internally to "simplify" consumers.
- Stamping version 1 and never bumping it — bump on any non-backwards-compatible field change, even without a migration wired up.
- First-launch idempotency missing the partial-init case (dir exists but file missing) — refuse to start, do not auto-fill.
- Multiple Amundsen processes writing the same project file concurrently — atomic rename gives last-write-wins; V0 assumes single-process.
- Reload failing silently — `/reload-config` must surface the failing file name to the TUI; quiet success would train trust in stale config.
- api_key_env typos — error must quote the env var name verbatim from the YAML, not the canonical one.
- Schema validation living only in the loader — all writes must round-trip through the same validation.
- Project scope materialising before the developer has authorised anything — `<project>/.amundsen/` should not appear until the first persisted grant.

## Out of Scope

- File watching / live reload (notify, fs-events) — deferred past V0.
- Schema migrations between major versions — the version field is in place; migration framework lands when v2 does.
- Audit log of grant changes or config edits — out of V0; TUI shows current state only.
- GC of unreachable context-file paths — flagged in permissions spec, deferred.
- Cross-process write coordination (file locking, advisory locks) — single-process assumption in V0.
- OS keychain integration — `api_key_env` indirection is the V0 surface; keychains reachable later behind it.
- Textual content of the annotated YAML files — ownership is in scope; the prose is its own deliverable.
- Session-scope persistence — the session layer is in-memory and owned by amundsen-permissions.
- TUI rendering of config errors, the `/reload-config` command itself, in-process command dispatch — amundsen-tui and cli.

## References

- .claude/spec/amundsen.md — parent; workspace layout and the V0 local-storage decision.
- .claude/spec/amundsen-permissions.md — sibling; the consumer that drives most of this crate's design.
- .claude/spec/amundsen-core.md — sibling; transitively depends on this crate for provider settings.
- https://docs.rs/serde_yaml/ — serde_yaml.
- https://docs.rs/serde-yaml-ng/ — actively-maintained fork; evaluate before committing.
- https://doc.rust-lang.org/std/fs/fn.rename.html — std::fs::rename; the atomic-write target.
