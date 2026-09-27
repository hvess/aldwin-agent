---
name: review
description: The feedback loop every agent commit to Aldwin runs, and cannot land without. Ten stages — toolchain, lint (the rust skill's checkable rules included), tests, design tokens, frame snapshots, then blind subagent judges for the code (quality-gate, constraints, ADRs), the Rust (the rust skill) and the changed frames (the design), iterated until nothing is found, and a commit gate that refuses any tree without a passing review. Use before every commit, not while the change is being written.
---

# Review

Ten stages. Five are deterministic and one command runs them. Three are
subagents, because no script can read code against a design principle or
look at a picture. One is the loop itself, and the last is the gate.

| stage | what | who |
| --- | --- | --- |
| 1 toolchain | `rustc --version` against the baseline | `review` |
| 2 lint | `cargo fmt --check`, `clippy -D warnings` with the workspace lints — the `rust` skill's checkable rules | `review` |
| 3 test | `cargo test --workspace` | `review` |
| 4 tokens | regenerate `tokens.rs` from `.claude/design/` and diff | `review` |
| 5 frames | `render_snapshot`: the baseline and design conformance | `review` |
| 6 code judge | the diff against `quality-gate`, the Key Constraints, the ADRs, the crate's spec | a subagent |
| 7 Rust judge | the diff against the `rust` skill's rules no lint checks | a subagent |
| 8 frames judge | the changed scenes' frames against the design | a subagent |
| 9 iterate | any failure: fix, and run again from stage 1 | you |
| 10 gate | the commit is refused unless exactly its tree passed | the pre-commit hook |

**An agent's commit cannot land without this.** `.githooks/pre-commit`
runs stage 10 for every commit made by an agent — any process with `AGENT`
set, which Claude Code's project settings and Aldwin both set — and a
`PreToolUse` guard refuses the ways around it (`--no-verify`, `-n`, changing
where hooks are read from, and the git commands that write commits without
the hook). That is deliberate and it is not to be worked around: if the gate
refuses, the answer is to run this loop, never to find another way to
commit. If a command you need is refused, ask the developer.

## Before you start

**Stage exactly what the commit is, and nothing else.** A review reads the
working tree and records the staged one, so `review` refuses to run while
they differ — untracked files included. Put anything that is not part of
this commit aside first (`git stash push --keep-index --include-untracked`
is one way).

Nothing else is asked of you. There is no goal and no focus to write: which
judges this change needs is read off the staged diff, and which frames stage
8 looks at off which scenes' snapshot it changes. Nothing the author writes
is an input to its own review.

## 1–5. The deterministic stages

```sh
cargo run --release -p aldwin-review -- review
```

Always through `cargo run`, never a path under `target/`: a path runs
whatever was last built there, and `cargo build` builds the debug profile,
so `cargo build && ./target/release/…` ran a stale release binary — a loop
older than the change it was reviewing.

It prints one line per stage, then one line per judge — required or not,
and why — and writes a run directory under `target/review-frames/` holding
`review.html`, `run.json`, `change.diff` and, when stage 8 is required, the
frames.

| stage | what a failure means |
| --- | --- |
| 1 toolchain | the toolchain moved. Clippy's lint set changes between releases, so stage 2 may now fail on code nobody touched — record the new version deliberately in `baseline.json` rather than puzzling over it |
| 2 lint | fix it before anything else (`cargo fmt` for the first). A missing doc, `# Errors`, `# Panics` or `Debug` is the `rust` skill's rule, enforced — write the doc; never `#[allow]` it |
| 3 test | a regression, or a test that needed updating with the change |
| 4 tokens | the app's design system and the imported one have drifted. `cargo run -p aldwin-review -- tokens --write`, then read the diff before committing it |
| 5 frames | the rendered frames changed against the baseline, or a cell left the design system — the failure names which. If the change is *meant* to alter the frames, regenerate deliberately after reading the diff: `UPDATE_SNAPSHOTS=1 cargo test -p aldwin-tui --test render_snapshot`, and stage the new `render.snap` |

**All five are hermetic.** Same inputs, same result, no clock, no network,
no subprocess of the app, no compositor — about fifteen seconds, almost all
of it `cargo test`. Capture is *not* a stage: it runs after them, only when
stage 8 is required, only for the changed scenes.

**Every one of these must pass before any judge runs.** A judge reading code
that does not build, or frames drawn with a drifted palette, reports a
consequence as a cause. The report enforces it: it leaves a judge a
placeholder only when stages 1–5 passed, so `judge` refuses to write into
any other run.

`--stages-only` runs stages 1–5 and stops, without requiring the tree to be
staged. It is the check to run while you are still working. It records
nothing, and it is not a review.

`UPDATE_SNAPSHOTS` is removed from every `cargo` the loop runs, so a shell
that still exports it cannot have stage 5 rewrite the baseline it checks.

### Which judges run

`review` decides, from what the staged diff touches:

- **stage 6, code** — any path under `crates/`, `.githooks/` or
  `.claude/hooks/`, or `Cargo.toml` / `Cargo.lock` / `.claude/settings.json`
  — the gate's own enforcement is judged like code;
- **stage 7, Rust** — any `.rs` file;
- **stage 8, frames** — any scene whose section of `render.snap` changed.
  Every snapshot scene is also a capture scene (a test in `scene.rs` keeps
  the two lists equal), so those are exactly the frames it looks at.

A change that calls for no judge — docs only — is recorded as passed as soon
as stages 1–5 pass. Otherwise `review` exits non-zero: a review whose judges
have not run is not a review.

## 6–8. The judges

Spawn **one fresh subagent per required judge, per iteration, all in
parallel.** Not two to compare, not three to vote — a second judge measures
variance, which is a thing to fix in the prompt rather than average away.
Fresh every iteration: one that remembers its last verdict anchors on it,
and one that knows what you changed is biased toward seeing the change work.

Hand each one its prompt below, filled in with the paths `review` printed —
the run's `change.diff`, and for stage 8 the exact frame list. Never the
directory to glob, never your own account of the change.

What every prompt does, each for a measured reason:

- **It ranks its sources.** The sources disagree in places, and a judge with
  no precedence rule invents one. Two frame judges on the same unchanged
  pixels once returned opposite verdicts because the prompt did not say
  which source won.
- **It refuses to let low confidence become a finding.** Anything unsure goes
  under "questions", which does not fail the stage.
- **It is blind.** The judge sees the change, not the conversation that made
  it and not an earlier verdict.
- **It asks for findings, not a number.** A verdict passes with no findings
  and fails with any; severity orders the fixing.
- **Its whole output is one JSON block**, saved verbatim — so the judge's own
  words reach the report, not the author's transcription of them.

The shape all three return:

    ```json
    {
      "iteration": 1,
      "findings": [
        { "severity": "major",
          "source":   "quality-gate §2 — the core knows no adapters",
          "expected": "aldwin-core names no adapter crate",
          "found":    "core::agent imports aldwin_llm::OpenAiCompatibleClient",
          "at":       "crates/core/src/agent.rs:12" }
      ],
      "contradictions": ["one string per place a source disagrees with itself, both halves cited"],
      "questions":      ["one string per thing you could not resolve"],
      "matches":        ["one string per thing you checked and found holds"]
    }
    ```

### Stage 6 — the code judge

> You are reviewing a change to Aldwin, a Rust TUI coding agent, against the
> decisions the project has made about its code. You are blind to the
> conversation that produced the change by design: you see the change, not
> its author's reasons.
>
> **The change:** `<run>/change.diff` — the staged diff. The working tree
> holds exactly that change applied, so you may read any file under
> `crates/` for the context a hunk needs. **Judge only what the diff
> changes.** Code it does not touch is not this change's business, and
> reporting it is the most common way this stage goes wrong.
>
> **What you judge against, and which source wins.** When two disagree, the
> one higher in the list wins:
>
> 1. `.claude/CLAUDE.md`, the section **Key Constraints (non-negotiable)**.
> 2. `.claude/adr/*.md` — numbered decisions. A change that follows a later
>    ADR where an earlier one disagrees is conformant.
> 3. `.claude/skills/quality-gate/SKILL.md` — every section.
> 4. The spec for each crate the diff touches: `.claude/spec/aldwin-<crate>.md`,
>    or `.claude/spec/archive/` for an archived one. A spec step the change
>    completes should be noted in it. A change to `.githooks/`,
>    `.claude/hooks/` or `.claude/settings.json` is the review loop's own
>    enforcement: judge it against `.claude/spec/aldwin-review.md`,
>    Decisions 15 and 16. Decision 16 says what the commit guard is for: a
>    spelling built to get past it is out of its scope, not a finding.
>
> Read nothing else, and do not judge Rust idiom — a separate judge owns the
> `rust` skill.
>
> - **major** — the change breaks source 1, 2 or 3.
> - **minor** — the change breaks source 4, or leaves a spec or the
>   open-tasks ledger (`.claude/spec/aldwin-open-tasks.md`) stale where it
>   completed or discovered work.
>
> **Your entire output is one fenced `json` block and nothing else**, in the
> shape above. At most six findings, each one you can demonstrate with a
> `file:line` and the sentence of the source it breaks. A finding whose
> source you cannot name is not a finding. Anything you are unsure of is a
> question. Do not pad.

### Stage 7 — the Rust judge

> You are reviewing a change to a Rust workspace against one document: the
> project's `rust` skill. You are blind to the conversation that produced
> the change by design.
>
> **The change:** `<run>/change.diff` — the staged diff. You may read any
> file under `crates/` for context. **Judge only the lines the diff adds or
> changes.**
>
> **What you judge against:** `.claude/skills/rust/SKILL.md`, and nothing
> else. Its Good and Avoid examples are this codebase's own code; follow the
> rule they illustrate, not their exact text.
>
> Some of its rules are already enforced by `cargo clippy -D warnings` and
> the workspace lints, and the change has passed them: formatting, naming,
> `?` over `match` where clippy's `question_mark` fires, docs on public
> items, `# Errors` and `# Panics` sections, `Debug` on public types. **Do
> not report those** — they cannot be present. Judge what no lint checks,
> among them:
>
> - iterator chains and combinators over manual loops with `push`;
> - `?` through `From` / `#[from]` rather than `map_err` to the same
>   conversion, and `map_err` only for context `From` cannot express;
> - `thiserror` for error types;
> - imports grouped per crate with braces, and imports rather than
>   qualified paths in signatures;
> - examples in the doc comments of public functions the change *adds* (a
>   doc the change writes for an existing item does not need one);
> - "Avoid Overengineering": only what the task required, no extra layers;
> - "A bug fix lands with the test that would have caught it".
>
> - **major** — a rule the skill states as a requirement, broken in code the
>   change adds.
> - **minor** — a preference it states ("prefer", "where appropriate")
>   passed over without a reason written next to it.
>
> **Your entire output is one fenced `json` block and nothing else**, in the
> shape above: `source` is the skill's section heading, `at` is `file:line`.
> At most six findings. Anything unsure is a question.

### Stage 8 — the frames judge

> You are reviewing rendered frames of a terminal UI against the design
> system they are meant to implement. You are blind to the code by design:
> you do not see the diff, the source, or any earlier review.
>
> **The scenes whose rendering this change moved** are the ones in the list
> below. **Judge only those frames.** Deviations elsewhere are not this
> change's business, and reporting them is the single most common way this
> stage goes wrong.
>
> **What you may read, and which source wins.** When two disagree, the one
> higher in the list wins — this is not a tiebreak you get to make:
>
> 1. `.claude/adr/*.md` — numbered decisions that deliberately amend the
>    design. A frame following one is conformant, full stop.
> 2. `crates/review/baseline.json`, the `contradictions` array —
>    disagreements already settled. Never report one of these.
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
> Put it under "contradictions" and move on.
>
> **The frames:** `<the exact list review printed>`
>
> Each `.png` has a `.txt` beside it, same name: the app's own declared grid,
> one line per row, every character at its exact column. **Use the `.txt`
> for anything positional** — columns, rows, alignment, spacing, copy.
> **Use the `.png` only for colour.** The grid is `tokens/layout.css`: a
> 3-cell margin, a 2-cell mark column, prose on cell 5 (`--body-x`). There is
> no label column. The capture cell is 8x18px, so cell column N starts at
> pixel x = 8N.
>
> - **major** — the frame contradicts source 1, 2 or 3.
> - **minor** — the frame contradicts source 4, or a source 5 statement
>   nothing higher speaks to.
>
> **Your entire output is one fenced `json` block and nothing else**, in the
> shape above: `source` is the ranked source and where in it, `expected` what
> it states, `found` what the frame shows (row and cells), `at` which frames.
> At most six findings. Do not promote a guess to a minor finding; saying you
> could not tell is worth more.

### Write each verdict, with the command

Save each judge's JSON block verbatim and run, once per judge:

```sh
cargo run --release -p aldwin-review -- judge --run <dir> --stage 6 --findings code.json
```

Never transcribe or edit the block: that routes the judge's findings through
the hands of the agent being judged. `judge` renders it into the report,
passes the stage only with no findings, and exits non-zero otherwise. When
the last required judge passes — and the index is still the tree the run
reviewed — it writes the pass record, and the commit can go through.

**This is a command rather than an instruction for a reason.** When the
verdict was appended by hand, the section came back empty on three
consecutive runs: the agent was reading findings and fixing code, which is
exactly when a manual step gets skipped.

## 9. Iterate

Any failure — a stage, or a judge's finding — means: fix it, stage the fix,
and run the whole loop again from stage 1, with fresh judges. A fix changes
the tree, so a verdict on the old tree says nothing about the new one; the
record is keyed by tree for exactly that reason.

**Cap it at five iterations.** If it has not passed by then, stop and take it
to the developer: five failed passes is a disagreement about what a source
means, and another iteration will not settle it.

## 10. Commit

Commit exactly what was staged. `.githooks/pre-commit` runs
`aldwin-review gate`, which finds the pass record for the tree being
committed or refuses with the sentence to act on. A commit that changes
anything after the review — `git commit -a` with unstaged edits, a fixup —
is a different tree and is refused until it is reviewed.

## When a judge is wrong

It will sometimes be, because the sources contradict themselves in places.

- A frames finding that is the *design's* fault goes into `contradictions`
  in `crates/review/baseline.json` — both halves of what the design says, and
  which half the app follows. Where the design disagrees with Apple's HIG,
  the second half is the HIG's own sentence, quoted. **Keep that list
  short**: it is a bug list for the design system.
- A code or Rust finding that contradicts a later decision is resolved by
  the decision: if no ADR or skill says it, the fix is to say it there — a
  judge should never have to be argued with twice over the same point.

**Any finding a judge produces twice belongs in a deterministic stage.** A
frames finding that cites a token becomes an assertion in
`crates/tui/tests/render_snapshot.rs`; a Rust finding a lint can express
becomes a workspace lint; a code finding about structure becomes a test.
That is what keeps the judges getting cheaper instead of accumulating a
longer checklist.

## What this does not cover

- **Whether the change is what the developer wanted.** That is the
  developer's to judge; the loop checks that it breaks nothing and holds to
  what the project has decided.
- **Stage 5's baseline proves the frames did not change**, not that they were
  ever right. Whether they are is stage 8's, for the scenes that moved.
- **Nothing mechanical checks layout.** Stages 4 and 5 check tokens,
  colours, glyphs and copy, not whether a band is in the right place.
- **Capture is not hermetic and does not need to be.** It is stage 8's input,
  not a gate: a bad frame is something the judge will say out loud.
- **The judges are not reproducible, and no prompt fixes that.** Treat the
  findings as the output.
- **The guard binds Claude Code only.** An Aldwin session's commits meet
  the gate, but nothing refuses its `--no-verify`; the developer's own
  commits are not gated.
