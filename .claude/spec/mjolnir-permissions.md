# mjolnir-permissions

Default-deny permission engine — three persistent scopes, tiered prompts, friction by design.

**Status:** active — one known gap, see Progress below
**Scope:** mjolnir-permissions crate only. Policy engine, allowlist shape, prompt round-trip. Excludes TUI rendering, YAML I/O (config), and tool implementations.
**Owner:** Maximilian
**Last Updated:** 2026-05-20

**Progress (2026-08-29):** Everything else in this spec is implemented and
tested — `6c60023`. Not yet built: the "MCP tool calls use the standard
tool prompt, extended once with an edit-shape follow-up" Decision (line
57) and the matching Design bullet on MCP edit-shape detection. Building
it needs a new prompt payload/response shape here plus a new persisted
field in mjolnir-config (grant strings are deliberately opaque
`kind:pattern`, not a good fit for the path-arg/content-arg mapping) — a
real, separable follow-up, not a prerequisite for the rest of this spec.
Every MCP tool currently goes through the plain four-tier prompt with
`edit_class: false`. Keep this spec active until that's built.

**Progress (2026-08-30, storage race audit-fix):** A rust-skills audit
(m01-ownership through m15-anti-pattern, unsafe-checker, coding-guidelines)
plus a follow-up 3-pass verification found this spec's own "Persisted
denies silently overwritten by later allow" Pitfall was live, just not in
the form it names: mjolnir-config's `Config::with_permissions_mut` (and the
`with_mcp_mut`/`with_context_files_mut`/`set_provider`/`set_tui` siblings)
released its read lock before mutating and only reacquired a write lock for
the final swap, leaving a window where `/reload-config` (interceptor task)
could land its own read-modify-write between a grant write's disk write and
its in-memory swap and get silently reverted — a real lost-update, not just
an allow/deny ordering bug. Fixed by holding a single write lock across the
whole read → mutate → persist → swap sequence in every one of those
helpers; see mjolnir-config's `store.rs` and its new
`concurrent_grant_writes_do_not_lose_updates` regression test.

**Progress (2026-09-02, directory-scope prompt option):** Developer report:
permissions felt "aggressive" for ordinary reading — each new file under an
already-trusted directory re-triggered its own prompt, since the tool
four-tier prompt only ever persisted an exact-match grant of the literal
target the check ran against, even though the Grammar (line 45,
`read:./**`) already supports a path-glob pattern. The gap was pure
wiring, not the engine: `record_tool_decision`'s `pattern` param was
already caller-chosen (its own doc comment says so), but the one real
caller (mjolnir-tools' dispatcher) always passed the original `target`
verbatim, and nothing upstream ever offered the developer a coarser
option. Fixed without changing the engine's Model/Decisions at all —
`check_tool` gained a `path_like: bool` param (threaded from a new
`Tool::permission_target_is_path` in mjolnir-tools), carried unchanged
into `PromptPayload::Tool` for the TUI to act on; `PromptResponse::Tool`
gained a `pattern: String` field so the developer's own chosen grant
(exact target, or a `<dir>/**` glob when they toggled to it) is what
actually gets persisted, not always the target `check_tool` was called
with. mjolnir-tui's decision panel now shows a humanized "Claude wants to
read a file" title with the literal `kind: target` dimmed underneath (a
separate developer complaint that the raw title alone didn't say what was
being asked), plus a Tab-toggle scope hint line for any path-like target
with an enclosing directory — the four-tier options list itself is
unchanged (still 8 labeled entries; the toggle picks which pattern they'd
persist, not a ninth option, per this spec's own Pitfall on the tier list
growing). Edit-class prompts are untouched: `check_tool`'s edit_class
branch and `record_tool_decision`'s edit_class refusal both short-circuit
before `path_like`/`pattern` ever come into play, per this spec's Edit
Exception and mjolnir's own non-negotiable "Edit is never allowlistable"
constraint — no scope toggle, no directory grant, no tier list, ever, for
Edit. See mjolnir-tools.md and mjolnir-tui.md's matching Progress notes.

