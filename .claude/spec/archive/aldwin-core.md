# aldwin-core

Agent loop, append-only conversation state, and the typed boundary between LLM and tools.

**Status:** archived — implemented, tested, audited
**Scope:** aldwin-core crate only — narrow cut. Excludes tool implementations, permissions, TUI, and provider wire format.
**Owner:** Maximilian
**Last Updated:** 2026-05-16

**Completed:** 2026-08-29. Implemented in full (agent loop, append-only log,
event/command types, LlmClient/ToolDispatcher trait boundary) —
`34760d6`, `440c699`. No known gaps against this spec.

**Post-archive fix (2026-08-29):** A live run surfaced a real bug in
`run_step`: the live in-turn `messages` vector only appended a `ToolUse`
content block's *text* portion to the assistant message, never the
`ToolUse` block itself — only `messages_from_log` (used at the start of a
fresh turn) reconstructed that correctly. Any turn with more than one
step involving a tool call therefore sent a request on step 2+ missing
the assistant's `tool_calls`, which a provider that validates role
sequencing (tool must follow the assistant message that requested it)
rejected outright — "Unexpected role 'tool' after role 'user'" against
an OpenAI-compatible endpoint. Fixed in `afe7033` with a
regression test (`multi_step_turn_carries_tool_use_into_the_next_steps_live_request`)
that inspects the second step's actual request rather than only the log.
This crate stays archived — the fix didn't reopen a design question, but
note it here since "no known gaps" above was wrong until this landed.

**Post-archive addition (2026-08-29, `/clear`):** aldwin-tui's live-feedback
batch (see its own spec's same-day Progress entry) added a way to clear
conversation context mid-session. New `Command::ClearHistory` — handled
identically to `Submit`'s existing mid-turn-rejection shape (a no-op with
a `warn!`, since "forget everything" has no sound meaning while a turn is
still in flight using that same history); between turns, wipes
`ConversationLog` via a new `ConversationLog::clear()` and acknowledges
with a new `Event::HistoryCleared` (empty payload, same shape as
`PermissionsChanged` — tells the layer above to refresh/reset its own
state rather than carrying it). aldwin-cli's `/clear` slash command
forwards `Command::ClearHistory` to the core rather than handling it
locally like `/help`, since core is what owns `ConversationLog`. Still no
open design question — this is an additive command/event pair on the
existing shapes, not a change to the turn/step/log model above.

