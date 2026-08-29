# amundsen-permissions

Default-deny permission engine — three persistent scopes, tiered prompts, friction by design.

**Status:** active — one known gap, see Progress below
**Scope:** amundsen-permissions crate only. Policy engine, allowlist shape, prompt round-trip. Excludes TUI rendering, YAML I/O (config), and tool implementations.
**Owner:** Maximilian
**Last Updated:** 2026-05-20

**Progress (2026-08-29):** Everything else in this spec is implemented and
tested — `6c60023`. Not yet built: the "MCP tool calls use the standard
tool prompt, extended once with an edit-shape follow-up" Decision (line
57) and the matching Design bullet on MCP edit-shape detection. Building
it needs a new prompt payload/response shape here plus a new persisted
field in amundsen-config (grant strings are deliberately opaque
`kind:pattern`, not a good fit for the path-arg/content-arg mapping) — a
real, separable follow-up, not a prerequisite for the rest of this spec.
Every MCP tool currently goes through the plain four-tier prompt with
`edit_class: false`. Keep this spec active until that's built.

## Why

Every tool call, shell invocation, CLAUDE.md ingestion, and MCP tool request runs through this engine. It owns scope precedence, pattern matching, the tiered prompt round-trip, and the in-memory shape of allowlists and denylists. Cross-cutting — any crate that gates an action calls into this one rather than reimplementing the policy.

## Vocabulary

- **Guarded Action:** Anything the engine gates. Three families: tool invocations (built-in or MCP-bridged), context-file injection (CLAUDE.md / AGENTS.md), and Edit (always per-call, never allowlistable). MCP connection is not guarded — the config entry that names the server is the consent.
- **Scope:** A persistence tier. Three: session (in-memory, until process exit), project (per project root), global (per user). Precedence is session > project > global; within a scope, deny beats allow.
- **Grant:** A persisted (allow | deny) decision keyed by `kind:pattern`, stored at one scope.
- **Pattern:** Grammar for guarded-action targets. Tool args via glob (`shell:cargo test*`), file paths via path glob (`read:./**`). Exact match is a glob with no wildcards.
- **Prompt:** Blocking round-trip on a Deny-by-absence check. Three shapes: tool four-tier (allow once / for session / persist project / persist always — symmetric for deny), context-file two-tier (persist project / just this session), Edit binary (approve / deny this one edit).

## Model

- **Default Deny:** Every guarded action starts denied. No "obviously safe" carve-out — Read, Explain, shell, and every MCP tool are gated identically. Edit is the only structural exception and has its own binary prompt (see edit_exception). In a new project the first session triggers a permission prompt for every tool the agent attempts to use, since nothing is pre-allowed. That initial burst of prompts is intentional — it is how the developer builds the allowlist by encounter rather than by upfront configuration.
- **Precedence:** Session > project > global, higher wins in both directions. A session allow overrides a global deny for the session's duration; a session deny overrides a global allow for the session's duration. Nothing the session decides propagates to disk. Within a scope, deny beats allow.
- **Grammar:** Grants are (kind, pattern, decision) per scope. Tool invocations key by `kind:pattern` against the assembled argv. Path-based actions key by `kind:path-glob`.
- **Prompt Round Trip:** On Deny-by-absence: engine emits PromptRequested with action, args, and shape; caller awaits PromptResponse and calls back to record. Session decisions persist to in-memory engine state; project / global decisions persist via amundsen-config.
- **Context Files:** CLAUDE.md / AGENTS.md ingestion gated by the two-tier prompt. Decline means do not inject. Decisions are path-keyed by absolute path with no content hash — see decision below. The session initializer (cli crate) tests each candidate file before composing the additional-context string.
- **MCP Connection:** Adding an MCP server to config implicitly authorises spawn-and-enumerate; no connect-time prompt. Each tool the server advertises is default-denied and flows through the four-tier prompt on first invocation. The first-invocation prompt also asks whether the tool is edit-shaped — if yes, the developer supplies the path-arg and content-arg names and subsequent calls route through the Edit binary approval gate. Tools whose shape cannot be pre-diffed (arbitrary patch / partial edit / mutation by query) cannot inherit the gate and stay on the standard four-tier prompt.
- **Edit Exception:** Edit is never allowlistable in any scope. The prompt shape collapses to approve/deny per invocation. Enforcement is keyed to an edit_class flag on tool registration, not the tool name — a tool author cannot escape by renaming.
- **First Launch:** No interactive wizard. amundsen-config writes a fully-denied annotated YAML and points at the README.

## Interfaces

