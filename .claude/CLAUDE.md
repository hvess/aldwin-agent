# Mjolnir

## Project Overview

Mjolnir is a Rust TUI coding agent — a discussion-first harness where the developer's understanding is the product, not the agent's throughput. It is not a mobile SDK project. Do not apply mobile SDK, FFI, Android, or iOS framing here.

Workspace: seven Cargo crates under `crates/`. Specs for all seven live in `.claude/spec/`. Read the relevant spec before working on any crate.

## Language & Platform

All code is Rust. Idioms are Rust idioms — do not translate patterns from Kotlin, Swift, or other languages. The relevant references are the Rust Book, std docs, and crate documentation (ratatui, crossterm, reqwest, rmcp, serde).

## Spec Workflow

Specs are in `.claude/spec/` — read before implementing. As of 2026-08-29, four (config, core, llm, cli) are archived under `.claude/spec/archive/` — implemented, tested, and audited with no known gaps. The remaining four (permissions, tools, tui, screenshot) stay active, each with a dated Progress note on its known gaps; screenshot is the newest — it is built and usable — `crates/screenshot` plus the `screenshot` skill run a whole session (contract, preflight, capture, six gates, blind scoring, report); its known gaps are in the spec's Status and the skill's "What this does not cover". When a spec step is completed, note it; when all steps are done, move the spec to `.claude/spec/archive/`.

## Design System

The TUI's visual design is not invented locally — it is imported. A local
copy of everything below lives in `.claude/design/` (see its `IMPORT.md`);
read that first. Re-syncing from `claude.ai/design` has its own traps
(which of the two projects is live, and why the obvious lookups lie) —
the `design-sync` skill carries them; read it before any `DesignSync`
call. Re-sync only when you need something the local copy doesn't carry.

Two things about reading the handoff, each learned the hard way:

1. **Measure the handoff HTML; reading it is not enough.** Neither the token
   CSS nor the component prose states cell positions. They exist only as
   pixel values in the HTML's inline styles, and have to be divided by the
   cell size in `cells.css` (9×20px; the frame is 120×36 cells) to become
   grid coordinates. A design pass that skipped this step produced a layout
   that was wrong in every column while matching every colour exactly.
2. **Render it before trusting your reading of it.** Headless Chromium
   works, but under snap confinement it silently no-ops writes outside
   `/root` — copy the input there and write screenshots there too, or you
   get a reported success and no file.

The glyph vocabulary is fixed and closed: `▌ ● ◐ ○ ✔ ▶ █ + -`. If a mark is
needed and it is not in that table, do not draw one — `─` and the box-
drawing set are *not* in it. **One exception, ADR 0002:** a markdown table
in assistant prose is drawn with `┌ ┬ ┐ ├ ┼ ┤ └ ┴ ┘ ─ │`. It is scoped to
that one construct and is not a licence for a second stroked surface. The
Content Fundamentals hold too: third-person "The agent", lowercase labels,
sentence-case prose.

Two rules that now govern every layout decision (Turn 13):

- **Nothing inside a frame is stroked.** Every boundary is a step on the
  seven-rung ground ladder (`--color-ground-0…6`). No `Block::bordered()`,
  no rule rows, no underline attributes — a band is a rect with its own
  `Style::bg`. The rule governs boundaries between *regions*; a markdown
  table's are between *cells*, which is why ADR 0002 carves it out.
- **The grid is 3-cell margin, 8-cell label column, 2-cell gutter**, so
  body text lands on cell 13. There is deliberately no `--body-col` token;
  derive it, never restate it.

`.claude/spec/mjolnir-tui.md`'s Progress entries record what was measured
and what it corrected; read the 2026-09-06 entry before touching layout in
`crates/tui/src/ui/`.

## Decision records

`.claude/adr/` holds numbered architecture decision records for changes that
alter a stated constraint or a persisted format. Read them before reopening
a decision they cover.

- **0001 — Grants are per tool and per program, not per command string.**
  Tool classes pick the grant unit; `edit` is out of the permissions model
  entirely and stays a conscious diff. Also re-derives first run's access
  scale to three points.
- **0002 — A markdown table is drawn, and it is the only stroked thing in
  the frame.** Carves one exception out of Turn 13's no-stroke rule and the
  closed glyph table, on the grounds that a one-dimensional ground ladder
  cannot express a two-dimensional grid of cell boundaries. Leaves a debt:
  the upstream design system has no table component yet.

## Key Constraints (non-negotiable)

- Default-deny permissions: no tool may act without an explicit grant. No "obviously safe" carve-out.
- Edit is never allowlistable: friction on Edit is structural, not a setting.
- Discussion-first: resting state is conversation. Action only on explicit developer signal.
- No Anthropic wire types past `LlmClient`: audit at the trait boundary, not after.
