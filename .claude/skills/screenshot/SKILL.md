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

## 3. Capture, gate, and check against the design

```sh
mjolnir-screenshot run --run <dir>
```

Every scene the session names, at all three sizes in both themes — six frames
each. Each frame leaves the PNG a human reads, the declared cell grid, the
inferred region map, **the declared spans (`.facts.json` / `.facts.txt`)**,
**the assertion results (`.expect.json`)**, and its gate results. Frames with
a violation or a failed assertion also get a `.marked.png`.

`--quiet-ms 150` roughly halves capture time for an iteration pass; restore
the default for the pass the verdict is read from. `--theme dark` halves it
again, and is legitimate because the run proves the themes are a palette swap
(below) — but only for a pass that is not about colour.

### The seven gates

Deterministic, zero-tolerance, and they read the **declared cells** — what
the app said it was drawing — never the picture:

| gate | checks |
| --- | --- |
| colour | every declared colour is a role in that theme's palette, or a role dimmed toward a ground |
| breakages | unpainted cells, glyphs outside the design's table, a wide glyph clipped at the row edge |
| layout | nothing but a band's ground in the 3-cell margin |
| role pairing | no band painted in an ink role, no glyph painted in a ground rung |
| content | third person, no contractions, and a count that agrees with its pronoun |
| contrast | ink at 3.3:1, a mark at 1.6:1, a band against its neighbour at 1.15:1 — every floor the design states about itself |
| regression | every `render.snap` region the diff touches falls inside the focus set |

### The design assertions

`crate::expect` holds `HANDOFF.md`'s screen sections as assertions the harness
executes — positions, tones, row counts, separators — each citing the line it
comes from, each resolving its arithmetic through `tokens/cells.css` rather
than restating a number. **This is what the verdict is computed from.**

```sh
mjolnir-screenshot conformance --run <dir>
```

Every failure, grouped by design screen, with its citation. Read this first;
it is the deterministic half of what blind judging used to produce, it runs in
seconds, and on the run that introduced it it independently found four of the
twelve deviations that had cost six judges 890K tokens.

`run` also proves, per run, that the two themes are a **palette swap** —
identical declared grids, differing only in resolved hex. That is what
licenses judging one theme instead of two. If it ever reports `THEMES
DIVERGE`, a theme-conditional layout has appeared and the one-theme shortcut
is no longer sound.

A gate or an assertion failing does not stop the run; it blocks the exit.

## 4. Judge — advisory, and much smaller than it used to be

```sh
mjolnir-screenshot judge-set --run <dir>
```

That prints the frames to hand over and what the judge may read. **Twelve
frames, not seventy-two**: one theme, because the run proves the declared
grids are identical across themes, so a light frame cannot hold a spatial
defect its dark twin does not; and the 120×36 design frame, because that is
the only frame the design specifies.

Spawn a **separate agent**. It gets the frames, their `.facts.txt`, the
criteria, the focus set, `.claude/design/` and `.claude/design/ERRATA.md`. It
does **not** get the diff, the source, or its own earlier scores, and you do
not tell it what you changed or hope it will say.

An agent that both fixes and judges converges on its own scorer; one that
knows the intent behind a change is biased toward seeing that intent met; one
that remembers its last score anchors on it.

Three things changed about this step, each for a measured reason:

- **Hand it `.facts.txt`, not a PNG to decode.** Every span's declared role,
  hex, band and contrast is in that file. Six judges once spent most of 890K
  tokens recovering exactly that from pixels, and three of them inferred a
  role name wrongly on the way — `--tui-dim` from a hex that was declared
  `--tui-context`.
- **`ERRATA.md` is admissible, and nothing else is.** It is the reference
  correcting itself — prose against its own tokens, prose against its own
  rendered frames, and the ADRs — under a rule that an entry may cite only a
  measurement or a numbered ADR. That is not "a bit of context about the
  change", which remains forbidden. Without it, judges re-derive the same
  four settled contradictions every single run.
- **Ask for findings against named rules, not a 0–100.** A score from a fresh
  model with no anchors is uncalibrated: one judge gave two screens 64 and 86
  on substantially the same finding set. Scores may still be recorded in
  `session.json` and the report still prints them, but they are **advisory
  and do not gate**. What the judge is for is the deviation nobody wrote an
  assertion about — on the run that proved this, two of twelve.

## 5. Loop

Iterate capture → score → fix, at most **five times**, recording what you
changed each pass in `session.json`'s `iterations`. Ask the crate whether you
may stop rather than deciding yourself:

```sh
mjolnir-screenshot verdict --run <dir>     # non-zero while the loop must continue
```

    exit = preflight clean ∧ regression in focus ∧ no gate violation
           ∧ no failed assertion ∧ iterations ≤ 5

The judge's score is **not** in that conjunction any more, and was for five
runs during which the loop never once exited. What replaced it is the
assertion suite: reproducible bit-for-bit, comparable between runs, and every
failure already carrying the `HANDOFF.md` line it violated.

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
- **An assertion suite only checks what somebody wrote down.** It cannot find
  the deviation nobody enumerated. On `run-1789850385` two of twelve new
  findings were of that kind — a tool line's name and target sharing one
  colour, and first run's option ramp — and no table would have held either.
  A clean `conformance` is not a clean design.
- **The `contrast` gate does not fail a dimmed span.** The transcript behind a
  permission panel is blended toward the ground, and no opacity satisfies both
  the design's stated 35% and its stated 3.3:1 ink floor — at 35% the dimmed
  body measures 2.706:1 dark and 1.975:1 light, *worse* than what ships. The
  design has no answer, so the gate does not invent one: the ratios are
  carried in `.facts.json` and reported, never failed. A regression in the dim
  would not be caught here.
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
- **`prompt_scoped` does not reach a queued second prompt.** It was written to
  widen a grant with Tab; ADR 0003 removed the toggle. It now sends two tool
  calls and a non-default selection — but its panel is **byte-identical** to
  `prompt_path`'s, and `decision::queue_note`'s `(+N more pending)` row
  appears in none of the 72 frames, because the second call is logged as
  running before its prompt is queued. The scene covers the selection and not
  the queue; open-tasks entry 14.
- **Scenes talk to an OpenAI-compatible fake, never the Anthropic client.**
  `base_url` is ignored for the anthropic provider, so a local fake can only
  be reached that way. A clean run says nothing about the Anthropic adapter.
- **Key encoding is untested by construction.** The harness chooses the bytes
  a key sends, so a defect in what foot would *actually* send for that key —
  `40cb6b1` was one — is invisible here. Never report a green run as evidence
  about key handling.
