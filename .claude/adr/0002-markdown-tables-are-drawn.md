# ADR 0002 — A markdown table is drawn, and it is the only stroked thing in the frame

**Status:** accepted, 2026-09-08
**Restated 2026-09-24** in the Aldwin Design System's tones, which replaced
Mjolnir's whole: the rules are `label3`, the header and the body `label`.
The decision is unchanged; `--tui-quiet`, `--tui-label` and `--tui-body`
below are the names it was made in.
**Supersedes:** nothing wholesale — it carves one exception out of the Turn 13
"nothing inside a frame is stroked" rule and out of the closed glyph table in
`.claude/design/HANDOFF.md`
**Affects:** `aldwin-tui` (`ui/markdown.rs`), `.claude/design/` (the glyph
table needs a table component upstream), `CLAUDE.md`'s Design System section

## Context

Assistant replies contain markdown tables and the TUI rendered them as literal
`|` pipes and a `---|---` delimiter row. Adding real support meant picking a
presentation, and the design system has no table component to import — so the
question was what its existing rules imply for one.

Two rules bear on it, both from the design system's Turn 13 rebuild:

- **`HANDOFF.md:244`** — "Content inside a frame is separated by **a full row
  of a different ground**, never by a rule."
- **`HANDOFF.md:230-243`** — the glyph table is closed: `▌ ● ◐ ○ ✔ ▶ █ + -`,
  and "if a mark is needed and it is not in that table, do not draw one." No
  box-drawing set appears in it.

The first implementation followed both literally: a header row toned as a field
name, one `break_` band beneath it, rows on the panel ground, column position
carrying the whole shape. It shipped, it was screenshotted, and it looked
plausible.

**It was rejected on sight by the developer**, twice, the second time after the
rules above were quoted back: *"I want a real table, it's the only thing that
makes sense here."*

## Decision

**A markdown table in assistant prose is drawn as a box:** `┌ ┬ ┐ ├ ┼ ┤ └ ┴ ┘
─ │`, one cell of padding either side of each cell's content, header row above
a `├─┼─┤` rule, closed top and bottom.

One bounded exception inside the exception: a column bottoms out at one cell, so
`n` columns need `3n + 1`. Below that — 15-odd columns on a narrow terminal —
rows are clipped to the column with the system's `…` and the right edge is lost.
Chosen over drawing a `┐` where the table does not end, and over silently
dropping the columns that do not fit: a visible `…` is the true statement.

Rules are `--tui-quiet`, the tier below `dim` — the structure is present without
competing with the cells for the eye. Cells keep their existing tones: header
`--tui-label` (the field-name tier), body `--tui-body`.

Nothing else changes. A turn break, a markdown `---`, first run's step
separators, the boundary between any two bands: all still bands, and
`render_snapshot.rs`'s box-drawing assertion still covers all eleven chrome
scenes.

## Why the rule does not reach this case

The honest version is that the developer asked for it and reaffirmed after the
constraint was put to them, which is sufficient. But there is also a real
distinction, and it is worth stating because it bounds the exception:

**The ground ladder is one-dimensional.** Every boundary the Turn 13 rule
governs separates one region from the next along a single axis — a bar from the
transcript, a turn from the turn before it, a quoted field from the prose around
it. A step in tone expresses that perfectly, and a drawn rule adds nothing.

**A table's boundaries are two-dimensional**: one per column, repeated down
every row, and they have to agree with each other. A ladder cannot express that
— there is no "step in tone" that runs vertically between column two and column
three of every row at once. The rule-less version proved it in practice: column
position alone held the shape only while every cell was populated and every
column was comfortably wide, and degraded into ragged prose the moment a cell
was empty or a neighbour was narrow.

So the exception is not "tables are special enough to break the rule." It is
that the rule is a statement about *region* boundaries, and a table's are
*cell* boundaries, which the design system had simply never had to have an
opinion about.

## Consequences

- **The glyph table upstream is now wrong**, and this is the debt this ADR
  incurs. `.claude/design/` is a local copy of an imported system; the source
  of truth is the two projects on `claude.ai/design`, and neither has a table
  component or the box-drawing glyphs in its Iconography table. Until it is
  added there, this crate draws a mark the design system does not list.
  Recorded here rather than silently diverging.
- `render_snapshot.rs`'s "no box-drawing glyph anywhere" assertion is still
  correct for every scene it covers. Its doc comment now names this exception,
  so a future table scene fails loudly with the reason rather than confusingly.
- The exception is **scoped to a markdown table in assistant prose**. It is not
  a general licence: `Block::bordered()` stays out, the inline diff and code
  fence stay recessed fields with no outline (that was a Turn 13 fix for a real
  class of bug — see `row.rs:63-75`), and a new stroked surface needs its own
  argument, not a citation of this one.

## Alternatives considered

- **Keep the band version.** Rejected by the developer, twice. It is what the
  rules say and it is not what a table needs.
- **Rules only between columns, no top/bottom/outer box.** Half a table: the
  vertical rules have nothing to terminate against, so the block has no top or
  bottom edge and reads as prose with pipes in it.
- **Take it to the design system first, implement after.** The right order in
  principle, and it is still the follow-up. It was not worth blocking a
  developer-requested render on a round trip through two design projects, one
  of which is already partly stale (`IMPORT.md`).

## Follow-up

Add a table component and the box-drawing glyphs to the source design system
(`claude.ai/design`, project `4ea574fb-…`), then re-sync `.claude/design/` and
delete the "the glyph table upstream is now wrong" consequence above. Until
then, `IMPORT.md`'s glyph vocabulary section and this ADR disagree, and this
ADR is the one that describes the code.
