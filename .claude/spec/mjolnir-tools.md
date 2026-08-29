# mjolnir-tools

ToolDispatcher impl, built-in tool set, Edit approval gate, MCP bridge via rmcp.

**Status:** active — one known gap, see Progress below
**Scope:** mjolnir-tools crate only. Built-in tool implementations, registry, dispatch, Edit approval surface, MCP bridge. Excludes permission policy, agent loop, TUI, config persistence.
**Owner:** Maximilian
**Last Updated:** 2026-05-20

**Progress (2026-08-29):** All four V0 built-ins (Read, Edit, shell,
Explain) and the MCP bridge are implemented and tested — `053792a`,
`bcf6209`, `6eb4f6d`, audit-fixed in `bb8acff`. Explain's LSP support is
V0-scoped to Rust only (rust-analyzer), matching the crate's own
LSP-scope-creep Pitfall rather than a gap. Not yet built: the MCP
first-invocation edit-shape follow-up ("MCP Edit-Shape Detection" — see
mjolnir-permissions.md's matching gap, which this depends on). Every MCP
tool currently registers with `edit_class: false` and never graduates to
Edit's binary approval gate. Keep this spec active until that's built.

## Why

Owns every concrete tool Mjolnir can dispatch — the V0 built-ins (Read, Diff, Explain, Edit, shell) and the MCP bridge that maps remote tools onto the same dispatch surface. Implements core's ToolDispatcher trait. Hosts the Edit approval gate as structural friction the developer cannot configure away. Other crates supply policy and protocol; this crate supplies behaviour.

## Vocabulary

- **Tool:** A registered (name, input schema, edit_class, dispatch fn) tuple. Built-in tools register at startup; MCP tools register lazily on first server enumeration.
- **Registry:** In-process map of name → tool. Single source for both core's ToolDispatcher impl and TUI listing.
- **Edit Class:** Boolean flag set at registration for built-ins, or at first-invocation prompt for MCP tools. When set, the permission engine refuses to attach anything other than the per-call binary approval gate to invocations. For MCP, the marking includes the path-arg and content-arg mapping needed to render the diff at call time.
- **Dispatch:** Resolve name → tool, check permissions, run the future. Concurrent across calls within a step (the core awaits join). Each tool owns its own cancellation behaviour.
- **Approval Gate:** The binary prompt round-trip Edit waits on inside its own future. Distinct from the four-tier permission prompt — approval is per-call, never persisted, never allowlistable.
- **MCP Bridge:** Subprocess host for MCP servers via rmcp. Each remote tool surfaces as a registry entry with a dispatch fn that proxies to the server; edit_class is decided at first invocation via the permission engine's edit-shape follow-up, not at registration.

## Design

- **Registration:** Built-ins register at crate init with a static descriptor (name, JSON schema, edit_class, async fn). MCP tools register after the bridge enumerates a server. Duplicate names are rejected; MCP-supplied names that collide with built-ins are namespaced `<server>:<name>`.
- **Dispatch Flow:** Core calls dispatcher with (name, assembled input). The dispatcher resolves the tool, calls permissions.check, and either runs the future, returns a structured Denied, or — on PromptRequired — emits PromptRequested via the core event stream, awaits PromptResponse on the core command channel, calls back to record the decision, and then continues. Errors are structured, not panics — the model sees them and adapts.
- **Builtin Tools:**
  - **Read** — Read a file from disk. Path-globbed via permissions.
  - **Explain** — LSP-backed code intelligence. Single tool with an `op` enum (definition, references, hover, implementations, workspace_symbols). Output is structured location and signature data only — no prose summaries. LSP servers spawn lazily per-project per-language, persist for session, shut down at process exit. Diffs between refs/paths are handled via `shell:git diff*`; `shell:diff*` is the fallback when the working directory is not a git repository.
  - **Edit** — Propose a single edit (path, before, after). Always per-call approval; the gate lives inside the tool's future. Approval emits ToolApprovalGranted; denial returns structured Denied to the model.
  - **shell** — Run a command. Argv pattern is permission-keyed (e.g. `shell:cargo test*`). Piped output (not PTY), project-root cwd (no per-call override), inherited env, 120s timeout (overridable per call), output capped at 50KB with a truncation marker.
- **Edit Approval:** Edit's future emits ToolApprovalRequested via the core, then awaits the matching ApproveTool/DenyTool command. Approval state is per-invocation; no persistence, no allowlisting. The diff is rendered from the (before, after) the tool already assembled — TUI owns formatting.
- **MCP Lifecycle:** Servers are spawned from config entries on first use of any of their tools. The bridge enumerates tools and routes invocations through rmcp. Spawn failures and protocol errors surface as structured tool errors, not crashes. Servers shut down on process exit; per-session reconnect is out of V0.
- **MCP Edit-Shape Detection:** At first invocation of an MCP tool, the permission engine's four-tier prompt grows a one-time follow-up: "does this tool modify a file? If yes, which arg is the path, which is the new content?" If the developer marks it edit-shaped and supplies the arg mapping, subsequent calls route through Edit's per-call binary approval gate — the bridge reads the file at call time to render the diff against the supplied content. Tools whose shape cannot be pre-diffed (apply_patch, edit_at_line, bulk_replace and the like) fall back to the standard prompt with no diff gate; the developer's choice there is accept-as-is or deny. Marking persists with the permission grant at the chosen scope.
- **Cancellation:** Pure-Rust tools cancel at await points. Shell sends SIGKILL to the process group. MCP invocations drop the response future and best-effort signal the server. Cancellation is advisory from core's perspective — the dispatcher promises to release the slot, not that the OS-level work stopped.

## Interfaces

- **Tool Dispatcher Impl:** Implements core's ToolDispatcher trait. Single entry point: dispatch(name, input, cancellation token) → future of Result<ToolOutput, ToolError>.
- **Registry View:** Read-only listing of registered tools (name, source: builtin|mcp, edit_class, schema). TUI status bar consumes this to display the names of tools currently running within a step.
- **Events:**
  - ToolApprovalRequested — Edit only; carries assembled diff payload
  - McpServerStateChanged — spawn / ready / errored / exited
- **Commands:**
  - ApproveTool — response to ToolApprovalRequested
  - DenyTool — response to ToolApprovalRequested

## Decisions

- **V0 built-in set is exactly Read, Explain, Edit, shell. Nothing else.** — Parent decision. Additional capabilities arrive via MCP, not built-ins. The minimal set is the contract; expanding it would normalise built-in growth as the escape valve. Diff is not a tool — `shell:git diff*` covers the common case, `shell:diff*` is the non-repo fallback, and the internal diff-rendering primitive Edit uses is an implementation detail of the approval surface.

- **Explain is LSP-backed code intelligence; output is structured, factual, concise.** — LSP gives the model navigation affordances (definition, references, hover, implementations, workspace symbols) without paying the token cost of reading every potential caller. Output is structured location and signature data only — no prose. Prose summaries would duplicate what Read + model reasoning already covers.

- **Edit approval gate lives inside the tool's future, not in the dispatcher.** — Keeps the dispatcher uniform — every tool is a future of a Result. Edit's friction is structural to the tool, not a side path. Matches core's "approval-gated tools handle their gate inside the future" decision.

- **edit_class is registration-time and immutable; never derived from tool name.** — Inherited from mjolnir-permissions. Naming-based enforcement is escapable by renaming; flag on the descriptor is not.

- **MCP tools become edit-shaped via a first-invocation follow-up, never via upfront config.** — Edit friction must extend to MCP tools that modify files, but the MCP protocol does not tell Mjolnir which tools those are. Marking at first invocation puts the question at the moment the developer is already paying attention to the call — matches the parent decision against first-run wizards ("understanding develops by encounter"). Tools whose shape cannot be pre-diffed (arbitrary patch / partial edit / mutation by query) cannot inherit the Edit gate and fall back to the standard prompt; that limitation is structural to the diff-rendering contract, not a setting.

- **MCP name collisions namespace under `<server>:<name>`; built-ins win unprefixed.** — Built-ins are the stable surface; remote tools must not silently shadow them. Prefixing is explicit and survives server churn.

- **Tool errors are structured and fed back to the model; transport errors do not retry here.** — Tool-level failure is signal for the model. Transient retry policy belongs to mjolnir-llm for upstream calls, not to the tool layer.

- **Each tool owns its cancellation; the dispatcher only promises to release the slot.** — Shell needs SIGKILL on the process group; pure-Rust tools want await-point abort; MCP wants the response future dropped. A single cancellation primitive at the dispatcher would have to lie about at least one of these.

## Pitfalls

- LSP integration may outgrow this crate — server lifecycle, JSON-RPC client, capability negotiation, and per-language config (rust-analyzer, sourcekit-lsp, kotlin-lsp) are real scope. Split into mjolnir-lsp if it eats more than ~25% of this crate's surface.
- Edit-shape marking for MCP tools drifting back into upfront config (e.g. a UI flow that asks at server registration rather than at first call) — defeats the encounter-driven design and re-creates the wizard the parent spec rejected.
- MCP edit-shape arg mapping going stale if a server changes its tool schema between sessions — detect schema-hash mismatch on the marked tool and re-prompt, do not silently reuse the old mapping.
- Approval state for Edit accidentally caching across calls "for ergonomics" — the gate is per-invocation by construction; any cache is a bypass.
- Concurrent tool calls in a step racing on shared resources (same file edited twice, same process group signalled twice) — dispatcher is concurrent; tools must be reentrant or self-serialise.
- rmcp version drift silently changing the wire shape under us — pin a known-good version and surface protocol mismatches as structured tool errors, not panics.
- A built-in growing a fifth member under the banner of "small obvious addition" — every addition is permanent surface. Channel it through MCP first.

## Out of Scope

- Permission policy, scope precedence, allowlist storage — mjolnir-permissions.
- Agent loop, append-only log, turn/step semantics — mjolnir-core.
- TUI rendering of diffs, tool listings, approval dialogs — mjolnir-tui.
- Config file format and persistence of MCP server entries — mjolnir-config.
- Anthropic wire format, SSE, retry — mjolnir-llm.
- Session persistence of approval history — out of V0 per parent.
- Sandboxing of shell execution (seccomp, landlock, containers) — out of V0 per parent.
- Streaming tool outputs (partial deltas during execution) — V0 returns terminal results only.
- Web-search, fetch, or any network tool as a built-in — channel via MCP.

## References

- .claude/spec/mjolnir.md — parent; built-in surface, MCP-as-extension, Edit-as-structural-friction.
- .claude/spec/mjolnir-core.md — ToolDispatcher trait, ToolApprovalRequested / ApproveTool placeholders.
- .claude/spec/mjolnir-permissions.md — check() interface, edit_class enforcement, MCP gating.
- https://github.com/modelcontextprotocol/rust-sdk — rmcp.
- https://modelcontextprotocol.io/specification — MCP protocol surface.
