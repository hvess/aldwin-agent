# Aldwin

A coding agent harness where the developer's understanding is the product, not the agent's throughput.

**Status:** active
**Scope:** Entire project — core, TUI, LLM client, tool layer.
**Owner:** Maximilian
**Last Updated:** 2026-09-29

## Why

A coding agent harness where the developer is first-class and the LLM is an assistant — not a code-generation firehose. Optimises for understanding, conscious craft, and learning rather than throughput. The inverse of Claude Code, OpenCode, and Cursor: those minimise user intervention; this one treats active engagement as the point. The lineage is "tools for thought" applied to coding assistants. The name reflects this — Aldwin is a tool, not an agent with a will of its own: inert on the wall, silent through however much deliberation the wielder needs, and only ever swung on an explicit decision to swing it — at which point it acts with total, decisive force, no hesitation and no half-measures. That's the harness's whole posture in one image: quiet until the developer decides, then precise and complete once they have. (Renamed 2026-08-29 from Amundsen, whose own rationale — Roald Amundsen reached the South Pole because he planned methodically where others improvised — is kept in git history, not contradicted: careful preparation followed by fast, decisive execution is one idea wearing two names.)

## Tagline

You decide what gets written. (The developer's call, 2026-09-27: the ethos is
reviewing the code yourself, so you keep your understanding of it and your say
in how it is shaped. It replaces "A tool for thought.")

## Mascot

- **Superseded by the design system (2026-09-23).** The mark is the Aldwin
  Design System's own: 108 half-block cells in the launch card, generated
  into `tokens.rs` from the frame like every other design value. The traced
  Mjolnir illustration (`ui::MJOLNIR_ART`) is gone with the Mjolnir system,
  and the little-owl archetype before it; both are in git history, with the
  reasoning that chose them in this section's earlier revisions.

## Workspace

- **Layout:** Cargo workspace with seven member crates under `crates/`. Trait definitions live in the crate that owns the boundary; concrete impls live in siblings that depend on it.
- **Crates:**
  - **aldwin-core** (`crates/core`)
    - Role: Agent loop, append-only log, event/command types, LlmClient and ToolDispatcher trait defs.
    - Depends on: (none)
    - Spec: docs/spec/archive/aldwin-core.md
  - **aldwin-llm** (`crates/llm`)
    - Role: LlmClient implementations. V0 Anthropic; V0.5 OpenAI-compat adapter covering Qwen, Kimi, Together, Fireworks, OpenRouter, vLLM, Ollama.
    - Depends on: aldwin-core, aldwin-config
    - Spec: docs/spec/archive/aldwin-llm.md
  - **aldwin-config** (`crates/config`)
    - Role: Per-domain YAML files (project and global scope), the transcript store, persistence. `roots:` in `permissions.yaml` is the one permissions key read (ADR 0011).
    - Depends on: aldwin-core
    - Spec: docs/spec/archive/aldwin-config.md
  - **aldwin-tools** (`crates/tools`)
    - Role: ToolDispatcher impl. Built-in tools (read, edit, run, explain, plan, ask, reload), the staged changeset the review opens over, the sandbox every spawned process runs in (writes only inside the workspace), MCP bridge via rmcp.
    - Depends on: aldwin-core, aldwin-config
    - Spec: docs/spec/aldwin-tools.md
  - **aldwin-tui** (`crates/tui`)
    - Role: ratatui frontend. Renders the event stream from the core, submits commands.
    - Depends on: aldwin-core
    - Spec: docs/spec/aldwin-tui.md
  - **aldwin** (`crates/cli`)
    - Role: Binary crate. Session bootstrap (composes the additional-context string handed to the core), wires concrete trait impls into the core, runs the TUI.
    - Depends on: aldwin-core, aldwin-llm, aldwin-config, aldwin-tools, aldwin-tui
    - Spec: docs/spec/archive/aldwin-cli.md
  - **aldwin-review** (`crates/review`)
    - Role: The submission loop's harness (`/review`): design tokens, rendered frames, the screenshot baselines. Dev-only; never in a release build.
    - Depends on: aldwin-core, aldwin-config, aldwin-llm
    - Spec: docs/spec/aldwin-review.md

## Decisions

- **Rust for both the core and the V0 TUI.** — Single static-binary distribution, best-in-class TUI via ratatui, strong primitives for subprocess/LSP/MCP, and the user already maintains a Rust FFI core. Rejected: Kotlin/JVM (weak TUI, fat distribution), TypeScript (same stack as Claude Code and OpenCode — defeats the point), Go (no compelling advantage), KMP (mobile sharing unused since clients are TUI then web).

- **V0 frontend is TUI only; V1 may add a web UI.** — TUI plus future web mandates a client/server-shaped boundary inside the process. Event/command types should serialise cleanly to JSON-RPC for V1 even if not yet wire-serialised in V0.

- **V0 supports Anthropic only; V0.5 adds an OpenAI-compatible adapter.** — Single-provider V0 lets the agent loop exploit Claude-specific features — caching breakpoints, extended thinking, real tool-use semantics — rather than degrading to a lowest-common-denominator abstraction. V0.5's OpenAI-compat adapter buys Qwen, Together, Fireworks, OpenRouter, vLLM, Ollama.