**Post-archive addition (2026-09-02, `Event::ThemeChanged`):** aldwin-tui
gained a light theme, then a `/theme` slash command to switch it from
inside the harness (see both specs' own same-day Progress entries). Unlike
`/clear`, this needed no `Command` at all — core is never involved in a
theme change, since it's purely a aldwin-cli config write plus a UI-facing
signal. New `Event::ThemeChanged { theme: String }`, same "opaque to core"
shape as `PermissionsChanged`'s payload and the same "core itself never
emits this" reasoning as `Notice` — aldwin-cli's slash-command
interceptor sends it directly into the `Event` channel it already shares
with the TUI (see aldwin-cli's own bootstrap wiring), core's agent loop
never touches it. No open design question here either — an additive event
variant on the existing "layer above core needs a vehicle to reach the
TUI" pattern `Notice`/`PermissionsChanged`/`HistoryCleared` already
established, not a new mechanism.

## Why

The narrow heart of Aldwin — the agent loop, the canonical conversation log, and the typed boundary the LlmClient and ToolDispatcher live behind. The core drives turns and steps and assembles the log. It does not know how to talk to Anthropic, render a TUI, what tools exist, what permissions apply, or what is in CLAUDE.md. Those concerns live in sibling crates so the core stays small, testable, and reusable from both V0's TUI and V1's web client.

## Vocabulary

- **Turn:** One unit of work — user submission through to the model's final response with no tool calls pending. May contain one or many steps. Cancellation, cache breakpoints, and "assistant is done" operate at this level.
- **Step:** One round-trip with the model — a single LLM call and its streaming response.
- **Event:** A typed message emitted upward (toward the TUI, future web client) describing a state change. Designed to serialise cleanly to JSON-RPC notifications.
- **Command:** A typed message accepted downward — submit, cancel, approve. The inverse of `event`.
- **Log:** The canonical conversation state. Append-only. Owned by the core, exposed read-only via snapshots.

## Design

- **State Model:** Single append-only log of structured records (user messages, assistant messages, tool calls, tool results, step boundaries, turn boundaries). Mutation only via the command path; readers receive immutable snapshots. V1's web client reconnects via "give me everything since cursor X".
- **Turn/Step Model:** A turn opens on Submit and closes when an inner step ends with no pending tool calls (or on cancellation / terminal error). A step opens when the LlmClient begins a request and closes on its terminal step-ended event. Both levels are observable so the loop's real round-trip count is visible.
- **Event Flow:** Per step the LlmClient yields: text deltas, thinking start/end markers (content dropped at source), one tool-use-requested per tool call carrying the fully assembled input, and a terminal step-ended carrying stop reason or structured error plus usage and cache stats. The core re-emits these annotated with step/turn IDs and appends to the log.
- **Tool Round Trip:** A step ending with tool_use carries one or more tool calls. The core dispatches concurrently, awaits all results, and starts the next step with results appended. Tool errors feed back to the model but emit upward as visibly distinct events. Approval-gated tools (Edit) wait inside the dispatcher's future; the core just awaits.
- **Cancellation:** Cancel is a hard stop. LLM stream dropped, in-flight tools aborted best-effort (SIGKILL for shell, await-point abort for pure-Rust), turn ends in `cancelled`. Partial output remains in the log as a well-formed entry; the cancelled turn must close cleanly, not leave a torn record.
- **System Prompt:** The core embeds the base Aldwin system prompt (discussion-first, friction on Edit, voice). At session start it receives an opaque additional-context string from the session initializer (working directory, project files if permitted). The core composes `<base>\n\n<session_context>` and sends that as the system prompt to every LLM call. The initializer can only append.

## Interfaces

- **LlmClient Trait:** Single streaming method taking (model, system prompt, tools, messages, cache breakpoints) and returning a stream of normalised events. V0 Anthropic impl lives in aldwin-llm.
- **ToolDispatcher Trait:** Dispatch a tool call by name with assembled input; returns a future resolving to a success or structured error. Approval-gated tools handle their gate inside the future; the core just awaits.
- **Events:**
  - TurnStarted
  - TextDelta
  - ThinkingStart
  - ThinkingEnd
  - ToolUseRequested — assembled tool call; end-only, no streamed JSON
  - ToolDispatched — dispatcher has begun executing the tool
  - ToolApprovalRequested — Edit approval gate; carries diff payload; semantics in aldwin-tools
  - ToolCompleted — tool returned with success or structured error
  - StepEnded — stop reason or structured error, usage, cache stats
  - RetryAttempt — provider-attributed transient retry, surfaced visibly
  - TurnEnded — end_turn, cancelled, or terminal error
  - PromptRequested — permission engine needs a developer decision; semantics in aldwin-permissions
  - PermissionsChanged — a grant was added, removed, or modified; semantics in aldwin-permissions
  - HistoryCleared — `/clear` wiped `ConversationLog`; see the 2026-08-29 post-archive addition above
- **Commands:**
  - Submit — user input opens a new turn
  - Cancel — hard-stop the current turn
  - ApproveTool — approve a pending Edit; semantics in aldwin-tools
  - DenyTool — deny a pending Edit; semantics in aldwin-tools
  - PromptResponse — developer's answer to a PromptRequested; semantics in aldwin-permissions
  - ClearHistory — `/clear`; wipes `ConversationLog`, a no-op mid-turn; see the 2026-08-29 post-archive addition above

## Decisions

- **Append-only conversation log, exposed read-only via snapshots.** — One place owns conversation state. Append-only matches how conversations accrete and catches a class of mutation bugs at compile time. Snapshots give the TUI and future web client a clean read model.

- **Both turn and step are first-class in the event model.** — Turn is what the user thinks about; step is what happens on the wire. Exposing both lets the developer inspect the loop's real behaviour — on-spec for "developer's understanding is the product".

- **LlmClient yields normalised, provider-agnostic events; thinking content dropped at source.** — Prevents V0.5's adapter from being a refactor. Thinking content is noise the developer cannot act on; only markers cross the boundary. Tool input is end-only because per-character JSON is not useful UX.

- **Parallel tool calls within a step run concurrently.** — The model productively requests multiple tools per response (e.g. read two files at once); serial execution pays unnecessary latency. The TUI groups concurrent calls by kind so the developer can still follow. Edit's per-edit gate lives in aldwin-tools; the core just awaits.

- **Cancellation is a hard stop; partial output preserved as a well-formed log entry.** — Control must return immediately. Graceful wind-down was rejected because a stuck tool would make cancel meaningless. The cancelled turn closes with TurnEnded(cancelled); the log is never torn.

- **V0 prompt-cache breakpoints — two markers, static prefix and last completed turn.** — Marker one covers the system prompt + tool definitions (never changes). Marker two covers the last completed turn (stable for the cache TTL). Adaptive placement is not justified before V0.5 forces a second case.

- **Tool errors feed back to the model but surface visibly; LLM API errors retry transiently with every attempt visible.** — The model adapts to its own tool mistakes; the developer should not babysit recoverable failures. For upstream LLM failures (529, network drop) every retry emits RetryAttempt carrying provider, status, and verbatim message. Silent retries are rejected — failures must be attributable to their actual source.

- **Core owns the base system prompt; session initializer supplies an opaque additional-context string only.** — The base prompt is Aldwin's operating contract — structurally inseparable from the loop. The initializer cannot reorder or replace it; it can only append. Keeps composition out of the core while keeping the contract in.

## Pitfalls

- Anthropic wire types leaking past LlmClient — V0.5 becomes a refactor instead of an adapter swap.
- turn/step terminology drift in code and comments — vocabulary section is canonical; enforce by grep if needed.
- Blocking the loop on tool I/O instead of awaiting concurrently — parallel calls degenerate to serial.
- Cancellation leaving a torn record — the cancelled turn must end with TurnEnded(cancelled) and the partial assistant output must be a valid log entry.
- Cache breakpoint placement going stale if the turn boundary definition shifts later.
- Silent retries returning under the banner of UX cleanliness — every retry is visible with provider attribution.
- The base system prompt being bypassed or reordered "for testing" — testing scaffolds must respect the structural ordering.

## Out of Scope

- Tool implementations (Read, Diff, Explain, Edit, shell, MCP) — aldwin-tools.
- Permission engine, scope resolution, allowlist storage — aldwin-permissions.
- TUI rendering, web-client rendering, theming — aldwin-tui and the future web client.
- Anthropic HTTP, SSE parsing, request signing, retry backoff arithmetic — aldwin-llm.
- OpenAI-compatible adapter — aldwin-llm V0.5.
- MCP transport, server lifecycle, tool discovery — aldwin-tools via rmcp.
- Config file format, scope resolution, YAML schema — aldwin-config.
- Session persistence, history surface, developer-authored memory — out of V0 per parent.
- The textual content of the base system prompt — ownership is in scope; the prose is its own deliverable.
- Concrete semantics of the approval round-trip for Edit and other gated tools — aldwin-tools.

## References

- .claude/spec/aldwin.md — parent; narrow-vs-broad cut and inherited decisions.
- https://docs.anthropic.com/en/api/messages — Anthropic Messages API, streaming and tool-use blocks.
- https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching — breakpoint placement guidance.
- https://docs.anthropic.com/en/docs/build-with-claude/extended-thinking — thinking block semantics.
