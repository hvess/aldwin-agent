---
name: review
description: The feedback loop to run after a change to Mjolnir is ready for submission. Runs lint, tests, design-token checks and screenshot baselines, then has an independent subagent compare the rendered frames against the designs and score its confidence. Iterates until the change is clean and the judge is 90% confident. Use when a feature is finished, not while it is being written.
---

# Review

Five stages. Four are deterministic and one command runs them. The fifth is a
subagent, because no script can look at a picture and say whether it matches a
design.

Run this when a change is **ready for submission** — not while it is being
written, and not to explore. It answers two questions: did this break
anything, and does it do what it set out to do.

## Before you start

You need two things from the developer, and stage 5 is useless without them:

- **the goal** — what this change set out to do, in a sentence;
- **the focus** — which screens it touched.

Ask if you have not been told. Do not infer the focus from the diff: a change
to a shared helper touches screens its diff never names, and a judge pointed
at the wrong screens reports the whole app's backlog instead of this change.

## 1–4. The deterministic stages

```sh
cargo build && ./target/release/mjolnir-review review
```

Roughly three minutes, almost all of it capture. It prints one line per stage
and the directory of frames stage 5 needs.

| stage | what it runs | what a failure means |
| --- | --- | --- |
| 1 lint | `cargo clippy --workspace --all-targets -- -D warnings` | fix it before anything else; a lint failure means the other stages ran against code you are about to change |
| 2 test | `cargo test --workspace` | a regression, or a test that needed updating with the change |
| 3 tokens · generated | regenerates `crates/tui/src/tokens.rs` from `.claude/design/tokens/` and diffs | the app's palette and the design have drifted. `cargo run -p mjolnir-review -- tokens --write`, then read the diff before committing it |
| 3 tokens · cells | every cell in every captured frame uses a palette colour and a glyph from the closed table | the app painted something outside the design system |
| 4 screenshots | `cargo test -p mjolnir-tui --test render_snapshot` | the rendered frames changed. If the change is *meant* to change them, regenerate deliberately: `UPDATE_SNAPSHOTS=1 cargo test -p mjolnir-tui --test render_snapshot` — after reading the diff |

Two flags for iteration passes: `--no-capture` skips the frames when you are
only chasing a lint or a test, and `--quiet-ms 150` roughly halves capture
time. Restore the default for the pass stage 5 reads.

**Every one of these must pass before stage 5 runs.** A judge looking at
frames drawn with a drifted palette is a judge reporting a consequence as a
cause.

## 5. Confidence

Spawn **one subagent per focused screen**, or one for the whole focus set if
it is small. Give it the prompt below, filled in. It is deliberately strict:
this is the flaky stage, and the prompt is the only thing mitigating that.

> You are reviewing rendered frames of a terminal UI against the design system
> they are meant to implement. You are blind to the code by design: you do not
> see the diff, the source, or any earlier review.
>
> **The change under review:** `<goal>`
> **The screens it touched:** `<focus>`
>
> **Judge only those screens.** Other screens are in the frames; ignore them
> entirely. Deviations elsewhere are not this change's business, and reporting
> them is the single most common way this stage goes wrong.
>
> Read, in this order:
> 1. `.claude/design/IMPORT.md`, then `HANDOFF.md`, then `tokens/*.css`.
> 2. `.claude/adr/*.md` — numbered decisions that amend the design. A frame
>    following one of these is conformant, not deviant.
> 3. `crates/review/baseline.json`, the `contradictions` array — places the
>    design contradicts *itself*. Do not report these as defects.
>
> Read nothing else. Not `crates/tui`, not `.claude/spec`.
>
> The frames are at `<frames dir>`, named `<scene>-<size>-<theme>.png`. The
> grid is `tokens/cells.css`: margin 3, label column 8, gutter 2, so body text
> lands on cell 13 — derive it; there is deliberately no `--body-col`. The
> capture cell is 8×18px, so cell column N starts at pixel x = 8N.
>
> Produce exactly two things:
>
> 1. **A confidence score, 0–100**: how confident you are that these screens
>    match their designs. Not how good they look — how closely they match.
> 2. **A list of what does not match.** For each: what the design specifies
>    and where you measured that (file and line), what the frame draws
>    instead (cells, colours, rows), which frames show it, and how sure you
>    are.
>
> If a screen matches, say so in one line. Do not pad the list. A finding you
> cannot cite a design line for is not a finding.

Write the score and the findings down. Then:

- **≥ 90** — stage 5 passes. The review is done.
- **< 90** — fix what the judge found, then run the whole loop again from
  stage 1. A fix that changes a frame changes which code paths that frame
  exercises, so stages 1–4 have to re-run, not just stage 5.

**Cap the loop at five iterations.** If it has not reached 90 by then, stop
and take it to the developer: five failed passes is not a fix problem, it is a
disagreement about what the design means, and another iteration will not
settle it.

**Spawn a fresh subagent every iteration.** One that remembers its last score
anchors on it, and one that knows what you changed is biased toward seeing the
change work.

## When the judge is wrong

It will sometimes be, because the design reference contradicts itself in
places. When a finding turns out to be the *design's* fault rather than the
app's, add it to `contradictions` in `crates/review/baseline.json` — both
halves of what the design says, and which half the app follows. Remove the
entry when the design is fixed upstream.

**Keep that list short.** It is a bug list for the design system, and a bug
list that only grows is a list nobody reads. If it is getting long, the answer
is to fix the design, not to keep recording it.

## What this does not cover

- **Stage 4 is a regression baseline, not a correctness one.** It proves the
  frames did not change. Whether they were ever *right* is stage 5's.
- **Stage 3 checks tokens and cells, not layout.** Nothing mechanical here
  asks whether a band is in the right place; that question needs a reference
  frame to compare against, which the design system does not currently ship.
- **`cargo fmt` is not in stage 1.** The codebase's alignment needs
  nightly-only rustfmt options and the workspace pins no nightly; see
  `stages::lint`.
- **Stage 5 is flaky and no prompt fixes that.** Two runs will not produce the
  same number. Treat the findings as the output and the score as a threshold,
  not as a measurement.
