# Mjolnir

A coding agent harness where the developer's understanding is the product, not the agent's throughput.

**Status:** active
**Scope:** Entire project — core, TUI, LLM client, tool layer.
**Owner:** Maximilian
**Last Updated:** 2026-05-20

## Why

A coding agent harness where the developer is first-class and the LLM is an assistant — not a code-generation firehose. Optimises for understanding, conscious craft, and learning rather than throughput. The inverse of Claude Code, OpenCode, and Cursor: those minimise user intervention; this one treats active engagement as the point. The lineage is "tools for thought" applied to coding assistants. The name reflects this — Mjolnir is a tool, not an agent with a will of its own: inert on the wall, silent through however much deliberation the wielder needs, and only ever swung on an explicit decision to swing it — at which point it acts with total, decisive force, no hesitation and no half-measures. That's the harness's whole posture in one image: quiet until the developer decides, then precise and complete once they have. (Renamed 2026-08-29 from Amundsen, whose own rationale — Roald Amundsen reached the South Pole because he planned methodically where others improvised — is kept in git history, not contradicted: careful preparation followed by fast, decisive execution is one idea wearing two names.)

## Tagline

A tool for thought.

## Mascot

- **Archetype:** Mjolnir — Thor's hammer. Superseded the little-owl archetype below at explicit developer direction (2026-08-29) toward something more "aggressive/directive," in the same spirit the owl was chosen for in the first place: "what we empower the developer/user to be." A hammer sidesteps the animal-mascot concern even more directly than the robot-owl did — it isn't a creature at all, it's a tool, always in the wielder's hand and never acting on its own, which reads as a literal restatement of "tool for thought" from the user's side rather than the LLM's.
- **Why:** Not a redesign exercise — a real photo reference, traced. `mjolnir-tui.md`'s "mascot pivot to Mjolnir" Progress entry has the full history: several procedurally-generated candidates (a cobra, a "tribal" infinity mark, two hand-coded Mjolnir attempts with a crosshatch-weave texture) were tried and rejected, most pointedly for not being "a true representation" of the reference images supplied. What landed, `ui::MJOLNIR_ART`, is a literal pixel trace of a real Mjolnir illustration (thresholded, trimmed, resized, read back one source pixel per Braille dot) rather than an original design — the opposite instinct from the owl's from-scratch invention, and worth remembering next time an original-mascot design stalls: a faithful trace of an existing well-composed reference can beat many rounds of procedural approximation.
- **Constraints:** None formally revisited for Mjolnir — unlike the owl, this mark's fidelity is judged against its real-world reference image, not a set of stated silhouette/finish rules.
- **Status:** Decided and implemented — `ui::MJOLNIR_ART` in the TUI's welcome banner (`mjolnir-tui.md`'s "mascot pivot to Mjolnir" Progress entry, `ui::intro_lines`). Uses the TUI's existing accent color, same as the owl did; the mascot color-palette question below is about the owl era and is moot now that the mark itself changed.
- **Superseded — the little-owl archetype (kept for history):** A small, careful automaton in the form of a little owl (Athene noctua), honestly mechanical rather than period brass, personality via perched stillness and considered head rotation. Chosen because an animal mascot mythologises the LLM as a creature with intent, while a robot in animal form sidesteps that; the little owl is Athena's bird, the literal source of the wise-owl trope, with perching over locomotion as the visual cue for patience and observation. Constraints that came with it: boxy/geometric silhouette surviving ASCII rendering at small sizes; one expressive feature (large camera-iris eye-lenses with subtle aperture motion) carrying personality; perched posture with head rotation, not ambulatory movement; talons that grip but do not act (can hold context, cannot edit); matte industrial finish in the Wall-E/Anki Vector lineage, not steampunk; slight wear and character marks, not factory-fresh; and no replicating an existing owl mascot's silhouette (Bubo, Hedwig, Owl from The Owl House). Implemented once as an ASCII owl in the welcome banner before being replaced — see `ui.rs`'s git history for that version.

## Workspace