- **Write a thin Anthropic client over reqwest + eventsource-stream.** (Its own SSE decoder since 2026-09-29: archive/aldwin-llm.md.) — No official Anthropic Rust SDK. Community crates (anthropic-sdk, misanthropic, clust) lag behind. The Messages API surface is small; direct control over caching, retry, and streaming beats fighting an SDK abstraction.

- **Minimal LlmClient trait boundary from day one, even with a single provider.** — Prevents Anthropic wire types leaking into the agent loop. Discipline is "design for the second case, build only the first" — do not pre-build the capability-aware abstraction until V0.5's second adapter validates it.

- **Use the official rmcp crate for MCP client work.** — First-party Rust MCP SDK; reinventing the transport adds no value.

- **No OS-level sandboxing in V0.** — *Reversed by ADR 0004; since ADR 0011 it is the boundary.* Every process Aldwin starts — a `run`, the language server, an MCP server — runs under Landlock (Linux) or Seatbelt (macOS) and can write only inside the workspace (an MCP server not even there, ADR 0014); reads and the network are open. Where neither exists, the developer is told once at startup. The original reasoning — that prompts and allowlists were the established model — described the product ADR 0009 replaced.

- **Aldwin is a co-author of the commits it makes.** — ADR 0013, 2026-09-27: every `git commit` from anything Aldwin starts carries `Co-Authored-By: Aldwin <noreply@aldwin.codes>`. At startup Aldwin puts a symlink to its own binary, named `git`, first on its `PATH`; started under that name the binary execs the real git, adding `--trailer` to a `commit`. With a git older than 2.32, which has no `--trailer`, no shim is installed and that is said once. Merges, cherry-picks, rebases and a git called by absolute path get no trailer; a shim that could not be installed is said once.

- **Agent loop is discussion-first; action follows intent.** — Amended by ADR 0008: resting state is conversation, and action follows the developer's *intent* rather than their grammatical mood. The agent proposes, explains, surfaces tradeoffs; the developer drives. Not OpenCode's build/plan toggle, not Claude Code's act-first model.

- **Read and Explain are first-class tools; Edit has deliberate friction.** — Amended by ADR 0009: an edit is *staged*, every edit of a turn is one changeset, and the changeset is reviewed in a full-window review at the first moment it would be observed on disk — before a run, or at the turn's end. Nothing is written before an approve. Friction on Edit preserves the developer's role as conscious author; it is structural, and there is no setting for it.

- **Tool sourcing — built-ins ship in the binary; MCP is the extension surface.** — Built-ins are read, edit, run, explain, plan, ask and reload (ADR 0004 replaced `shell` with `run`; ADR 0009 added `plan` and `ask`; ADR 0017 added `reload`). Additional capabilities via MCP through rmcp. An MCP tool runs like any other — nothing asks — and, because it executes in its own process over the real tree, the review opens before it exactly as before a run (ADR 0009 §4). An MCP server is given no write access to the workspace (ADR 0014, within its Limits).

- **Permission model — the workspace is the only boundary.** — *Superseded by ADR 0009, then ADR 0011.* Reads and runs need no grant and never ask; every tool refuses a path outside the workspace, and every process Aldwin starts can write only inside it; where that cannot be enforced, the developer is told once. `run` takes a shell command. There is no class and no `deny:` lock. There is no first-run wizard: every launch opens straight to the field under the launch card, and with nothing configured the first message asks provider then model. `CLAUDE.md` and `AGENTS.md` are read into the context without asking, and since 2026-09-28 the skills under `.agents/skills/` and `.claude/skills/` are listed in it by name and description for the model to read when a task calls for one, leaving out any linked from outside every workspace root. The previous Decision — default-deny across every surface, three scopes, per-file prompts — is what ADR 0004 built and ADR 0009 replaced.

- **Edit is never allowlistable.** — Amended by ADR 0008 (intent, not grammar) and ADR 0009 (the review): the agent stages edits when the developer's intent is clear, and the review is where the developer approves, comments on, or discards them. There is nothing to allowlist an edit into. Friction on Edit is structural, not a setting.

- **Developer-authored memory; sessions persist but nothing crosses between them.** — Amended by ADR 0005, which reversed the original "sessions are ephemeral" clause: a conversation is written to disk as it happens and `/resume` picks one back up. The rest of this Decision stands unchanged and is what ADR 0005 was careful not to touch — memory is developer-authored, Aldwin does not propose entries or prompt at end of session, and nothing is carried into a *new* session by itself. Privacy is local-only with no telemetry; inference is governed by the chosen model provider (Anthropic in V0; local models possible once V0.5 ships).

- **The records live in `docs/`, the instructions in `AGENTS.md`.** — The developer's call, 2026-09-27, ahead of open-sourcing: the specs, ADRs and design were under `.claude/`, where only Claude Code looks. Every agent reads a root `AGENTS.md`, so the instructions moved there and `docs/` took the rest. The same day the skills moved to `.agents/skills/`, the location Codex and OpenCode read, and `.claude/` left the repository: for an agent that takes many models, the repository ships nothing specific to one agent, and Claude Code's wiring stays on the developer's machine (the developer's call).

