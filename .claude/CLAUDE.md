# Aldwin

## Project Overview

Aldwin is a Rust TUI coding agent — a discussion-first harness where the developer's understanding is the product, not the agent's throughput. It is not a mobile SDK project. Do not apply mobile SDK, FFI, Android, or iOS framing here.

Workspace: seven Cargo crates under `crates/`. Specs for all seven live in `.claude/spec/`. Read the relevant spec before working on any crate.

## Language & Platform

All code is Rust. Idioms are Rust idioms — do not translate patterns from Kotlin, Swift, or other languages. The relevant references are the Rust Book, std docs, and crate documentation (ratatui, crossterm, reqwest, rmcp, serde).

## Spec Workflow

Specs are in `.claude/spec/` — read before implementing. Five are archived
under `.claude/spec/archive/` — config, core, llm and cli as of 2026-08-29,
and history as of 2026-09-20 — implemented, tested, and audited with no known
gaps. Four stay active: permissions, tools, tui and review. When a spec step is completed, note it;
when all steps are done, move the spec to `.claude/spec/archive/`.

`aldwin-review.md` is the feedback loop that runs after a change is ready
for submission — five stages, four of them deterministic and one a blind
subagent. `.claude/skills/review/SKILL.md` drives it; run `/review` when a
feature is finished. Read the spec's Progress entry before changing how any
stage works: it records what the loop replaced and why, and the failure it
replaced is easy to rebuild by accident.

`aldwin-open-tasks.md` is a ledger rather than a spec: known, understood,
undone work, each entry citing its evidence.

## Design System

The TUI's visual design is not invented locally — it is imported. A local
copy of everything below lives in `.claude/design/` (see its `IMPORT.md`);
read that first. Re-syncing from `claude.ai/design` has its own traps
(which of the two projects is live, and why the obvious lookups lie) —
the `design-sync` skill carries them; read it before any `DesignSync`
call. Re-sync only when you need something the local copy doesn't carry.

Two things about reading the handoff, each learned the hard way:

1. **Measure the handoff HTML; reading it is not enough.** The prose does not
   state cell positions and has repeatedly been wrong about them. In the
   current frame they are token references (`var(--label-col)`,
   `var(--row)`), so measuring is a lookup in `cells.css`; in the frames
   before it they were raw pixels needing division by the 9×20px cell (the
   frame is 120×36). A design pass that skipped this step produced a layout
   that was wrong in every column while matching every colour exactly.
2. **Render it before trusting your reading of it.** Headless Chromium
   works, but under snap confinement it silently no-ops writes outside
   `/root` — copy the input there and write screenshots there too, or you
   get a reported success and no file.

The glyph vocabulary is fixed and closed. **Do not quote it here** — it is
generated into `tokens::MARKS` from `HANDOFF.md`'s table and a copy in this
file would go stale, which it did: it read `▌ ● ◐ ○ ✔ ▶ █ + -` for a day
after the repaint made the prompt `▸` and a pass `✓`. If a mark is needed and
it is not in that table, do not draw one. **One exception, ADR 0002:** a
markdown table in assistant prose is drawn with `┌ ┬ ┐ ├ ┼ ┤ └ ┴ ┘ ─ │`,
scoped to that one construct and not a licence for a second stroked surface;
it is the only thing left in `MARKS_BY_EXCEPTION`.

Four of those marks are **statuses, and a status hue is spent nowhere else**:
`✓` ok, `✗` failed, `!` warned, `·` the info pointer. A diff sign is a status
too, which is why added shares the ok sage and removed the err rose. The one
brand colour — lantern gold — is spent on one thing per band: what is *open*,
selected, running, or being typed into. A settled `●` is a neutral. Reaching
for gold to mean "finished" is the habit this replaced.

The Content Fundamentals hold too: third-person "The agent", lowercase
labels, sentence-case prose. The one capitalised word is the brand `Aldwin`
in the top bar, which is a proper noun and not a label — the same word is the
speaker label `aldwin` two rows below it.

Two rules that now govern every layout decision (Turn 13):

- **Nothing inside a frame is stroked.** Every boundary is a step on the
  seven-rung ground ladder — `--color-ground-0` is the frame ground and the
  rungs run `up-1…3` / `down-1…3` from it. No `Block::bordered()`, no rule
  rows, no underline attributes — a band is a rect with its own `Style::bg`.
  The rule governs boundaries between *regions*; a markdown table's are
  between *cells*, which is why ADR 0002 carves it out. The steps are narrow
  on purpose (the tightest is 1.014:1), so "this looks low-contrast, nudge
  it" is undoing a decision rather than fixing an oversight.
- **The grid is 3-cell margin, 8-cell label column, 2-cell gutter**, so
  body text lands on cell 13. There is deliberately no `--body-col` token;
  derive it, never restate it.

**The design system reaches the app by generation, not by hand.**
`crates/tui/src/tokens.rs` is emitted from `.claude/design/tokens/*.css` by
`cargo run -p aldwin-review -- tokens --write` and committed; the review
loop's stage 3 regenerates it and fails on any diff. Do not edit it, and do
not add a colour or a grid constant to the app by writing a literal — add it
to the design, re-sync, regenerate.

