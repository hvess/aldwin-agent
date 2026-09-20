# Mjolnir

## Project Overview

Mjolnir is a Rust TUI coding agent — a discussion-first harness where the developer's understanding is the product, not the agent's throughput. It is not a mobile SDK project. Do not apply mobile SDK, FFI, Android, or iOS framing here.

Workspace: seven Cargo crates under `crates/`. Specs for all seven live in `.claude/spec/`. Read the relevant spec before working on any crate.

## Language & Platform

All code is Rust. Idioms are Rust idioms — do not translate patterns from Kotlin, Swift, or other languages. The relevant references are the Rust Book, std docs, and crate documentation (ratatui, crossterm, reqwest, rmcp, serde).

## Spec Workflow

Specs are in `.claude/spec/` — read before implementing. As of 2026-08-29,
four (config, core, llm, cli) are archived under `.claude/spec/archive/` —
implemented, tested, and audited with no known gaps. Four stay active:
permissions, tools, tui and review. When a spec step is completed, note it;
when all steps are done, move the spec to `.claude/spec/archive/`.

`mjolnir-review.md` is the feedback loop that runs after a change is ready
for submission — five stages, four of them deterministic and one a blind
subagent. `.claude/skills/review/SKILL.md` drives it; run `/review` when a
feature is finished. Read the spec's Progress entry before changing how any
stage works: it records what the loop replaced and why, and the failure it
replaced is easy to rebuild by accident.

`mjolnir-open-tasks.md` is a ledger rather than a spec: known, understood,
undone work, each entry citing its evidence.

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

**The design system reaches the app by generation, not by hand.**
`crates/tui/src/tokens.rs` is emitted from `.claude/design/tokens/*.css` by
`cargo run -p mjolnir-review -- tokens --write` and committed; the review
loop's stage 3 regenerates it and fails on any diff. Do not edit it, and do
not add a colour or a grid constant to the app by writing a literal — add it
to the design, re-sync, regenerate.

Where the reference disagrees with itself, the disagreement is recorded in
`crates/review/baseline.json` under `contradictions`, with both halves of
what the design says and which half the app follows. An entry leaves that
file when the design is fixed upstream. Keep it short — the list is a bug
list for the design system, and a previous version of this idea grew to
fourteen entries and became the problem it was built to solve.

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
- **0003 — A permission option is a sentence that states its own rule.**
  Adopts `5a`'s single-sentence row over the name + detail pair, and `5a`'s
  per-row scoping with it, which amends 0001 §3: there is no `Tab` scope
  toggle and no grant-summary row, because each row quotes the pattern it
  would write. Two stated costs — the session tier grants the exact target,
  and the panel no longer names the file a grant lands in.

## Key Constraints (non-negotiable)

- Default-deny permissions: no tool may act without an explicit grant. No "obviously safe" carve-out.
- Edit is never allowlistable: friction on Edit is structural, not a setting.
- Discussion-first: resting state is conversation. Action only on explicit developer signal.
- No Anthropic wire types past `LlmClient`: audit at the trait boundary, not after.