- **Layout:** Cargo workspace with seven member crates under `crates/`. Trait definitions live in the crate that owns the boundary; concrete impls live in siblings that depend on it.
- **Crates:**
  - **mjolnir-core** (`crates/core`)
    - Role: Agent loop, append-only log, event/command types, LlmClient and ToolDispatcher trait defs.
    - Depends on: (none)
    - Spec: .claude/spec/mjolnir-core.md
  - **mjolnir-llm** (`crates/llm`)
    - Role: LlmClient implementations. V0 Anthropic; V0.5 OpenAI-compat adapter covering Qwen, Kimi, Together, Fireworks, OpenRouter, vLLM, Ollama.
    - Depends on: mjolnir-core
    - Spec: .claude/spec/mjolnir-llm.md
  - **mjolnir-config** (`crates/config`)
    - Role: Per-domain YAML files, scope resolution (session > project > global), persistence.
    - Depends on: (none)
    - Spec: .claude/spec/mjolnir-config.md
  - **mjolnir-permissions** (`crates/permissions`)
    - Role: Default-deny permission engine. Three scopes, allowlist persistence, per-file CLAUDE.md and AGENTS.md prompt tracking. Cross-cutting — any crate that gates an action calls into this one.
    - Depends on: mjolnir-config
    - Spec: .claude/spec/mjolnir-permissions.md
  - **mjolnir-tools** (`crates/tools`)
    - Role: ToolDispatcher impl. Built-in tools (Read, Explain, Edit, shell), MCP bridge via rmcp, Edit approval gate.
    - Depends on: mjolnir-core, mjolnir-permissions, mjolnir-config
    - Spec: .claude/spec/mjolnir-tools.md
  - **mjolnir-tui** (`crates/tui`)
    - Role: ratatui frontend. Renders the event stream from the core, submits commands.
    - Depends on: mjolnir-core
  - **mjolnir** (`crates/cli`)
    - Role: Binary crate. Session bootstrap (composes the additional-context string handed to the core), wires concrete trait impls into the core, runs the TUI.
    - Depends on: mjolnir-core, mjolnir-llm, mjolnir-config, mjolnir-permissions, mjolnir-tools, mjolnir-tui

## Decisions

- **Rust for both the core and the V0 TUI.** — Single static-binary distribution, best-in-class TUI via ratatui, strong primitives for subprocess/LSP/MCP, and the user already maintains a Rust FFI core. Rejected: Kotlin/JVM (weak TUI, fat distribution), TypeScript (same stack as Claude Code and OpenCode — defeats the point), Go (no compelling advantage), KMP (mobile sharing unused since clients are TUI then web).

- **V0 frontend is TUI only; V1 may add a web UI.** — TUI plus future web mandates a client/server-shaped boundary inside the process. Event/command types should serialise cleanly to JSON-RPC for V1 even if not yet wire-serialised in V0.

- **V0 supports Anthropic only; V0.5 adds an OpenAI-compatible adapter.** — Single-provider V0 lets the agent loop exploit Claude-specific features — caching breakpoints, extended thinking, real tool-use semantics — rather than degrading to a lowest-common-denominator abstraction. V0.5's OpenAI-compat adapter buys Qwen, Together, Fireworks, OpenRouter, vLLM, Ollama.

- **Write a thin Anthropic client over reqwest + eventsource-stream.** — No official Anthropic Rust SDK. Community crates (anthropic-sdk, misanthropic, clust) lag behind. The Messages API surface is small; direct control over caching, retry, and streaming beats fighting an SDK abstraction.

- **Minimal LlmClient trait boundary from day one, even with a single provider.** — Prevents Anthropic wire types leaking into the agent loop. Discipline is "design for the second case, build only the first" — do not pre-build the capability-aware abstraction until V0.5's second adapter validates it.

- **Use the official rmcp crate for MCP client work.** — First-party Rust MCP SDK; reinventing the transport adds no value.

- **No OS-level sandboxing in V0.** — Neither Claude Code nor OpenCode implements seccomp/landlock isolation; permission prompts and command allowlists are the established model. Revisit if a credible threat model emerges.

- **Agent loop is discussion-first; action only on explicit user signal.** — Resting state is conversation. The agent proposes, explains, surfaces tradeoffs; the developer drives. Not OpenCode's build/plan toggle, not Claude Code's act-first model. Action requires an explicit signal ("apply", "do it", "go ahead").

- **Read and Explain are first-class tools; Edit has deliberate friction.** — Edits propose-then-wait by default, with the diff visible and approval required. Friction on Edit preserves the developer's role as conscious author. See the per-edit-request decision below. Diff is not a tool — `shell:git diff*` covers the VCS-aware case and `shell:diff*` is the non-repo fallback; the internal diff-rendering primitive Edit uses is an implementation detail of the approval surface, not a callable affordance.

