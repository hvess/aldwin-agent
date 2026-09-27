# ADR 0006 — Thinking is carried, not dropped

**Status:** accepted, 2026-09-21
**Amends:** the archived `aldwin-llm.md`'s "thinking content is dropped at the
parse site"; `aldwin-core`'s `ContentBlock` and `LogRecord`, which is a
persisted format (ADR 0005)
**Affects:** `aldwin-llm`, `aldwin-core`, `aldwin-tui`

## Context

Extended thinking is on by default (`WireThinking::adaptive()`), and the wire
layer threw the result away. `wire.rs` said so in as many words — *"thinking
content is dropped at the parse site (only start/end markers cross the
boundary)"* — and `ContentBlock` had no variant it could have been kept in if
it had crossed.

The reasoning was that thinking is not prose and the TUI has nothing to draw
it with. That is true and is not the part that went wrong. Two things follow
from dropping it that nobody costed:

**1. A reasoning-only turn renders as nothing.** In a transcript reviewed on
2026-09-21, turn 5 of an 18-turn session reported `stop_reason: EndTurn` with
`output_tokens: 14096`, zero `assistant_message` records and zero tool calls.
The developer saw an empty response and typed "Continue". Fourteen thousand
tokens of work existed, were paid for, and reached nobody. The transcript
format has no thinking event at all, so it was not even recoverable
afterwards.

**2. The next request is invalid.** When a turn that produced thinking goes on
to call a tool, the provider requires the thinking block back — with its
signature — on the assistant message that requested the call. We were
reconstructing that message from text and tool-use blocks alone. This has not
been the cause of a reported failure, which is luck rather than design: it is
a 400 waiting on the right combination of adaptive thinking and a multi-step
turn.

## Decision

### 1. The whole block crosses the boundary

`LlmEvent::ThinkingEnd` carries `{ text, signature }` rather than being a unit
marker, and `ThinkingDelta { text }` streams the fragments. The assembler
buffers thinking exactly as it already buffered tool input, and closes it out
on `content_block_stop`. `redacted_thinking` — which arrives whole rather than
in deltas — is carried opaquely as `RedactedThinking { data }`.

### 2. `ContentBlock` gains `Thinking` and `RedactedThinking`

Blocks are kept **in the order they arrived**. The provider wants a turn back
as it wrote it, and with interleaved thinking that can be thinking, text,
thinking again — so text is flushed into the message whenever a thinking block
interrupts it, live and in the log alike, rather than thinking being sorted to
the front. (The first version of this ADR sorted it; an audit the same day
caught that this hands back a turn the provider never emitted.)
`map_content_block` serialises both kinds, signature included.

**The signature is never rendered and never regenerated.** It is the
provider's stamp over the block; our only correct relationship with it is to
carry it unchanged.

### 3. It is persisted

`LogRecord::Thinking` and `LogRecord::RedactedThinking`, appended as each
block closes rather than at the end of the step — so a crash mid-step cannot
lose a block that the provider will later demand back. `messages_from_log`
replays them in log order, which is arrival order by construction.

This changes a persisted format, which is why this is an ADR. It is a forward
change only: a transcript written before today has no thinking records, and
replays exactly as it did.

### 4. An OpenAI-compatible provider carries it but does not echo it

`delta.reasoning` is accumulated and closed out the same way, with an **empty
signature** — that wire issues none, and an invented one would be a lie in a
field whose whole purpose is provenance. On the way out, `map_message_into`
drops thinking blocks: there is no assistant-side reasoning block to put one
in, and sending one would be a 400. Which providers can accept thinking back
is a wire question, not a history one.

### 5. Carried is unconditional; sent is not

History keeps every block. The Anthropic wire drops two cases on the way out,
because sending them fails the request:

- **A block with no signature.** `/model` can move a running session from an
  OpenAI-compatible provider onto Anthropic; the signature is how Anthropic
  verifies a block is its own, and an empty one is a guaranteed rejection.
- **Thinking with nothing after it.** The requirement to echo thinking exists
  for turns that went on to call a tool. A turn that *only* thought — the
  14,096-token case in Context — maps to an empty message and is removed; two
  user messages in a row are accepted. Without this, the very turn this ADR
  was written for would have produced an assistant message of nothing but
  thinking.

And the cache breakpoint never lands on a thinking block, which the provider
refuses: it goes on the last block that can carry one, in the nearest message
at or before the requested index. Moving it earlier only shortens the cached
prefix.

### 6. A turn that produces nothing says so

Independently of thinking: a step that ends the turn having produced no text
and no tool call now emits a `Notice`. Carrying thinking fixes the observed
blank turn, but it does not make a blank turn impossible, and "the agent ended the
turn without a reply" is always better than a frame that
looks like a hang.

## Consequences

**The TUI still does not draw thinking, and this ADR does not make it.**
Reasoning is carried for the wire and the transcript; the log shows what the
agent said, not what it thought. Drawing it needs a treatment the design
system does not specify, and it is not invented locally; it is to be drawn
once the design does (open-tasks 3).

**Transcripts get larger**, by roughly the thinking budget per step. They are
already `0600` under `~/.aldwin/history/` and already carry tool output; this
does not change their sensitivity class, but it does mean reasoning about
repository content is now on disk; transcripts are always written and never
pruned, by decision (ADR 0005).

**`ThinkingEnd` is no longer a unit variant.** Every match on it needed
updating; two tests that pinned the old drop behaviour were rewritten to pin
the new contract rather than deleted.

## Alternatives rejected

- **Keep dropping it, and fix only the blank turn with a notice.** Cheaper,
  and leaves the provider-correctness bug in place for a future 400 nobody
  will connect to this.
- **Carry it in memory but not on disk.** The wire requirement outlives the
  process: a resumed session that calls a tool has to produce the thinking
  that preceded it, and `/resume` rebuilds from the log.
- **Render thinking in the transcript as dimmed prose.** Plausible, and a
  design decision rather than a wire one. It belongs in the design system,
  not in an ADR about the parse site.
- **Fabricate a signature for OpenAI-compatible reasoning** so both wires
  look alike. A provenance field we invent values for is worse than an empty
  one that is honestly empty.
