# Aldwin

## Project Overview

Aldwin is a Rust TUI coding agent — a discussion-first harness where the developer's understanding is the product, not the agent's throughput. It is not a mobile SDK project. Do not apply mobile SDK, FFI, Android, or iOS framing here.

Workspace: eight Cargo crates under `crates/` — cli, config, core, llm, login, review, tools and tui. Each has a spec in `.claude/spec/` or its `archive/`. Read the relevant spec before working on any crate.

## Language & Platform

All code is Rust. Idioms are Rust idioms — do not translate patterns from Kotlin, Swift, or other languages. The relevant references are the Rust Book, std docs, and crate documentation (ratatui, crossterm, reqwest, rmcp, serde).

## Spec Workflow

Specs are in `.claude/spec/` — read before implementing. Six are archived
under `.claude/spec/archive/`: config, core, llm and cli as of 2026-08-29,
the transcript feature (history) as of 2026-09-20 — implemented, tested, and
audited with no known gaps — and permissions as of 2026-09-24, closed when
ADR 0011 deleted its crate. Four stay active: tools, tui, review and login. When a spec step is completed, note it; when all steps are done, move the spec to
`.claude/spec/archive/`.

`aldwin-review.md` is the feedback loop every agent commit runs — ten
stages: five deterministic, three blind subagent judges (code, Rust,
frames), the iteration, and a gate. `.claude/skills/review/SKILL.md` drives
it; run `/review` before every commit. **An agent's commit cannot land
without it**: the pre-commit hook refuses any tree without a passing review,
and a `PreToolUse` guard refuses the ways around the hook. If the gate
refuses, run the loop — never look for another way to commit. Read the
spec's Progress entries before changing how any stage works: they record
what the loop replaced and why, and the failure it replaced is easy to
rebuild by accident.

`aldwin-open-tasks.md` is a ledger rather than a spec: known, understood,
undone work, each entry citing its evidence.

## Design System

The TUI's visual design is not invented locally — it is imported. A local
copy of everything below lives in `.claude/design/` (see its `IMPORT.md`);
read that first. Re-syncing from `claude.ai/design` has its own traps
(which of the two projects holds what, and why the obvious lookups lie) —
the `design-sync` skill carries them; read it before any `DesignSync`
call. Re-sync only when you need something the local copy doesn't carry.

**The design is the Aldwin Design System (project `b9de8837-…`), imported
2026-09-23.** It replaced the Mjolnir system whole — "None of Mjolnir's
values carry over" — and it changed behaviour, not only paint: reads and
runs need no permission, an edit is a staged change reviewed in a
full-window review, and the plan and the agent's questions are drawn as
their own components. ADR 0009 is the record of what that meant for the
product; read it before reopening any of it.

Two things about reading the design, each learned the hard way:

1. **Measure the frame; reading the prose is not enough.** Nearly every
   position in `frames/Aldwin Agent TUI.dc.html` is a `var(--…)` from
   `tokens/layout.css`, so measuring is a token lookup (frame E's panel
   insets are bare `ch`; baseline records it) — but the frame also
   holds things the prose never states: the brand mark is 108 half-block
   cells that exist nowhere else, and the context bar's ramp is arithmetic
   in `ContextBar.jsx`. A design pass that read the README alone would have
   drawn the mark as a glyph.
2. **Render it before trusting your reading of it.** Firefox headless works
   on this machine; build a standalone page by inlining the token CSS, since
   the frame links `styles.css` relatively and `<x-dc>` is the host's
   templating. Rendering is what showed the plan's running dot is amber and
   the footer's status word sits on the body column.

The glyph vocabulary is fixed and closed. **Do not quote it here** — it is
generated into `tokens::MARKS` from `guidelines/glyphs.html` and the frame,
and a copy in this file would go stale. If a mark is needed and it is not in
that table, do not draw one. **One exception, ADR 0002:** a markdown table
in assistant prose is drawn with `┌ ┬ ┐ ├ ┼ ┤ └ ┴ ┘ ─ │`, scoped to that one
construct; it is the only thing in `MARKS_BY_EXCEPTION`.