**Progress (2026-09-02, permissions.yaml lost its explanation on the first
write):** Developer report: "the permissions model is not clear, and editing
permissions.yaml doesn't really appear to make any sense." Root cause was
entirely in mjolnir-config, not this engine — see that (archived) spec's
matching 2026-09-02 post-archive fix for the full account. In short: the
annotated, comment-explained `permissions.yaml` mjolnir-config writes on
first launch lost every comment the moment any grant was persisted (which in
ordinary use is almost immediately — the first "for this project"/"always"
choice at a four-tier prompt), because the write path re-serializes the
in-memory value from scratch with no way to carry a source file's original
comments along. A developer opening their real, in-use `permissions.yaml`
therefore found a bare `version`/`allow: [...]`/`deny: [...]` with no
explanation of the `kind:pattern` grammar, the session/project/global scope
model, or that a hand-added `edit:...` entry parses fine but has no effect
(Edit is never allowlistable — see the Edit Exception below). Fixed by
threading each domain's header through every write, not just the first one,
and expanding what the permissions header actually explains — this spec's
Model/Decisions/Grammar were already correct and needed no change; the gap
was purely in how (and how much of the time) that model got explained to the
developer looking at the file. Kept as one line here since this is the spec
a developer investigating "permissions felt confusing" would open first —
this file's own 2026-09-02 directory-scope entry above is the other half of
the same live-session feedback batch.

**Progress (2026-09-03, the prompt now says what it would write; the deny
tiers stopped being offered):** Follow-up feedback on the same theme as the
entry above — "permissions are not clear, are we approving the tool? are we
approving the directory? what are we concretely doing" — and, alongside it,
"do we need all of the deny options?" Both were answered entirely in
mjolnir-tui (see its matching Progress note): the decision panel now states
the literal `kind:pattern` rule a saved answer would add, in exactly the
form it takes in `permissions.yaml`, and each option says how long it lasts
and which file, if any, it lands in. Nothing in this engine changed — no
Model, Grammar, Decisions, or API — which is the point worth recording here:
the four-tier prompt's symmetry (allow and deny at every tier) was a
*presentation* choice this spec never required, and `record_tool_decision`
still accepts `Decision::Deny` at any `ToolTier` for a caller that wants it.
What the panel offers is now four allow tiers and a single non-persisting
deny; a standing "never do this" rule is a deliberate `permissions.yaml`
edit, which is also the reading this spec's own annotated header teaches. The
tier-list Pitfall below still holds and is not weakened by this: it guards
against the list *growing* a ninth option, and the list got shorter.

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
- **Prompt Round Trip:** On Deny-by-absence: engine emits PromptRequested with action, args, and shape; caller awaits PromptResponse and calls back to record. Session decisions persist to in-memory engine state; project / global decisions persist via mjolnir-config.
- **Context Files:** CLAUDE.md / AGENTS.md ingestion gated by the two-tier prompt. Decline means do not inject. Decisions are path-keyed by absolute path with no content hash — see decision below. The session initializer (cli crate) tests each candidate file before composing the additional-context string.
- **MCP Connection:** Adding an MCP server to config implicitly authorises spawn-and-enumerate; no connect-time prompt. Each tool the server advertises is default-denied and flows through the four-tier prompt on first invocation. The first-invocation prompt also asks whether the tool is edit-shaped — if yes, the developer supplies the path-arg and content-arg names and subsequent calls route through the Edit binary approval gate. Tools whose shape cannot be pre-diffed (arbitrary patch / partial edit / mutation by query) cannot inherit the gate and stay on the standard four-tier prompt.
- **Edit Exception:** Edit is never allowlistable in any scope. The prompt shape collapses to approve/deny per invocation. Enforcement is keyed to an edit_class flag on tool registration, not the tool name — a tool author cannot escape by renaming.
- **First Launch:** No interactive wizard. mjolnir-config writes a fully-denied annotated YAML and points at the README.

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

- **MCP tool calls use the standard tool prompt, extended once with an edit-shape follow-up.** — The four-tier prompt is the deliberation moment. A parallel "show me args every call even when allowed" review layer would split mental models without adding structural protection. The one exception is the edit-shape question at first invocation — if marked, subsequent calls route through the Edit binary approval gate so MCP cannot bypass the friction Mjolnir's own Edit tool enforces. Marking happens at first call, never at upfront config — see pitfall below.

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

- On-disk YAML schema and scope-storage file layout — mjolnir-config.
- TUI rendering of prompts and the permissions panel — mjolnir-tui.
- Tool implementations (Read, Explain, Edit, shell, MCP) — mjolnir-tools.
- MCP transport, subprocess lifecycle, tool discovery — mjolnir-tools via rmcp.
- Edit approval surface (diff format, syntax highlighting) — mjolnir-tools.
- Session persistence and prompt history — out of V0 per parent.
- GC of unreachable context-file paths — flagged, deferred past V0.
- Audit log of grant changes — out of V0; TUI shows current state only.

## References

- .claude/spec/mjolnir.md — parent; default-deny and friction-as-feature decisions.
- .claude/spec/mjolnir-core.md — core's command/event surface the prompt round-trip plugs into.
- .claude/spec/mjolnir-config.md — on-disk persistence of project/global grants.
