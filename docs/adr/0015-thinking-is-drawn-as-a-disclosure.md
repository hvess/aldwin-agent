# ADR 0015 — Thinking is drawn as a disclosure

**Status:** accepted, 2026-09-29. §1's "each thinking block is a
`Disclosure` where it happened" is superseded by 0018, which folds a turn's
thoughts into its one work row; the rest stands.
**Amends:** ADR 0006's Consequences, "The TUI still does not draw thinking";
`aldwin-core`'s `LogRecord::Thinking`, a persisted format (ADR 0005), and
`Event::ThinkingEnd`
**Affects:** `aldwin-core`, `aldwin-tui`, `aldwin-review` (a scene)

## Context

Since ADR 0006 the model's thinking is carried to the wire and saved in the
transcript, but the TUI read it only to say `Thinking` on the working line.
A turn that thought for a long time showed none of what it thought, and a
finished turn kept no trace of it on screen. ADR 0006 left it undrawn
because the design system has no treatment for reasoning and one is not
invented locally.

The design does have one for technical detail: "Technical detail sits one
disclosure below, exact and unabridged" (README, Content fundamentals),
drawn as the `Disclosure` component — `Read 1 file  ›`, which Space opens.
The developer's call, 2026-09-29: that is the treatment.

## Decision

1. **Each thinking block is a `Disclosure` where it happened** in the
   conversation, before the prose or work that followed it. Its summary is
   `Thinking` while the block streams, then `Thought for 12s` (`2m 05s`
   past a minute); in `label2`, as the work summary is. Space opens it with
   the turn's other disclosures, and its detail is the reasoning itself,
   unabridged, each line wrapped on the prose column in `label2`.
2. **A block's time is measured by core and saved with it.** Core times
   each block from `ThinkingStart` to its end, in whole seconds rounded up
   (at least 1), and puts the same number in `Event::ThinkingEnd` and
   `LogRecord::Thinking`'s new `seconds`. The live line and a resumed one
   read alike, and the TUI keeps no clock of its own for it.
3. **The field is optional.** A transcript written before this has no
   `seconds`; its thoughts are drawn as `Thought`. A turn stopped mid-thought
   leaves `Thought` too, never a `Thinking` that is no longer running.
4. **Redacted thinking is not drawn**: it is encrypted, so there is nothing
   to disclose.

## Consequences

The design system still draws no reasoning; `crates/review/baseline.json`
records the difference (`no-thinking-component-adr-0015`) until it does.

The transcript's `thinking` record gains one optional number. Older
builds ignore it on load, since serde skips unknown fields by default.

A long reasoning block opened while it streams is wrapped again on every
delta, as a streaming reply already is. The transcript cache
(`ui::Transcript`) re-renders only that entry.