Four colour rules, each the design's own sentence:

- **Blue means you.** The one accent is spent on the developer: the `›` of
  the prompt and the current row, the `▎` of a selection, a `◆` comment, a
  `✓` on a step done for them, and the key glyph of the action that is
  ready. The agent's prose is never blue.
- **Amber means running**, and nothing else is amber.
- **Green and red appear only in diffs.** A failure is a sentence in
  `label`, not a red row (ADR 0009 §5). There is no `✗` and no `!`.
- **Three text tones**: `label` for what is current, `label2` for what is
  said around it, `label3` for what is pending or structural.

The Content Fundamentals hold too: lead with a sentence, outcomes not tool
names, sentence case, "you" for the developer and never "we", no
exclamation marks. The one capitalised word in a window is the brand
`Aldwin` in the launch card.

Two rules that govern every layout decision:

- **Nothing inside a window is stroked.** A band is a rect with its own
  `Style::bg` — `win`, `tint`, `panel`, `field`, `select` — and the step
  between two grounds is the boundary. No `Block::bordered()`, no rule rows,
  no underline attributes. The light theme inverts the ladder, so a "raised"
  band is a step in either direction, never a lighter one by assumption.
- **The grid is a 3-cell margin and a 2-cell mark column, so prose lands
  on cell 5.** `--body-x` is declared *and* checked against the sum
  (`grid.rs`); the label column of the previous system is gone — the echoed
  prompt sits on its own ground with a `›`, and the agent's prose needs no
  name.

**The design system reaches the app by generation, not by hand.**
`crates/tui/src/tokens.rs` is emitted from `.claude/design/` by
`cargo run -p aldwin-review -- tokens --write` and committed; the review
loop's stage 4 regenerates it and fails on any diff. It carries the palettes
(OKLCH converted to sRGB by the generator), the mark's cells, the gauge's
ramp table, the grid and the glyphs. Do not edit it, and do not add a colour
or a grid constant to the app by writing a literal — add it to the design,
re-sync, regenerate.

Where the design disagrees with the product, the disagreement is recorded
in `crates/review/baseline.json` under `contradictions`, with both halves
and which the app follows. Keep it short — the list is a bug list for the
design system, and a previous version of this idea grew to fourteen entries
and became the problem it was built to solve.

