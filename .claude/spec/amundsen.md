# Amundsen

A coding agent harness where the developer's understanding is the product, not the agent's throughput.

**Status:** active
**Scope:** Entire project — core, TUI, LLM client, tool layer.
**Owner:** Maximilian
**Last Updated:** 2026-05-20

## Why

A coding agent harness where the developer is first-class and the LLM is an assistant — not a code-generation firehose. Optimises for understanding, conscious craft, and learning rather than throughput. The inverse of Claude Code, OpenCode, and Cursor: those minimise user intervention; this one treats active engagement as the point. The lineage is "tools for thought" applied to coding assistants. The name reflects this — Roald Amundsen reached the South Pole because he planned methodically where others improvised.

## Tagline

A tool for thought.

## Mascot

- **Archetype:** A small, careful automaton in the form of a little owl (Athene noctua). Honestly mechanical — contemporary-robot finish rather than period brass — with personality expressed through perched stillness and considered head rotation.
- **Why:** An animal mascot mythologises the LLM as a creature with intent; a robot in the form of an animal sidesteps that. The little owl is Athena's bird, the literal source of the wise-owl trope. Perching over locomotion as the visual cue for patience and observation. "Tool for thought" gets a literal reading.
- **Constraints:**
  - Boxy / geometric silhouette that survives ASCII rendering at small sizes.
  - One expressive feature carries personality — large camera-iris eye-lenses with subtle aperture motion.
  - Perched posture; head rotation rather than ambulatory movement.
  - Talons grip but do not act — can hold context (a bookmark, a paper) but do not edit.
  - Matte industrial finish in the contemporary-robot lineage (Wall-E, Anki Vector); not steampunk.
  - Slight wear and character marks — considered use, not factory-fresh.
  - Do not replicate the silhouette of any existing owl mascot (Bubo, Hedwig, Owl from The Owl House).
- **Status:** Form factor decided (little owl, matte industrial, perched, eye-lenses as expressive feature). Silhouette refinements and colour palette still open.

## Workspace

- **Layout:** Cargo workspace with seven member crates under `crates/`. Trait definitions live in the crate that owns the boundary; concrete impls live in siblings that depend on it.
- **Crates:**
  - **amundsen-core** (`crates/core`)
    - Role: Agent loop, append-only log, event/command types, LlmClient and ToolDispatcher trait defs.
    - Depends on: (none)
    - Spec: .claude/spec/amundsen-core.md
  - **amundsen-llm** (`crates/llm`)
    - Role: LlmClient implementations. V0 Anthropic; V0.5 OpenAI-compat adapter covering Qwen, Kimi, Together, Fireworks, OpenRouter, vLLM, Ollama.
    - Depends on: amundsen-core
    - Spec: .claude/spec/amundsen-llm.md
  - **amundsen-config** (`crates/config`)
    - Role: Per-domain YAML files, scope resolution (session > project > global), persistence.
    - Depends on: (none)
    - Spec: .claude/spec/amundsen-config.md
  - **amundsen-permissions** (`crates/permissions`)
    - Role: Default-deny permission engine. Three scopes, allowlist persistence, per-file CLAUDE.md and AGENTS.md prompt tracking. Cross-cutting — any crate that gates an action calls into this one.
    - Depends on: amundsen-config
    - Spec: .claude/spec/amundsen-permissions.md
  - **amundsen-tools** (`crates/tools`)
    - Role: ToolDispatcher impl. Built-in tools (Read, Explain, Edit, shell), MCP bridge via rmcp, Edit approval gate.
    - Depends on: amundsen-core, amundsen-permissions, amundsen-config
    - Spec: .claude/spec/amundsen-tools.md
  - **amundsen-tui** (`crates/tui`)
    - Role: ratatui frontend. Renders the event stream from the core, submits commands.
    - Depends on: amundsen-core
  - **amundsen** (`crates/cli`)
    - Role: Binary crate. Session bootstrap (composes the additional-context string handed to the core), wires concrete trait impls into the core, runs the TUI.
    - Depends on: amundsen-core, amundsen-llm, amundsen-config, amundsen-permissions, amundsen-tools, amundsen-tui

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

- **Tool sourcing — built-ins ship in the binary; MCP is the extension surface.** — V0 ships Read, Explain, Edit, shell as built-ins. Additional capabilities via MCP through rmcp. New MCP servers connect with all tools fully denied; the developer promotes individual tools to the allowlist deliberately. The friction layer is owned by Amundsen regardless of tool origin — MCP descriptors are suggestions Amundsen may refuse, downgrade, or wrap.

- **Permission model — default-deny across all surfaces, three persistent scopes.** — No tool may read, write, edit, or shell without explicit grant. Scopes: session > project > global; deny beats allow within a scope. No interactive first-run wizard — first launch writes a fully-denied annotated YAML. CLAUDE.md and AGENTS.md trigger per-file prompts. Current effective permissions are visible in the TUI at all times.

- **Edit happens only on explicit per-edit request, never allowlistable.** — Default is the strong reading — the developer must explicitly say "edit X". Configurable to a weaker reading (agent may propose diffs the developer then approves) but strong is default. Edit is never allowlistable in any configuration — friction on Edit is structural, not a setting.

- **No session persistence, no first-class history, developer-authored memory in V0.** — Sessions are ephemeral. Memory is developer-authored — Amundsen does not propose entries or prompt at end of session. Privacy is local-only with no telemetry; inference is governed by the chosen model provider (Anthropic in V0; local models possible once V0.5 ships).

## Pitfalls

- Anthropic wire types leaking past LlmClient — V0.5 becomes a refactor instead of an adapter swap. Audit early.
- Treating Qwen3-Coder as plain OpenAI-compatible in V0.5 — Alibaba's recommended tool-call format is XML-ish and measurably better than OpenAI JSON for Qwen3-Coder.
- Over-abstracting LlmClient in V0 before the second adapter exposes real requirements.
- Rust compile times slowing agent-loop iteration — plan for fast inner-loop builds via narrow crates around the agent loop and watch-mode tests.
- UX defaults that just apply changes, or that soften default-deny, drifting in under the banner of ergonomics or parity with Claude Code/OpenCode. Friction is the product — users who want frictionless already have Claude Code; users come to Amundsen for the friction.

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

- .claude/spec/amundsen-core.md — agent loop, conversation state, typed LLM/tool boundary.
- .claude/spec/amundsen-llm.md — Anthropic client, SSE, wire-level retry, cache placement, provider config.
- .claude/spec/amundsen-permissions.md — default-deny engine, scope precedence, tiered prompt round-trip.
- .claude/spec/amundsen-config.md — per-domain YAML, project and global scope, refuse-to-start.
- https://docs.anthropic.com/en/api/messages — Anthropic Messages API.
- https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching — prompt caching.
- https://ratatui.rs/ — ratatui.
- https://github.com/modelcontextprotocol/rust-sdk — rmcp.
- https://github.com/sst/opencode — OpenCode reference architecture (TS, provider-agnostic).
- https://github.com/QwenLM/qwen-code — Qwen Code (Gemini CLI fork tuned for Qwen3-Coder).
- https://qwenlm.github.io/blog/qwen3-coder/ — Qwen3-Coder model card and tool-call format.
- https://andymatuschak.org/ — Andy Matuschak, tools for thought research.
- http://worrydream.com/LearnableProgramming/ — Bret Victor, "Learnable Programming".
