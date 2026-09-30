# ADR 0018 — A turn's work is one row

**Status:** accepted, 2026-09-30
**Supersedes:** ADR 0015 §1's "each thinking block is a `Disclosure` where
it happened"; the rest of 0015 stands
**Affects:** `aldwin-tui` (`LogEntry::Work`, `Act`, the `thinking` snapshot scene)

## Context

A turn drew a row for every thing it did. Each thinking block was its own
disclosure (ADR 0015), and a `Work` row held only the calls of one step, so
a thought or a sentence of prose between two steps started another. A turn
that thought, read, thought and ran drew four one-line rows, a blank row
between each. The developer's report, 2026-09-30: "it adds a lot of spam
and I think this can simply be contained in one single row."

The design already draws it that way. Frames E and F show a turn as its
prose and one disclosure, `Read 3 files · Ran 6 tests, all passed · Saved
3 files  ›`. The design draws no reasoning (`baseline.json`,
`no-thinking-component-adr-0015`), so where a thought goes is ours to say.

## Decision

1. **A turn's thoughts and calls are one `Disclosure`**, a single
   `LogEntry::Work` holding `Act`s in the order they happened. It sits where
   the turn's first thought or call happened, and every later one joins it,
   across prose, a plan, a question or a review.
2. **Its summary puts the thinking first**, the thoughts' seconds added up
   (`Thought for 6s · Read 3 files · Ran 1 command`), `Thinking` while one
   streams, `Thought` when none has a time. The calls are counted by verb,
   as before.
3. **Opened, each act is a `DetailRow`**: a call as before; a thought as
   `Thought` with its time right-flush (`…` while it streams) and the
   reasoning under it, unabridged, in `label2` on the prose column.
4. **Prose is unchanged.** The agent's sentences stay where they were said,
   and prose on either side of an act stays two paragraphs even though no
   row now sits between them.

## Consequences

The transcript format is unchanged: a resumed turn folds its records into
the same one row, and each turn keeps its own.

With the disclosure open, a streaming thought re-renders the acts above it
on every delta, not only its own text, since they are one entry: twice, for
the new head and for the cached one the transcript checks it against.
Opened mid-turn this costs two wraps of the turn's earlier reasoning per
delta; the disclosure is closed by default, and a closed one is a single
row. Open or closed, the transcript cache clones and compares the whole
entry per delta, so a copy of the turn's reasoning rather than of one
thought: a memory copy of a few kilobytes, well under a frame.
