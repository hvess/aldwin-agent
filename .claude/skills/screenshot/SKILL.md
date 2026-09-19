---
name: screenshot
description: Capture and score Mjolnir's TUI the way a real terminal renders it — the shipped binary in headless foot, three fixed sizes in both themes, scored against the design system by a judge that never sees the code. Use when a change touches crates/tui, or to check a UI change against .claude/design/ before calling it done. Not for functional testing.
---

# Screenshot session

`.claude/spec/mjolnir-screenshot.md` is the spec; read it before changing how
any of this works. This file is how to *run* a session.

The harness is `crates/screenshot` (`mjolnir-screenshot`). It runs the shipped
binary inside a headless compositor, so nothing appears on the developer's
desktop and nothing of theirs leaks into a frame.

## 1. Open the session

Nothing runs until the goal, the focus set and the scenes are written down,
because everything downstream is judged against them. If the developer has not
given you these, ask.

```sh
mjolnir-screenshot start \
  --goal  "Multi-line support in the composer: the field meets the multi-line design, regresses nothing else, survives unexpected input" \
  --focus "conversation,approval:10-14"
```

That prints a run directory and writes `session.json` into it. Every command
after this takes `--run <dir>`. Scenes default to the ones the focus set
names; `--scenes` overrides. `mjolnir-screenshot scenes` lists the catalogue.

## 2. Preflight — one pass, and a failure ends the session

```sh
mjolnir-screenshot preflight --run <dir>
```

It checks the binary exists, that `render_snapshot` is green (a stale snapshot
makes the regression gate compare against fiction and report *clean*), and
that the measured cell still matches the baseline. A failure stops the
session; do not work around one.

## 3. Capture and gate

```sh
mjolnir-screenshot run --run <dir>
```

Every scene the session names, at all three sizes in both themes — six frames
each. Each frame leaves the PNG a human reads, the declared cell grid the
gates read, the inferred region map, and its gate results. Frames with
violations also get a `.marked.png` with each one outlined in magenta.

The six gates are deterministic and zero-tolerance. They read the **declared
cells** — what the app said it was drawing — never the picture:

| gate | checks |
| --- | --- |
| colour | every declared colour is a role in that theme's palette, or a role dimmed toward a ground |
| breakages | unpainted cells, glyphs outside the design's table, a wide glyph clipped at the row edge |
| layout | nothing but a band's ground in the 3-cell margin |
| role pairing | no band painted in an ink role, no glyph painted in a ground rung |
| content | first person in prose — the agent is written about in the third person |
| regression | every `render.snap` region the diff touches falls inside the focus set |

A gate failing does not stop the run; it blocks the exit. `run` also reports
the regression gate at the end, once per iteration rather than per frame.

## 4. Score — with a judge that cannot see the code

Spawn a **separate agent**. It gets the frames, the criteria, the focus set
and the design reference. It does **not** get the diff, the source, or its own
earlier scores, and you do not tell it what you changed or hope it will say.

An agent that both fixes and judges converges on its own scorer; one that
knows the intent behind a change is biased toward seeing that intent met; one
that remembers its last score anchors on it. If you find yourself wanting to
give the judge "just a bit of context so its feedback is actionable", that is
the defence being traded away.

It scores two things per frame, 0–100:

- **spatial** — rows and columns where the design puts them. The grid is
  `.claude/design/tokens/cells.css`: margin 3, label column 8, gutter 2, so
  body lands on cell 13. Derive it; there is deliberately no `--body-col`.
- **component fidelity** — what is in focus matches its referenced design.

Write them into `session.json` as `scores` entries (`scene`, `size`, `theme`,
`spatial`, `component`, optional `note`). The threshold is **90, taken as the
minimum across every frame**, never the mean.

## 5. Loop

Iterate capture → score → fix, at most **five times**, recording what you
changed each pass in `session.json`'s `iterations`. Ask the crate whether you
may stop rather than deciding yourself:

```sh
mjolnir-screenshot verdict --run <dir>     # non-zero while the loop must continue
```

    exit = preflight clean ∧ no gate violation ∧ min score ≥ 90 ∧ iterations ≤ 5

Two things the loop may never do:

- **Edit outside the focus set to move the number.** That is the failure the
  focus set exists to catch, and the regression gate is what catches it.
- **Widen an exemption to make a gate pass.** `crates/screenshot/baseline.json`
  holds deviations that are correct *by decision*, each citing the ADR or spec
  entry that made it one. Adding an entry is a conversation with the
  developer, not a step in the loop.

If a rendered component has no counterpart in the design system, **stop and
ask**. That is design debt needing a deliberate decision — ADR 0002 is what
handling one looks like — not something to invent a fix for.

## 6. Report

```sh
mjolnir-screenshot report --run <dir>
```

One self-contained HTML file, frames embedded, so it outlives the run
directory. It opens with the verdict — computed from the gates, the scores and
the iteration count, not written by you — then preflight, gates, score,
frames, and last your own account of what changed each iteration.

Older runs look after themselves: `start` keeps the last five and prunes the
rest, and `mjolnir-screenshot clean --keep N` sweeps on demand. What it will
not do is decide about *this* run.

Then **ask before deleting the run directory**. Acceptance is the developer's,
after reading the report. A 90 nobody has looked at is not an accepted 90, and
a failed run keeps its directory as evidence for the next attempt.

## What this does not cover

Be straight about these in any report rather than implying coverage the
harness does not have.

- **`breakages` is partial.** Unpainted cells, glyphs outside the design's
  table, a clipped wide glyph, and — through `layout` — content pushed into
  the margin. What it still cannot see is two runs colliding *inside* the body
  column, or text truncated with a well-formed ellipsis. Both are this UI's
  recurring defect, so a clean `breakages` is not proof of either.
- **`role pairing` is deliberately weak.** It catches a band painted in an ink
  role and a glyph painted in a ground rung. It does not check that a *label*
  is `--tui-label`, because the design does not enumerate which ink belongs on
  which band — anything more specific would be invented here rather than
  imported.
- **The region map is inferred from the rules the app is meant to follow.** A
  band drawn in the wrong place is one the map will confidently mislabel
  rather than notice. It is written into the run directory beside each frame
  so that judgement is visible; the independent check is the rendered handoff
  at 120×36.
- **`prompt_scoped`'s name is historical.** It existed to widen a grant with
  Tab; ADR 0003 removed the toggle, so the scene now covers a *queued* second
  prompt and a non-default selection, and sends no Tab. Both grant scopes are
  on screen in every prompt scene now, one per option row.
- **Scenes talk to an OpenAI-compatible fake, never the Anthropic client.**
  `base_url` is ignored for the anthropic provider, so a local fake can only
  be reached that way. A clean run says nothing about the Anthropic adapter.
- **Key encoding is untested by construction.** The harness chooses the bytes
  a key sends, so a defect in what foot would *actually* send for that key —
  `40cb6b1` was one — is invisible here. Never report a green run as evidence
  about key handling.