- **Tool sourcing — built-ins ship in the binary; MCP is the extension surface.** — V0 ships Read, Explain, Edit, shell as built-ins. Additional capabilities via MCP through rmcp. New MCP servers connect with all tools fully denied; the developer promotes individual tools to the allowlist deliberately. The friction layer is owned by Mjolnir regardless of tool origin — MCP descriptors are suggestions Mjolnir may refuse, downgrade, or wrap.

- **Permission model — default-deny across all surfaces, three persistent scopes.** — No tool may read, write, edit, or shell without explicit grant. Scopes: session > project > global; deny beats allow within a scope. No interactive first-run wizard — first launch writes a fully-denied annotated YAML. CLAUDE.md and AGENTS.md trigger per-file prompts. Current effective permissions are visible in the TUI at all times.

- **Edit happens only on explicit per-edit request, never allowlistable.** — Default is the strong reading — the developer must explicitly say "edit X". Configurable to a weaker reading (agent may propose diffs the developer then approves) but strong is default. Edit is never allowlistable in any configuration — friction on Edit is structural, not a setting.

- **Developer-authored memory; sessions persist but nothing crosses between them.** — Amended by ADR 0005, which reversed the original "sessions are ephemeral" clause: a conversation is written to disk as it happens and `/resume` picks one back up. The rest of this Decision stands unchanged and is what ADR 0005 was careful not to touch — memory is developer-authored, Mjolnir does not propose entries or prompt at end of session, and nothing is carried into a *new* session by itself. Privacy is local-only with no telemetry; inference is governed by the chosen model provider (Anthropic in V0; local models possible once V0.5 ships).

## Pitfalls

- Anthropic wire types leaking past LlmClient — V0.5 becomes a refactor instead of an adapter swap. Audit early.
- Treating Qwen3-Coder as plain OpenAI-compatible in V0.5 — Alibaba's recommended tool-call format is XML-ish and measurably better than OpenAI JSON for Qwen3-Coder.
- Over-abstracting LlmClient in V0 before the second adapter exposes real requirements.
- Rust compile times slowing agent-loop iteration — plan for fast inner-loop builds via narrow crates around the agent loop and watch-mode tests.
- UX defaults that just apply changes, or that soften default-deny, drifting in under the banner of ergonomics or parity with Claude Code/OpenCode. Friction is the product — users who want frictionless already have Claude Code; users come to Mjolnir for the friction.

## Out of Scope

- Native GUI clients (SwiftUI, Compose, desktop GUI) — TUI for V0, web for V1, nothing else.
- Mobile (iOS, Android) as a host for the core — desktop-only.
- Provider-agnostic abstraction beyond Anthropic + OpenAI-compatible (Gemini, Bedrock, Vertex).
- OS-level sandboxing (seccomp, landlock, containers) in V0.
- Multi-language core (KMP, JVM, TypeScript, Go) — considered and rejected.
- Autonomous long-running agent runs without user check-in; multi-step edit sequences.
- Vibe-coding / code-generation-firehose UX — default is discussion, not output.
- Throughput metrics (lines/min, edits/session) as success criteria — quality and understanding are the goals.

## References

- .claude/spec/mjolnir-core.md — agent loop, conversation state, typed LLM/tool boundary.
- .claude/spec/mjolnir-llm.md — Anthropic client, SSE, wire-level retry, cache placement, provider config.
- .claude/spec/mjolnir-permissions.md — default-deny engine, scope precedence, tiered prompt round-trip.
- .claude/spec/mjolnir-config.md — per-domain YAML, project and global scope, refuse-to-start.
- https://docs.anthropic.com/en/api/messages — Anthropic Messages API.
- https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching — prompt caching.
- https://ratatui.rs/ — ratatui.
- https://github.com/modelcontextprotocol/rust-sdk — rmcp.
- https://github.com/sst/opencode — OpenCode reference architecture (TS, provider-agnostic).
- https://github.com/QwenLM/qwen-code — Qwen Code (Gemini CLI fork tuned for Qwen3-Coder).
- https://qwenlm.github.io/blog/qwen3-coder/ — Qwen3-Coder model card and tool-call format.
- https://andymatuschak.org/ — Andy Matuschak, tools for thought research.
- http://worrydream.com/LearnableProgramming/ — Bret Victor, "Learnable Programming".
