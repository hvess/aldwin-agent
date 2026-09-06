# mjolnir-llm

V0 Anthropic client implementing core's LlmClient trait — thin reqwest + SSE, no wire-type leakage.

**Status:** archived — implemented, tested, audited
**Scope:** mjolnir-llm crate only. HTTP, SSE, Anthropic-wire to normalised event mapping, wire-level retry, prompt-cache placement, provider config resolution. Excludes the LlmClient trait itself (core), the agent loop (core), tool execution (tools), and YAML I/O (config).
**Owner:** Maximilian
**Last Updated:** 2026-09-06

**Completed:** 2026-08-29 — `be6cd75`. Known, accepted (not a spec
deviation): `build_request` clones the full conversation history per turn,
O(n²) over a long session — real cost, unmeasured against actual usage,
documented in `wire.rs`'s own doc comment rather than fixed. Not yet
exercised against the live Anthropic API in this environment (no API key
available here) — unit- and integration-tested against a real local HTTP
server instead; a live run is still worth doing before fully trusting the
retry/SSE paths against the real API's exact behavior.

**Post-archive fix (2026-09-01, extended thinking rejected by a real
Anthropic key):** The live run flagged above surfaced exactly the gap it
warned about — the first real request against a real key (not the local
mock server every test here runs against) came back `provider error 400:
"thinking.type.enabled" is not support for this model. Use
"thinking.type.adaptive" and "output_config.effort" to control thinking
behavior`. The pre-4.6 request shape this crate sent, `thinking: {"type":
"enabled", "budget_tokens": N}`, is rejected outright on every current
Claude model (Sonnet 5, Opus 5, the rest of the 4.6+ family) — only
`{"type": "adaptive"}` is accepted, with `budget_tokens` removed entirely
rather than optional. Fixed in `wire.rs`: `WireThinking` now serialises to
`{"type": "adaptive"}` with no `budget_tokens` field; `max_tokens` still
derives from `extended_thinking_budget` (unchanged `provider.yaml` key —
no config-schema break) plus the existing headroom constant, since that
field's role was always "how much room to give this turn," not literally
"the value of a wire field" (`wire_openai.rs`'s OpenAI-compatible adapter
already used it the same way, unaffected by this fix — its wire shape
never had a `thinking` field at all). `output_config.effort` was not
added: omitting it defaults to `"high"`, equivalent to sending it
explicitly, and no config surface exists in this crate to drive a
different value — not introduced speculatively. Two new/updated
`wire::tests` cover it: `build_request_sends_adaptive_thinking_with_no_budget_tokens_field`
asserts the serialised request body is exactly `{"type": "adaptive"}`,
and `build_request_max_tokens_gives_headroom_above_the_thinking_budget`
(renamed from the old budget_tokens-comparing version) confirms
`max_tokens` sizing is unaffected. All 49 `mjolnir-llm` tests pass,
workspace build/test/clippy clean.

