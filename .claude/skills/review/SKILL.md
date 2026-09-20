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
| 0 toolchain | `rustc --version` against the baseline | the toolchain moved. Clippy's lint set changes between releases, so stage 1 may now fail on code nobody touched — record the new version deliberately rather than puzzling over it |
| 1 lint | `cargo clippy --workspace --all-targets -- -D warnings` | fix it before anything else; a lint failure means the other stages ran against code you are about to change |
| 2 test | `cargo test --workspace` | a regression, or a test that needed updating with the change |
| 3 tokens | regenerates `crates/tui/src/tokens.rs` from `.claude/design/tokens/` and diffs | the app's design system and the imported one have drifted. `cargo run -p mjolnir-review -- tokens --write`, then read the diff before committing it |
| 4 frames | `cargo test -p mjolnir-tui --test render_snapshot` | either the rendered frames changed against the baseline, or a cell left the design system — the failure names which. If the change is *meant* to alter the frames, regenerate deliberately after reading the diff: `UPDATE_SNAPSHOTS=1 cargo test -p mjolnir-tui --test render_snapshot` |

**All five are hermetic.** Same inputs, same result, no clock, no network, no
subprocess of the app, no compositor. The whole path runs in about fifteen
seconds, almost all of it `cargo test`. Capture is *not* a stage — it runs
after them, only to make the pictures stage 5 looks at.

Stage 4 does two jobs and both read `TestBackend` buffers: the snapshot
baseline, and design conformance — every colour is one of the forty-two roles
`tokens.rs` carries (or one dimmed toward a ground), every glyph is from the
closed table, and the app's own copy is third person with no contractions.

Two flags: `--no-capture` skips the pictures entirely, which is what you want
for every pass that is not going to reach stage 5; `--quiet-ms 150` roughly
halves capture time when you do need them. Restore the default for the pass
stage 5 actually reads.

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

- **Stage 4's baseline half proves the frames did not change**, not that they
  were ever right. Whether they were is stage 5's.
- **Nothing mechanical checks layout.** Stages 3 and 4 check tokens, colours,
  glyphs and copy. Whether a band is in the *right place* needs a reference
  frame to compare against, and the design system does not ship one.
- **Stage 4's copy and glyph checks are scoped to app-owned rows.** A model's
  reply is rendered verbatim and may legitimately contain an em dash or a
  contraction, so a global check would fail on ordinary use. The split is by
  scene and band — see `app_owned_rows` in the test, which states what it
  misses.
- **Capture is not hermetic and does not need to be.** It spawns a
  compositor, a terminal and a fake provider, and waits `quiet_ms` for the
  app to settle. It is stage 5's input, not a gate: a bad frame is something
  the judge will say out loud.
- **`cargo fmt` is not in stage 1.** The codebase's alignment needs
  nightly-only rustfmt options and the workspace pins no nightly; see
  `stages::lint`.
- **Stage 5 is flaky and no prompt fixes that.** Two runs will not produce the
  same number. Treat the findings as the output and the score as a threshold,
  not as a measurement.