**Where the design disagrees with a decision, the decision wins and the
disagreement is recorded.** The design's commands include `/changes` and
`/undo`; the product ships `/resume`, `/model`, `/quit`, `/clear` (the
developer's call, 2026-09-23), `/theme` (2026-09-25) and `/connect` (ADR
0012, 2026-09-26); `/changes` and `/undo` will not be built (the
developer's call, 2026-09-27). That is in `baseline.json`. A frame is authority
on tone and position, not on scope.

## Decision records

`.claude/adr/` holds numbered architecture decision records for changes that
alter a stated constraint or a persisted format. Read them before reopening
a decision they cover.

- **0001 — Grants are per tool and per program, not per command string.**
  *Superseded in full by 0004, which 0009 in turn supersedes.* Kept for its
  reasoning about the read/write axis.
- **0002 — A markdown table is drawn, and it is the only stroked thing in
  the frame.** Carves one exception out of the no-stroke rule and the closed
  glyph table. Still in force: the new design system has no table component
  either.
- **0003 — A permission option is a sentence that states its own rule.**
  *Superseded by 0009*: there is no permission option.
- **0004 — A permission is a declared class, an enforced sandbox, and a lock.**
  *Superseded by 0009 and 0011* except §5 (reach), which stands as 0007 and
  0011 widened it. Kept for why argv and the lock were tried.
- **0005 — A session outlives its process.** Unchanged: a conversation is
  written to disk as it happens and `/resume` picks one back up.
- **0006 — Thinking is carried, not dropped.** Unchanged.
- **0007 — Reach is a workspace, and every tool honours it.** In force, and
  since 0011 the whole rule; only its containment of `run`'s arguments is
  superseded. `roots:` in `permissions.yaml` is the one way to widen the
  workspace.
- **0008 — Discussion-first is about intent, not grammar.** Unchanged.
- **0009 — The review is the only gate.** *§1–§3 superseded by 0011.* Reads
  and runs need no grant and never ask. An edit is staged, every edit of a
  turn is one changeset, and the review opens at the first moment the
  changeset would be observed on disk — before a `run`, or at the turn's
  end. Nothing Aldwin writes lands before an approve. There is no first
  run. `plan` and `ask` carry the plan and a question to the screen. A
  failure is a sentence.
- **0010 — Review lines are selected with the mouse.** The diff has no line
  cursor; a click selects a line, a drag selects a run, and `Shift ↑↓`
  selects from the keyboard. The mouse is captured only while a review is
  open, so the conversation keeps the terminal's own text selection.
- **0011 — The workspace is the only boundary.** There is no class, no
  argv rule and no `deny:` lock. `run` takes a shell command. Every process
  Aldwin starts — a run, the language server, an MCP server — runs in one
  sandbox that can write only inside the workspace roots and a short
  incidental list; reads and the network are open, the network by stated
  non-goal. Where the system cannot confine, the developer is told once at
  startup. `allow:`, `default:` and `deny:` still parse, and are reported.
  *An MCP server gets no workspace root: 0014.*
- **0012 — A connected account is tried before an API key.** `/connect`
  connects an account (xai today) through the device-code flow; its tokens
  live in the global-only `connections.yaml`. `provider.yaml` is unchanged:
  a provider that offers an account is reached through it when connected,
  through its key when exported, and otherwise every request answers with
  one sentence naming both fixes. The sign-in is the `aldwin-login` leaf
  crate.
- **0013 — Aldwin is a co-author of the commits it makes.** Every `git
  commit` from anything Aldwin starts carries `Co-Authored-By: Aldwin
  <noreply@aldwin.codes>`. Aldwin's own binary is a git shim: at startup a
  0700 directory holding a `git` symlink to it goes first on the process's
  `PATH`, and started as `git` it runs the real one with `--trailer` added
  after `commit`; git's own duplicate check keeps it to one. Merges,
  cherry-picks, rebases, `commit-tree`, aliases and an absolute-path git get
  no trailer. A shim that could not be installed is said once at startup.
- **0014 — An MCP server cannot write the workspace.** Every MCP stdio
  server starts in the sandbox with no workspace root, so it reads the tree
  and writes only the incidental paths; a write into the workspace is the
  kernel's refusal, and the model uses `edit` instead. It closes ADR 0009
  §4's one exception wherever the sandbox confines; where nothing can be
  confined 0011 §3 holds, and a workspace under an incidental path stays
  writable.

## Key Constraints (non-negotiable)

- Nothing reaches disk without the review: `edit` stages, and only an approve at the review writes (ADR 0009 §4). There is no approve for one call; the changeset is reviewed whole. An MCP server is given no write access to the workspace (ADR 0014; its Limits say where that cannot hold).
- Edit is never allowlistable: there is nothing to allowlist it into. The review is structural, not a setting.
- **The workspace is the only boundary** (ADR 0007, ADR 0011). It is `roots[0]` plus whatever the project `permissions.yaml` declares. Every tool refuses a path outside it, symlinks included, and an approved write is resolved again before it lands.
- Every process Aldwin starts — `run`'s shell, the language server, an MCP server — can write only inside the workspace and the incidental paths (an MCP server only the incidental paths, ADR 0014), enforced by the kernel (Landlock, Seatbelt), not trusted. Where it cannot be enforced, it runs unconfined and **the developer is told once** — never silently (ADR 0011 §3).
- Discussion-first: resting state is conversation. Action follows the developer's **intent**, not their grammatical mood — a stated constraint is an instruction, an agreed plan is carried out whole (ADR 0008). The structural protection is the review, never the phrasing rule.
- No Anthropic wire types past `LlmClient`: audit at the trait boundary, not after.
