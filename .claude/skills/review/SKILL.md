---
name: review
description: The feedback loop to run after a change to Aldwin is ready for submission. Runs lint, tests, design-token checks and screenshot baselines, then has an independent subagent compare the rendered frames against the designs and score its confidence. Iterates until the change is clean and the judge is 90% confident. Use when a feature is finished, not while it is being written.
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
cargo build && ./target/release/aldwin-review review \
  --goal  "<what this change set out to do, in a sentence>" \
  --focus "<the scenes it touched>"
```

Both flags are required — a review that cannot say what it is reviewing
cannot judge whether the change did it, and stage 5 is handed them verbatim.

Roughly three minutes, almost all of it capture. It prints one line per stage,
the directory of frames stage 5 needs, and writes **`review.html`** into that
directory: the goal, the focus, the commit, and a row per stage with what it
measured. Any stage that failed gets its diagnostic quoted underneath. It is
self-contained — no stylesheet, no script, no embedded frames — so it opens
in a browser from disk and survives being moved.

| stage | what it runs | what a failure means |
| --- | --- | --- |
| 0 toolchain | `rustc --version` against the baseline | the toolchain moved. Clippy's lint set changes between releases, so stage 1 may now fail on code nobody touched — record the new version deliberately rather than puzzling over it |
| 1 lint | `cargo clippy --workspace --all-targets -- -D warnings` | fix it before anything else; a lint failure means the other stages ran against code you are about to change |
| 2 test | `cargo test --workspace` | a regression, or a test that needed updating with the change |
| 3 tokens | regenerates `crates/tui/src/tokens.rs` from `.claude/design/tokens/` and diffs | the app's design system and the imported one have drifted. `cargo run -p aldwin-review -- tokens --write`, then read the diff before committing it |
| 4 frames | `cargo test -p aldwin-tui --test render_snapshot` | either the rendered frames changed against the baseline, or a cell left the design system — the failure names which. If the change is *meant* to alter the frames, regenerate deliberately after reading the diff: `UPDATE_SNAPSHOTS=1 cargo test -p aldwin-tui --test render_snapshot` |

**All five are hermetic.** Same inputs, same result, no clock, no network, no
subprocess of the app, no compositor. The whole path runs in about fifteen
seconds, almost all of it `cargo test`. Capture is *not* a stage — it runs
after them, only to make the pictures stage 5 looks at.

Stage 4 does two jobs and both read `TestBackend` buffers: the snapshot
baseline, and design conformance — every colour is one of the nineteen roles
`tokens.rs` carries (or a mark or gauge mix of two of them), every glyph is
from the closed table, nothing is stroked, and the two hue rules hold: the
agent's prose is never blue, nothing outside a diff is red or green.

Two flags: `--no-capture` skips the pictures entirely, which is what you want
for every pass that is not going to reach stage 5; `--quiet-ms 150` roughly
halves capture time when you do need them. Restore the default for the pass
stage 5 actually reads.

**Every one of these must pass before stage 5 runs.** A judge looking at
frames drawn with a drifted palette is a judge reporting a consequence as a
cause.

## 5. Confidence

Spawn **one subagent per iteration. One.** Not two to compare, not three to
vote — a second judge doubles the cost of the slowest stage and what it buys
is a measurement of variance, which is a thing to fix in the prompt rather
than average away.

Give it the prompt below, filled in. It is deliberately strict: this is the
only stage that is not reproducible, and the prompt is most of what bounds
that.

Five things the prompt does that matter, each for a measured reason:

- **It ranks its sources.** The design's screen prose, its own token tables,
  its tokens and the ADRs disagree in places, and a judge with no precedence
  rule invents one. Two judges on the same unchanged pixels once returned
  opposite verdicts on the same option-row tones — one calling them correct
  "through HANDOFF's superseded Nocturne table", the other calling them a
  major deviation. That is not a judge being careless; it is a prompt failing
  to say which source wins.
- **It refuses to let low confidence become a deduction.** Anything unsure
  goes under "questions", which does not score. The earlier version defined
  `minor` as "a nit, or anything you hold at low confidence", which invited
  guesses into the arithmetic.
- **It enumerates the frames.** `review` prints the exact paths for the
  focused scenes — hand it that list, not the directory. A judge that has to
  decide what to open is a judge making a decision you did not ask it to make.
- **It sends geometry to the `.txt` and colour to the `.png`.** Every frame
  ships a declared grid beside it: 521 bytes against 28 KB, every character at
  its exact column. Two judges once spent the bulk of 30-odd tool calls
  pixel-sampling positions that were sitting in that file.
- **It does not ask for a number.** The judge reports findings with a
  severity; you compute the score. A model picking "80" cannot say what makes
  it 80 rather than 70, and the arithmetic below can.

> You are reviewing rendered frames of a terminal UI against the design system
> they are meant to implement. You are blind to the code by design: you do not
> see the diff, the source, or any earlier review.
>
> **The change under review:** `<goal>`
> **The screens it touched:** `<focus>`
>
> **Judge only those screens.** Deviations elsewhere are not this change's
> business, and reporting them is the single most common way this stage goes
> wrong.
>
> **What you may read, and which source wins.**
>
> Read all of these. When two of them disagree — and they do — the one
> higher in this list wins. This is not a tiebreak you get to make:
>
> 1. `.claude/adr/*.md` — numbered decisions that deliberately amend the
>    design. A frame following one is conformant, full stop.
> 2. `crates/review/baseline.json`, the `contradictions` array — disagreements
>    already settled. Never report one of these.
> 3. `.claude/design/tokens/*.css` — the token layer. This is what the app
>    can actually draw through, so it is the operative statement.
> 4. `.claude/design/frames/Aldwin Agent TUI.dc.html` — the ten frames.
>    Every position in them is a `var(--…)` from `tokens/layout.css`, so a
>    position is a lookup, not a measurement; the brand mark and the context
>    bar's ramp exist only here.
> 5. `.claude/design/README.md` — the design's prose: content fundamentals,
>    the colour rules, the glyph list. It states rules the frames only show.
>
> Read `.claude/design/IMPORT.md` first for context. Read nothing else — not
> `crates/tui`, not `.claude/spec`.
>
> **If a lower source contradicts a higher one, that is not an app defect.**
> The app is drawing what the higher source says. Put it under
> "contradictions" below, which does not affect the score, and move on.
>
> **The frames, and how to read them:**
>
> `<the exact list review printed>`
>
> Each `.png` has a `.txt` beside it, same name: the app's own declared grid,
> one line per row, every character at its exact column. **Use the `.txt` for
> anything positional** — columns, rows, alignment, spacing, copy. **Use the
> `.png` only for colour.** Sampling a pixel to find a column is slow and
> gets you an antialiased edge; the grid is exact.
>
> The grid is `tokens/layout.css`: a 3-cell margin, a 2-cell mark column,
> prose on cell 5 (`--body-x`, declared and equal to the sum). There is no
> label column. The capture cell is 8x18px, so cell column N starts at
> pixel x = 8N.
>
> **Your entire output is one fenced `json` block and nothing else.** No
> preamble, no commentary around it. This shape:
>
>     ```json
>     {
>       "iteration": 1,
>       "findings": [
>         { "severity": "major",
>           "source":   "3 — tokens/layout.css",
>           "design":   "layout.css:7 — --group-gap is 5ch between footer groups",
>           "frame":    "footer row 31: groups start at cells 5, 16 and 26 — 3 cells apart",
>           "frames":   "all 30" }
>       ],
>       "contradictions": ["one string per place the design disagrees with itself, both halves cited"],
>       "questions":      ["one string per thing you could not resolve"],
>       "matches":        ["one string per thing you checked and found correct"]
>     }
>     ```
>
> - **findings** — things you can demonstrate, at most six. These are the only
>   entries that affect the score.
>   - **blocking** — the change under review does not do what it set out to do.
>   - **major** — the frame contradicts source 1, 2 or 3.
>   - **minor** — the frame contradicts source 4, or a source 5 statement
>     nothing higher speaks to.
> - **contradictions** — the design against itself. Do not score.
> - **questions** — unresolved or low confidence. Do not score. **Do not
>   promote a guess to a minor finding**; saying you could not tell is worth
>   more than a number.
> - **matches** — what you checked and found correct, one line each.
>
> A finding whose `source` you cannot name is not a finding. Do not pad, and
> do not give an overall score — the score is computed from your severities.

### Write the result, with the command

Do not hand-edit the HTML:

```sh
./target/release/aldwin-review stage5 --run <dir> --findings findings.json
```

where `findings.json` is **the judge's JSON block, saved verbatim**. The
prompt asks for exactly that block and nothing else, so this is a paste
rather than a transcription.

That matters for two reasons. Transcribing the judge's prose into JSON by
hand is tedious enough that it is where the step died five times out of
seven — and worse, it routes the judge's findings through the hands of the
agent whose work is being judged. The judge's own words go in the report.

It derives the score from the severities — `100 − (25 × blocking) − (15 ×
major) − (5 × minor)`, floored at 0, so **90 means at most two minor
deviations and nothing else** — renders the section with the arithmetic
shown, prints the verdict and exits non-zero below the threshold.

**This is a command rather than an instruction for a reason.** The first
version of this skill asked the agent to append the section by hand, and it
came back empty on three consecutive runs: the agent was reading findings and
fixing code, which is exactly when a manual step gets skipped. A report step
that depends on remembering is a report step that will not happen.

- **≥ 90** — stage 5 passes. The review is done.
- **< 90** — fix what the judge found, then run the whole loop again from
  stage 0. A fix that changes a frame changes which code paths that frame
  exercises, so the deterministic stages have to re-run too.

**Cap the loop at five iterations.** If it has not reached 90 by then, stop
and take it to the developer: five failed passes is a disagreement about what
the design means, and another iteration will not settle it.

**Spawn a fresh subagent every iteration.** One that remembers its last score
anchors on it, and one that knows what you changed is biased toward seeing the
change work.

### Graduating a finding

**Any finding this stage produces twice belongs in stage 4.** If two judges —
or two runs — report the same deviation and it cites a *token* rather than
prose, it is a deterministic check being run non-deterministically, expensively,
and without a failure message. Write the assertion into
`crates/tui/tests/render_snapshot.rs` and let the judge stop paying attention
to it.

That is what keeps this stage getting cheaper instead of accumulating a longer
checklist. The three-cell margins and the "nothing outside a diff is red"
check arrived that way.

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