- **MIT, a one-line installer, and CI on every pull request.** — The developer's calls, 2026-09-27, for open-sourcing at `hvess/aldwin-agent`. MIT because OpenCode, Pi and Hermes Agent use it. `install.sh` installs a release only after checking the archive against `SHA256SUMS` and `SHA256SUMS` against its signature, with `allowed_signers` taken from the repository at the release's tag. The signing key was rotated for the move (2026-09-27): `allowed_signers` carries only the current key, and no release before 0.5.0 lives on this repository. `.github/workflows/ci.yml` runs the review loop's stages 2–5 on Linux and macOS; stage 1 compares against a toolchain string recorded from a local build, and 6–8 need an agent. `/update` (the developer's call, 2026-09-29) installs a newer release with the same two checks, against the `allowed_signers` the running binary was built with (aldwin-cli.md's 2026-09-29 entry).

- **The system prompt carries its reasons, examples and boundary cases.** — The developer's call, 2026-09-29, after reading Claude's own consumer system prompt as learning material: apply every lesson from it, in as much detail as it gives. `prompt::BASE` moved to `crates/core/src/prompt.md` and gained which instructions win, what counts as an agreed plan, a scope rule, tool output as material rather than instructions, how to investigate, when not to ask, owning a mistake, the session's model switches, and good-and-not examples with their reasons. ADR 0016 records it, superseding ADR 0008's bar that each clause trace to an observed failure. The context gained the session's date. A second pass the same day checked the prompt against the code: the review is described as the dispatcher runs it (a commented changeset stays staged; calls in one response run together, so an edit and its check go in separate responses; a commented review answers the run instead of running it), the developer's files change only through `edit`, never a command, and new sections cover how a turn works, writing code to the project's conventions, checking work so that "Done" is true, the context budget, `explain`, commands that cannot be taken back, git and secrets. The same day the developer set the voice of every reply: concise, to the point, technical and exact, no jargon or foreign concepts, no walls of text, helpful as an assistant should be. That is the prompt's "How you answer" section, placed second so it governs everything after it.

## Pitfalls

- Anthropic wire types leaking past LlmClient — V0.5 becomes a refactor instead of an adapter swap. Audit early.
- Treating Qwen3-Coder as plain OpenAI-compatible in V0.5 — Alibaba's recommended tool-call format is XML-ish and measurably better than OpenAI JSON for Qwen3-Coder.
- Over-abstracting LlmClient in V0 before the second adapter exposes real requirements.
- Rust compile times slowing agent-loop iteration — plan for fast inner-loop builds via narrow crates around the agent loop and watch-mode tests.
- UX defaults that just apply changes, or that soften default-deny, drifting in under the banner of ergonomics or parity with Claude Code/OpenCode. Friction is the product — users who want frictionless already have Claude Code; users come to Aldwin for the friction.

## Out of Scope

- Native GUI clients (SwiftUI, Compose, desktop GUI) — TUI for V0, web for V1, nothing else.
- Mobile (iOS, Android) as a host for the core — desktop-only.
- Provider-agnostic abstraction beyond Anthropic + OpenAI-compatible (Gemini, Bedrock, Vertex).
- Restricting the network from inside the sandbox — a stated non-goal of ADR 0011.
- Multi-language core (KMP, JVM, TypeScript, Go) — considered and rejected.
- Autonomous long-running agent runs without user check-in; multi-step edit sequences.
- Vibe-coding / code-generation-firehose UX — default is discussion, not output.
- Throughput metrics (lines/min, edits/session) as success criteria — quality and understanding are the goals.

## References

- docs/spec/archive/aldwin-core.md — agent loop, conversation state, typed LLM/tool boundary.
- docs/spec/archive/aldwin-llm.md — Anthropic client, SSE, wire-level retry, cache placement, provider config.
- docs/spec/archive/aldwin-config.md — per-domain YAML, project and global scope, refuse-to-start.
- docs/adr/0009-the-review-is-the-only-gate.md — the review, staging, plan and ask, no first run.
- docs/adr/0011-the-workspace-is-the-only-boundary.md — the one boundary, and the sandbox that holds it.
- docs/adr/0013-aldwin-is-a-co-author-of-the-commits-it-makes.md — the co-author trailer, and the git shim that adds it.
- https://docs.anthropic.com/en/api/messages — Anthropic Messages API.
- https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching — prompt caching.
- https://ratatui.rs/ — ratatui.
- https://github.com/modelcontextprotocol/rust-sdk — rmcp.
- https://github.com/sst/opencode — OpenCode reference architecture (TS, provider-agnostic).
- https://github.com/QwenLM/qwen-code — Qwen Code (Gemini CLI fork tuned for Qwen3-Coder).
- https://qwenlm.github.io/blog/qwen3-coder/ — Qwen3-Coder model card and tool-call format.
- https://andymatuschak.org/ — Andy Matuschak, tools for thought research.
- http://worrydream.com/LearnableProgramming/ — Bret Victor, "Learnable Programming".