- **Check:** Synchronous: given a guarded action and args, return Allow / Deny / PromptRequired. PromptRequired carries the prompt shape; the caller emits PromptRequested, awaits PromptResponse, and calls back to record the decision and resolve.
- **Effective View:** Immutable snapshot of the merged session view with per-grant scope attribution. TUI re-fetches on PermissionsChanged.
- **Events:**
  - PromptRequested — action, args, shape (tool four-tier | context-file two-tier | edit binary); flows through core event stream
  - PermissionsChanged — a grant added, removed, or modified at some scope; flows through core event stream
- **Commands:**
  - PromptResponse — developer's choice for an outstanding PromptRequested; received via core command channel

## Decisions

- **Default-deny is the floor; no carve-out for "obviously safe" tools.** — A "Read is always safe" exception would invite future "shell ls is always safe" exceptions. Uniformity of friction is structural, not negotiable.

- **Cross-scope precedence is higher-wins for both allow and deny.** — The session is the developer's deliberate space; honour a session-scope override in either direction. Within a scope, deny beats allow.

- **Four-tier tool prompt (once / session / project / always), symmetric for allow and deny.** — Reaching every scope from the prompt itself avoids punting persistence to follow-up config edits. Locking something down does not deserve to be a hidden ceremony.

- **Context-file prompts are two-tier (project / session); no "once", no global.** — Context-file paths are intrinsically project-scoped (a global tier has no future-project path to pre-trust). Injection is system-prompt-level, so "once" has no meaningful boundary.

- **Context-file decisions are path-keyed only, no content hash.** — Path + content hash would re-prompt on every typo fix, training the developer to click through as reflex — worse safety than trusting them to vet upstream changes to a project file they already approved.

- **Grammar supports arg-pattern matching, not just per-binary toggles.** — `shell:cargo test*` ≠ `shell:cargo install*`. Coarse per-binary grants would dominate in practice and erode deliberate allowance.

- **MCP tool calls use the standard tool prompt, extended once with an edit-shape follow-up.** — The four-tier prompt is the deliberation moment. A parallel "show me args every call even when allowed" review layer would split mental models without adding structural protection. The one exception is the edit-shape question at first invocation — if marked, subsequent calls route through the Edit binary approval gate so MCP cannot bypass the friction Amundsen's own Edit tool enforces. Marking happens at first call, never at upfront config — see pitfall below.

- **Adding an MCP server to config is implicit consent to spawn-and-enumerate.** — The config edit is the consent. The tool-call layer remains default-denied per tool.

- **Edit is never allowlistable; enforcement via edit_class flag on tool registration.** — Keying on the flag rather than the tool name prevents bypass by renaming. The engine rejects any attempt to attach a non-binary prompt to an edit_class tool.

- **No first-run wizard; first launch writes a fully-denied annotated YAML.** — A wizard trains the developer to set permissions in the abstract. Understanding develops by encounter, not by upfront configuration.

## Pitfalls

- A "Read is always safe" carve-out drifting in under the banner of ergonomics — every carve-out is permanent.
- The four-tier prompt growing a fifth option ("allow for this subtree", "until next reload") — each tier doubles cognitive load.
- Persisted denies silently overwritten by later allow at the same scope — storage must express deny-wins, not last-write-wins.
- effective_view snapshot going stale because the TUI polled instead of subscribing to PermissionsChanged.
- edit_class enforcement keyed to tool name instead of the registration flag — escapable by renaming.
- Session-scope allowances leaking into project storage via a confused "remember this" path — pass the persist tier end-to-end.
- Path-keyed context-file decisions accumulating cruft as projects move or files rename — GC deferred past V0.
- Edit-shape marking for MCP tools migrating into upfront config (asked at server registration rather than at first call) — defeats the encounter-driven design and re-creates the wizard this spec rejects.

## Out of Scope

- On-disk YAML schema and scope-storage file layout — amundsen-config.
- TUI rendering of prompts and the permissions panel — amundsen-tui.
- Tool implementations (Read, Explain, Edit, shell, MCP) — amundsen-tools.
- MCP transport, subprocess lifecycle, tool discovery — amundsen-tools via rmcp.
- Edit approval surface (diff format, syntax highlighting) — amundsen-tools.
- Session persistence and prompt history — out of V0 per parent.
- GC of unreachable context-file paths — flagged, deferred past V0.
- Audit log of grant changes — out of V0; TUI shows current state only.

## References

- .claude/spec/amundsen.md — parent; default-deny and friction-as-feature decisions.
- .claude/spec/amundsen-core.md — core's command/event surface the prompt round-trip plugs into.
- .claude/spec/amundsen-config.md — on-disk persistence of project/global grants.