**Post-archive fix (2026-09-06, first live OpenAI-compatible endpoint —
Proton Lumo):** `OpenAiCompatibleClient` had only ever been run against
the local mock server and a Mistral probe, both of which put `usage` in
the same SSE chunk as `finish_reason`. Proton's Lumo
(`https://lumo-api.proton.me/ai/v1/chat/completions`, models `lumo-lite`
and `lumo-max`) sends it in a *trailing, choice-less* chunk after that
one, so the assembler's emit-StepEnded-at-finish_reason rule reported
`usage { 0, 0 }` for every turn on that backend — the client returns the
moment it sees StepEnded, so the usage chunk was never read. Fixed in
`wire_openai.rs`: `Assembler::end_step` emits StepEnded immediately when
usage is already known (Mistral's ordering, unchanged) and otherwise
holds the stop reason until a usage chunk arrives or the stream ends;
`Assembler::finish` flushes a held-back step with zero usage.
`client_openai.rs` calls `finish()` in the one arm every stream-ending
condition reaches (`[DONE]`, closed connection, idle timeout, framing
error), so a turn that completed but never reported usage yields
StepEnded instead of a retry or a `StreamInterrupted`. Accepted cost: on
a backend that sends `finish_reason` and then neither usage, `[DONE]`,
nor a close, StepEnded now waits out the 60s idle timeout rather than
firing instantly — no observed backend behaves that way. Second gap the same live run exposed: `lumo-max` streams its thinking as
`delta.reasoning` fragments interleaved with content, a field the adapter
had no place for, so a reasoning model ran silent — no thinking
indicator, just a pause. `WireDelta` now carries `reasoning`, and the
assembler brackets it: the first non-empty fragment emits ThinkingStart,
and the first text delta, tool call, or `finish_reason` after it emits
ThinkingEnd. The text itself is discarded, matching `wire.rs`'s existing
treatment of Anthropic's ThinkingDelta — core's vocabulary has
ThinkingStart/ThinkingEnd and no thinking-text event, and this fix does
not invent one. Also corrected the `base_url` comment in `annotated.rs`
(both provider constants): it is the full chat-completions URL, used
verbatim as the request endpoint, not a prefix the client appends a path
to. Coverage: four assembler tests (trailing-usage ordering for text and for
tool calls, the reasoning bracket, and a `finish_reason` closing an open
one), two client tests over captured Lumo frames, and
`crates/llm/tests/live_lumo.rs` — two
`#[ignore]`d live tests (text turn, tool turn) run with `LUMO_API_KEY`
set, retargetable at any OpenAI-compatible endpoint via `LUMO_BASE_URL` /
`LUMO_MODEL`. Both pass against the live API: text streams, usage lands
non-zero, and a `get_weather` tool call comes back parsed. All 55
`mjolnir-llm` tests plus the workspace suite pass, clippy clean. One
downstream fix this surfaced, recorded in `.claude/spec/mjolnir-tui.md`:
`App::thinking` was never cleared by `TurnEnded`, so a stream dying
mid-thinking left the spinner claiming the agent was still thinking.

## Why

Writing the Anthropic client by hand is what makes caching, streaming, and retry behaviour controllable rather than abstract. This crate owns the wire and translates Anthropic SSE into the core's normalised event stream. No Anthropic type crosses its public surface, so the V0.5 OpenAI-compatible adapter is a sibling impl behind the same trait, not a refactor.

## Vocabulary

- **Wire Event:** SSE from Anthropic's Messages API. Parsed internally; never crosses the trait boundary.
- **Breakpoint Marker:** Abstract pointer from mjolnir-core ("cache up to here"). Translated to a cache_control `{ type: ephemeral }` placement on a specific content block at request-build time.

## Design

- **HTTP:** Single shared reqwest::Client per session, rustls, HTTP/2 enabled, default pool.
- **SSE:** eventsource-stream over reqwest::Response::bytes_stream().
- **Wire Isolation:** All Anthropic wire types in a private `wire` module. Only AnthropicClient, ProviderConfig, and LlmError are pub.
- **Event Normalisation:** text_delta → TextDelta. Thinking blocks → ThinkingStart on content_block_start, ThinkingEnd on content_block_stop, delta content dropped at the parse site. Tool blocks → buffer input_json_delta chunks, emit one ToolUseRequested with the assembled object on content_block_stop. message_stop → StepEnded with stop reason, structured error if any, and Usage (input, output, cache create, cache read) folded from message_start + message_delta.
- **Cache Placement:** Two cache_control `{ type: ephemeral }` breakpoints per V0 request — final tool definition (covers static system + tools prefix) and final content block of the last completed turn. Anthropic permits four; V0 uses two.
- **Retry:** Retryable: 408, 429, 500, 502, 503, 504, 529, plus connect/read/write transport failures. Full-jitter exponential backoff (1s base, 30s cap), max 4 attempts. Every attempt emits RetryAttempt { provider: "anthropic", status, retry_in, message } with the verbatim upstream message. Mid-stream errors after the first event are not retried — the step ends with a structured error and partial output stays in the log.
- **Idle Timeout:** 60s SSE silence drops the stream and engages the retry path. Not user-tunable in V0.
- **Extended Thinking:** Enabled by default, always adaptive — `thinking: { type: "adaptive" }` (see the 2026-09-01 post-archive fix above; the pre-4.6 `{ type: "enabled", budget_tokens: N }` shape this line originally described is rejected on every current Claude model). `extended_thinking_budget` (resolved from provider.yaml) no longer names a literal request field; it sizes `max_tokens`' headroom above the response instead.
- **Provider Config Resolution:** Composes its own view from mjolnir-config's raw project_provider() and global_provider() snapshots — flat project-over-global overlay. Reads std::env::var(api_key_env) at construction; refuses to start on a missing var, surfacing the var name verbatim from the YAML.
- **API Version:** anthropic-version header pinned in code as a const. Provider config cannot override it.
- **Cancellation:** Dropping the returned event stream is sufficient — reqwest drops the connection, no detached tasks, no buffer survives the drop.

## Interfaces

- **Anthropic Client:** AnthropicClient implements mjolnir_core::LlmClient. Constructed from a resolved ProviderConfig.
- **Provider Config:** ProviderConfig { kind, model, api_key_env, base_url (V0.5), extended_thinking_budget }. Built via `resolve(project, global) -> Result<ProviderConfig, ConfigError>`.
- **Errors:** LlmError (transport, HTTP status, SSE parse, schema mismatch, retry exhausted, idle timeout, cancelled). Mapped to core's StepEnded.error variant at the trait boundary.

## Decisions

- **Anthropic wire types kept in a private `wire` module; only AnthropicClient, ProviderConfig, LlmError are pub.** — Structural enforcement of the parent's leakage pitfall — compiler-checked, not audited.

- **Single shared reqwest::Client per session (rustls, HTTP/2).** — reqwest::Client is cheap to clone but expensive to construct; per-call forfeits TLS resumption and HTTP/2 reuse.

- **SSE via eventsource-stream over reqwest::Response::bytes_stream().** — One HTTP path. eventsource-client owns its own lifecycle and would duplicate stacks.

- **Thinking content dropped at the parse site; only markers cross the boundary.** — Core spec mandate. Parse-site drop means no buffering, no accidental re-emission.

- **Tool input buffered into one end-only ToolUseRequested per call.** — Core's end-only contract; per-delta emission would push Anthropic's streaming shape upward.

- **Explicit retryable status set, full-jitter exponential backoff, max 4 attempts, every attempt visible.** — Silent retries violate core's provider-attribution stance. Full jitter mitigates correlated 529 storms.

- **Mid-stream errors after the first event do not retry.** — Re-running from scratch produces a different completion and a torn log of two attempts.

- **Two cache breakpoints — final tool definition, final block of last completed turn.** — Tools come last in the static portion, so the first marker caches system + tools; the second caches the last turn's accumulated state.

- **Extended thinking enabled by default; budget configured in provider.yaml.** — Thinking is what differentiates Claude on the kinds of questions this project is built around. Budget is the developer's concern, so it surfaces in YAML. *Adjusted, not reversed, 2026-09-01:* the wire request itself now always sends adaptive thinking (the only mode current models accept — see the post-archive fix above); the YAML budget's job narrowed from "the thinking budget" to "how much headroom `max_tokens` gets," but stays developer-configurable in the same place under the same name, so no provider.yaml written for this decision needs to change.

- **anthropic-version pinned in code, not config.** — The API version is Mjolnir's contract with Anthropic, not the developer's.

- **Provider config resolution lives in this crate — flat project-over-global field overlay.** — Only one provider is active at a time; field overlay is the right shape, not a name-keyed union.

## Pitfalls

- Re-exporting `wire::*` from lib.rs leaks Anthropic types into core's call sites.
- Buffering thinking content under a `debug` flag undermines the parse-site drop.
- One normalised event per input_json_delta chunk produces malformed partial JSON.
- No-jitter backoff on 529 amplifies correlated load spikes across clients.
- Transparent mid-stream resume splices two different completions into one log entry.
- Pinning anthropic-version and forgetting to bump it — audit on a recurring cadence.
- api_key_env typos must surface the typo'd name verbatim, not the canonical one.
- Idle timeout firing on heavy thinking budgets — scale with budget if 60s misfires; do not remove.
- Falling back to a community SDK mid-implementation — fix the SSE edge case instead.
- V0.5 — treating Qwen3-Coder as plain OpenAI-compatible JSON tool-call; their XML-ish format is measurably better.

## Out of Scope

- LlmClient trait definition — mjolnir-core.
- Agent loop, conversation log, turn/step bookkeeping — mjolnir-core.
- Tool execution and approval gate — mjolnir-tools.
- Permission engine — mjolnir-permissions.
- provider.yaml on-disk format — mjolnir-config.
- OpenAI-compatible adapter implementation — V0.5.
- Gemini, Bedrock, Vertex adapters — parent spec exclusion.
- LLM call audit log, persisted usage history — V0 surfaces usage on StepEnded but does not persist.
- Streaming resume after mid-stream failure — Anthropic publishes no resume capability.
- OS keychain for the API key — api_key_env is the only V0 surface.

## References

- .claude/spec/mjolnir.md — parent.
- .claude/spec/mjolnir-core.md — LlmClient trait, normalised events, cache markers, retry visibility.
- .claude/spec/mjolnir-config.md — provider.yaml shape, api_key_env indirection, raw per-layer snapshots.
- https://docs.anthropic.com/en/api/messages — Messages API.
- https://docs.anthropic.com/en/api/messages-streaming — SSE event shapes.
- https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching — cache_control placement.
- https://docs.anthropic.com/en/docs/build-with-claude/extended-thinking — thinking blocks, adaptive thinking, effort (current models no longer accept `budget_tokens` — see the 2026-09-01 post-archive fix above).
- https://docs.rs/reqwest/ — reqwest.
- https://docs.rs/eventsource-stream/ — SSE parser.
- https://qwenlm.github.io/blog/qwen3-coder/ — Qwen3-Coder tool-call format (V0.5).
- https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/ — full-jitter backoff.