Where the reference disagrees with itself, the disagreement is recorded in
`crates/review/baseline.json` under `contradictions`, with both halves of
what the design says and which half the app follows. An entry leaves that
file when the design is fixed upstream. Keep it short — the list is a bug
list for the design system, and a previous version of this idea grew to
fourteen entries and became the problem it was built to solve.

`.claude/spec/aldwin-tui.md`'s Progress entries record what was measured
and what it corrected; read the 2026-09-06 entry before touching layout in
`crates/tui/src/ui/`.

**Where the design disagrees with an ADR, the ADR wins and the disagreement
is recorded.** The 2026-09-21 repaint redrew the permission frame from a base
predating ADR 0004 — four options over a `cargo *` pattern, a "shell command"
sentence — and restored a fourth access rung reading "nothing asks". Neither
shipped; both are in `baseline.json`. A frame is authority on *tone and
position*, not on a permission model.

## Decision records

`.claude/adr/` holds numbered architecture decision records for changes that
alter a stated constraint or a persisted format. Read them before reopening
a decision they cover.

- **0001 — Grants are per tool and per program, not per command string.**
  *Superseded in full by 0004.* Tool classes pick the grant unit; `edit` is
  out of the permissions model entirely and stays a conscious diff. Kept for
  its reasoning about why a read/write axis looked unsound over a `shell` that
  took a whole command line — 0004 answers it by removing that tool.
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

- **0005 — A session outlives its process.** Reverses one clause of
  `aldwin.md`'s V0 "sessions are ephemeral" Decision: a conversation is
  written to disk as it happens and `/resume` picks one back up. The other two
  clauses are deliberately untouched — memory stays developer-authored and
  nothing crosses into a *new* session. Four boundaries keep it a persistence
  decision rather than a memory one; the fourth is that there is no
  `--resume` flag, so the zero-arg Decision stands.

- **0006 — Thinking is carried, not dropped.** Reverses `aldwin-llm.md`'s
  "thinking content is dropped at the parse site". The whole block crosses the
  boundary with its signature, `ContentBlock` and `LogRecord` gain variants for
  it (a persisted-format change), and blocks go back in the order they arrived —
  which the provider requires when that turn calls a tool. Carried is
  unconditional, *sent* is not: unsigned blocks and thinking-only turns are
  dropped at the Anthropic wire, and a cache breakpoint never lands on one. Fixes a turn
  that spent 14,096 tokens and rendered nothing. The TUI still does not draw
  it; that is open-tasks entry 24, not this ADR.

- **0007 — Reach is a workspace, and every tool honours it.** Amends 0004 §4
  and §5. `run` never called `paths.rs`, so §5's "no tool is pointed outside
  your project by us" was true of three tools and false of the one that
  executes programs — observed as `edit` refusing a sibling directory while
  `run` deleted two checkouts in it. One `Workspace`, a declared root list,
  `cwd` on `run`, output kept on timeout, reads enforced on macOS via Seatbelt,
  and §4's "every call asks" fallback finally built as written.

- **0008 — Discussion-first is about intent, not grammar.** Amends the
  non-negotiable above and `prompt::BASE`. "You act only on explicit
  instruction" made grammatical mood the trigger; the agent answered a stated
  constraint with the same two-option menu twice, produced nothing at all for a
  turn phrased as "what I am thinking is…", and argued the developer out of
  work it had just tested. Also states, because nothing did, that tool results
  are shown to the model and not to the developer.

- **0004 — A permission is a declared class, an enforced sandbox, and a lock.**
  Supersedes 0001 entirely and amends 0003's option list. There is no arbitrary
  command: a program runs only if a grant names it, and argv is executed
  directly rather than through a shell. A grant is a program and a class
  (`git: read`); the class belongs to the *call*, which is what makes a
  read/write axis sound where 0001 found it unsound. The agent declares the
  class and the sandbox enforces it — a read-declared call runs with the tree
  read-only and the network unreachable, so a wrong declaration costs a prompt
  rather than a tree. Deny is a lock nothing narrower can override.

## Key Constraints (non-negotiable)

- Default-deny permissions: no tool may act without an explicit grant. No "obviously safe" carve-out.
- Edit is never allowlistable: friction on Edit is structural, not a setting.
- No arbitrary commands: argv is executed directly, never through a shell, and a program runs only if a grant names it (ADR 0004).
- A read-declared call is enforced, not trusted: it runs where writing is impossible. Where it cannot be enforced, **every call asks** — it is never a flat error and never run unconfined (ADR 0004 §4, built in ADR 0007 §6).
- **Every tool honours the workspace boundary, `run` included** (ADR 0007). Reach is `roots[0]` plus whatever the project `permissions.yaml` declares; three tools out of four enforcing it is the bug that ADR records.
- Discussion-first: resting state is conversation. Action follows the developer's **intent**, not their grammatical mood — a stated constraint is an instruction, an agreed plan is carried out whole (ADR 0008). The structural protection is the diff gate and the permission model, never the phrasing rule.
- No Anthropic wire types past `LlmClient`: audit at the trait boundary, not after.
