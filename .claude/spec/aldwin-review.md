# aldwin-review

The feedback loop every agent commit to Aldwin runs, and cannot land without.

**Status:** active — built and in use. Replaced `mjolnir-screenshot` on
2026-09-20; became ten stages with a commit gate on 2026-09-27. See
Progress.
**Scope:** the ten-stage review loop — the crate `crates/review`
(`aldwin-review`) that runs the five deterministic stages, decides which
judges a change needs, writes their verdicts and keeps the pass record; the
`review` skill that drives the loop and owns the three judges; and the hooks
that make an agent's commit depend on it (`.githooks/`, `.claude/hooks/`,
`.claude/settings.json`). Excludes what the stages themselves test (that is
each crate's own spec) and the design system's content.
**Owner:** Maximilian
**Last Updated:** 2026-09-27 (the commit gate)

## Why

A change needs two questions answered before it lands: did it break
anything, and does it hold to what this project has decided — its
architecture, its Rust, its design. The first is mechanical and the second
mostly is not, and the whole design of this loop is keeping them apart.

Five stages are deterministic: they run a command, compare against something
committed, and say yes or no. Three are subagents, each reading one thing
against one set of sources. Nothing in the deterministic five makes a
judgement about whether the UI *looks like* the design, because the previous
harness tried exactly that and the attempt is what this spec replaces.

Whether the change is what the developer *wanted* is not a question the loop
asks. That is the developer's to judge — the premise of a discussion-first
tool — and every earlier attempt to ask it made the author write the
rubric it was graded against.

## The stages

| stage | answers | how | hermetic |
| --- | --- | --- | --- |
| 1 toolchain | are these results comparable to the last run's | `rustc --version` against the baseline | yes |
| 2 lint | is it formatted, does it build clean, and does it keep the `rust` skill's checkable rules | `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets -- -D warnings` with the workspace lints (`missing_docs`, `missing_debug_implementations`, `clippy::missing_errors_doc`, `clippy::missing_panics_doc`) | yes |
| 3 test | does the suite pass | `cargo test --workspace` | yes |
| 4 tokens | is the app's design system still the imported one | regenerate `crates/tui/src/tokens.rs` and diff | yes |
| 5 frames | do the frames match the baseline, and does every cell come from the design | `render_snapshot.rs` against `tests/snapshots/render.snap`, plus colour, glyph and copy conformance | yes |
| 6 code judge | does the diff hold to `quality-gate`, the Key Constraints and the ADRs | a blind subagent over `change.diff`; runs when a crate or the workspace manifest changed | no |
| 7 Rust judge | does the diff hold to the `rust` skill's rules no lint checks | a blind subagent over `change.diff`; runs when Rust source changed | no |
| 8 frames judge | do the changed scenes look like the design | a blind subagent over the captured frames of the scenes whose snapshot changed | no |
| 9 iterate | — | any failure: fix, run again from stage 1, fresh judges; at most five passes | — |
| 10 gate | was exactly this tree reviewed, and did it pass | `aldwin-review gate`, run by `.githooks/pre-commit` for an agent's commit | yes |

Stages 1–5 take about fifteen seconds and hold no clock, no network, no
subprocess of the app and no compositor. Capture is not a stage: it runs
after them, only for stage 8's scenes, and its non-determinism is harmless
there because a bad frame is something the judge says out loud.

Which judges run is read off the staged diff, never chosen, and the report
states each one's reason — including the one that matters most: a change
whose snapshot moved only in scenes capture cannot draw has no frames to
judge, and says so rather than passing silently.

## Decisions

1. **One screenshot baseline, not two.** `render.snap` serialises every
   cell's symbol, foreground, background and modifiers for thirteen scenes at
   three sizes in both themes — 72 sections — in under a second, in-process.
   Capturing the same frames through a real terminal and diffing those too
   would be a second fixture asserting the same thing on a slower clock. The
   real terminal earns its place by producing **pictures for stage 8**, which
   is the one thing `TestBackend` cannot do.

2. **The app's design system is generated, not transcribed.**
   `crates/tui/src/tokens.rs` is emitted from `.claude/design/tokens/*.css`
   and committed; stage 4 regenerates it and fails on any diff. Before this,
   `palette.rs` carried eighty-four hand-written hex literals with a
   `// neutral-200` comment beside each as the only link to the design.

   The first generation reproduced all eighty-four exactly, so the
   transcription had in fact been kept honest — which is the argument for
   generating it, not against. It was honest because someone was checking by
   hand, every time, forever.

3. **Ten roles are deliberately not carried**, listed with reasons in
   `crates/review/src/tokens.rs`: `--chrome` and `--dot` paint the mock's
   title bar, `--syn` and `--call` are reserved and applied nowhere, and the
   six `--canvas-*` roles are the documentation page around the frames. A
   role that is neither carried nor on that list fails the stage rather than
   being silently dropped. (Under Mjolnir the list was three `--tui-*` roles;
   the principle is the same.)

4. **Only tokens the app consumes are generated.** `layout.css` declares the
   mock's window measures — `--fw`, `--chrome-h`, the body heights — which
   are pixels, not cells, and which the app does not read. Emitting a
   constant nothing uses would be the generator asserting a layout rule;
   whether the app *should* consume one is stage 8's question.

5. **Stage 8 judges only the scenes the change moved.** The app has
   deviations that cannot be fixed in `crates/tui` — there is no branch in
   `StatusInfo`, no clock in the workspace, and the design has no
   edit-approval screen. A judge assessing the whole app reports those every
   run and the loop never terminates, which is precisely how the previous
   harness failed: five runs, never once exited. Which scenes moved is read
   off `render.snap`'s staged diff — a change to a shared helper moves
   screens its code diff never names, but never one its snapshot diff does
   not. It was a `--focus` the author wrote until 2026-09-27.

6. **A judge passes with no findings, and that is the whole verdict.** Two
   runs will not produce the same findings; the findings are the output. The
   score this replaced had a threshold of 100 — the developer's, 2026-09-24
   — so it only ever compared the findings against none.

7. **Design contradictions live in `crates/review/baseline.json`.** The
   reference disagrees with itself in places, and stage 4 cannot run against
   it without somewhere to record where. Each entry states both halves — two
   things the design says, or the design against Apple's HIG, each quoted —
   and which half the app follows, and is removed when the design is fixed
   upstream or the decision reversed — it is a bug list for the design
   system, not a compensation layer for the app.

8. **Everything that reads declared cells is a hermetic test in
   `crates/tui`, not a capture.** Palette membership, the closed glyph table
   and the copy rules all ran through the review harness's real terminal
   until 2026-09-20 — a compositor, a subprocess and 2m45s per run, none of
   it reproducible. A `TestBackend` buffer holds the same declared cells, so
   they moved and now cost under two seconds. What a real terminal uniquely
   gives is a picture, and pictures are stage 8's.

9. **The copy and glyph rules are scoped to app-owned rows.** They govern
   Aldwin's own copy, not what it echoes: the transcript renders a model's
   reply verbatim, and an em dash or a contraction there is ordinary. A
   global buffer-level check would fail on real use, which is a check that is
   wrong rather than strict. `app_owned_rows` makes the split by scene and
   band and states what it misses.

10. **The toolchain is recorded, not pinned.** `rust-toolchain.toml` is read
    by rustup and this machine installs Rust from pacman, so there is nothing
    to pin against. Recording the version and failing when it moves is not
    hermeticity — it is the honest substitute, and it turns "clippy suddenly
    fails on untouched code" from a mystery into a line in the report.

11. **The report is plain HTML and stays that way.** `review.html` is
    self-contained — no stylesheet, no script, no embedded frames — and does
    **not** apply the design system this loop enforces. Dressing the referee
    in the players' kit makes it harder to trust. The binary writes only what
    was measured and leaves a marked placeholder per required judge; `judge`
    writes each verdict, verbatim, because the agent that made the change is
    the one that would otherwise write the verdict sentence.

12. **A review is not complete until every required judge is written, and
    the exit code says so.** `review` exits non-zero after a clean stages 1–5
    whenever a judge is required, because that is not a review — only the
    `judge` that completes the run exits zero, and it is what writes the pass
    record. `--stages-only` is the explicit opt-out for the fast check during
    development, and records nothing.

    This is the fourth fix for the same failure and the first one aimed at
    the cause. Stage 5's section came back empty on **five of seven runs**.
    The first three fixes — providing a command, making that command exit
    non-zero, printing the invocation — all addressed *recall*, and recall
    was never the problem. The problem was that nothing depended on it:
    `review` printed "clean" and exited zero at a point where the work was
    half done, and a step nothing depends on is a step that gets skipped
    under attention pressure. Compare stage 3, which has never been skipped
    once, because skipping it fails the next run.

13. **Each judge emits its own JSON, and it goes into the report verbatim.**
    Transcribing prose findings into the report's schema by hand was tedious
    enough to be where the step died — and it routed the judge's conclusions
    through the hands of the agent whose work was being judged. The prompt
    now asks for exactly one fenced `json` block and nothing else.

14. **The `rust` skill is enforced twice, and the split is by whether a
    machine can check it.** What a lint can check is a workspace lint and
    fails stage 2: public-API docs, `# Errors` and `# Panics` sections,
    `Debug` on public types. What it cannot — iterator chains over loops,
    `map_err` only where `From` cannot convert, grouped imports, doc
    examples, a fix with its test — is stage 7's, and a finding stage 7
    makes twice graduates to a lint or a test. Enabling the lints cost 608
    fixes, made in the commit that enabled them.

15. **The pass is recorded against the staged tree, and the commit checks
    it.** `judge` writes the record into the repository's git directory,
    `aldwin-review/<tree>.json`, when the last required judge passes and the
    index still is the tree the run reviewed. `gate` — stage 10 — refuses a
    commit whose tree has no passing record. Keyed by tree because the
    commit does not exist yet when the hook runs, and because any edit after
    the review changes the tree, so a record cannot be carried over to code
    it did not see. A review requires the working tree to equal the index:
    it builds and judges one and records the other, so they have to be the
    same tree. A change that calls for no judge — docs only — is recorded by
    `review` itself once stages 1–5 pass.

16. **An agent's commit is gated; the developer's is not.** Claude Code sets
    `CLAUDECODE` in every shell it starts, and `.githooks/pre-commit` checks
    for it. The hooks directory is set by a `SessionStart` hook; a
    `PreToolUse` guard refuses `--no-verify`, `-n`, anything naming the
    hooks setting or the record directory, and the git commands that write
    commits without `pre-commit` (`commit-tree`, `cherry-pick`, `revert`,
    `rebase`, `am`). A merge commit runs `pre-merge-commit`, which is the
    same gate. Edits to the settings, the hooks and the record are denied.

## Pitfalls

- **Letting the contradictions list grow.** It is seven entries, each
  accounted for in Progress below. A previous version of this idea reached
  fourteen and then needed its own admission rule, at which point it had
  become the thing it was built to prevent.
- **Running a judge on a failing stage 1–5.** A judge looking at frames drawn
  with a drifted palette, or at code that does not build, reports a
  consequence as a cause.
- **Inferring stage 8's scenes from the code diff.** A change to a shared
  helper touches screens its code diff never names; the snapshot diff is
  what names them.
- **Asking the author what the change is for.** A goal written by the agent
  being judged is a rubric it can always pass. Nothing the author writes is
  an input to its own review; staging is the only statement it makes.
- **Regenerating a snapshot to make stage 5 pass.** The regeneration is the
  deliberate act; reading the diff first is what makes it one.
- **Reading a clean stage 5 as a correct UI.** Its baseline half proves the
  frames did not change, and its conformance half proves every cell came from
  the design. Neither says a band is in the right place — nothing mechanical
  here does, because the design ships no reference frame to compare against.
- **Putting a check that needs a real terminal into stages 1–5.** They are
  hermetic and the value of that is the whole point; anything needing a
  compositor belongs after them, feeding stage 8.
- **Treating the gate as tamper-proof.** It makes skipping the loop a
  deliberate act, never an oversight. An agent that edits the settings
  through a shell, or deletes `.githooks/`, gets past it; the permission
  denials make that harder, not impossible.
- **Matching the guard on anything but commands.** The `PreToolUse` guard
  reads a Bash command's text, so a script that merely *mentions* the hooks
  setting or the record directory is refused too. Write such text with the
  file tools; do not loosen the guard to let a shell do it.

## Progress (2026-09-27, the commit gate)

The loop became ten stages and a commit depends on it. Before this it ran
when someone remembered to run `/review`, and nothing stopped an agent from
stopping at "the code is written".

- **The goal and the focus are gone.** Both were written by the agent being
  judged: a vague goal cannot fail, and a focus that missed a screen hid it.
  The "blocking" severity went with the goal — "did not do what it set out
  to do" needs a statement of intent, and whether the change is what the
  developer wanted is the developer's call. Stage 8's scenes now come from
  the snapshot diff.
- **Stage 5 became three judges.** Code (stage 6) and Rust (stage 7) read
  the staged diff; frames (stage 8) is the old stage 5. One subagent each
  per iteration, fresh every time, run in parallel; a verdict passes with no
  findings. `stage5` is now `judge --stage 6|7|8`.
- **Stages renumbered** 0–4 → 1–5, so the list reads 1 to 10. Earlier
  Progress entries keep the old numbers.
- **The commit gate** (Decisions 15 and 16), and with it the rule that a
  review is of the staged tree and nothing else.
- **The `rust` skill's lints** (Decision 14). Its examples were replaced the
  same day: four came from another project, and a judge citing them could
  have taken `NoteError` for a type in this workspace.
- **The loop's first run was on itself.** By its third iteration two of the
  judges' findings were the sources disagreeing, and the developer settled
  both rather than spend the remaining passes on them:
  - *"Include examples in doc comments"* is scoped to public functions a
    change adds. Unscoped, the lint backlog's ~70 new docs on existing items
    each needed one, against the same skill's "keep changes minimal".
  - *quality-gate §6 over §7*: `aldwin-review` reported every failure as an
    `io::Error` with a sentence, and §7's "same error shape as the crate"
    let new code keep doing so against §6's typed errors. The crate now has
    one `thiserror` enum, `aldwin_review::Error`: every `io::Error` built
    from a sentence and `keys::parse`'s string error became one of its
    variants, so the two sections agree again. `tokens::check` keeps an inner
    `Result<usize, String>` on purpose — a stale file is the stage's answer,
    not a failure to run it.
- **Known gaps, stated rather than hidden.** Seven snapshot scenes have no
  capture script (`answering`, `commented`, `running`, `selecting`,
  `stopping`, `working`, `wrapped`), so a change that moves only those has
  no frames judge, and the report says why. Three capture scenes
  (`launch_unconfigured`, `plan`, `resume`) have no snapshot, so no change
  can call for a judge of them. Aldwin's own `run` tool does not set
  `CLAUDECODE`; a commit made through Aldwin is not gated. All three are
  open-tasks 35–37.

## Progress (2026-09-24, the audit)

An audit of the loop found two ways it could pass a run it should have
failed, and both are closed with a test that would have caught them.

- **`UPDATE_SNAPSHOTS` leaked into the stages.** `stages::cargo` inherited
  the developer's environment, so a shell that still exported the switch
  from one deliberate regeneration had stage 4 rewrite `render.snap` and
  pass against its own output. Every `cargo` the loop runs now has it
  removed.
- **Stage 5 could be written over a failing run.** The report always left
  the placeholder, so `stage5` would put a judge's 100 beside a failed
  suite, or beside a run that captured no frames. The placeholder is now
  written only when stages 0–4 all passed and frames exist
  (`report::Run::reaches_stage5`), and `review` prints the `stage5` command
  only then. Decision 12 made the exit code say a review is incomplete;
  this makes the report unable to say it is complete when it is not.
- **The loop ran a stale binary.** The skill said `cargo build &&
  ./target/release/aldwin-review`, which builds debug and runs whatever
  release binary was last built. Every invocation is `cargo run --release
  -p aldwin-review --` now, in the skill and in the hint `review` prints.
- **`cargo fmt --check` is back in stage 1** (open-tasks 3): the developer
  chose stable rustfmt and `516dd63` reformatted the workspace — including
  the generated `tokens.rs`, which stage 3 then reported stale. The
  generator now emits through `rustfmt`, as bindgen and prost do, so stage 1
  and stage 3 agree about the same file.
- **The generator stopped guessing.** A light value it could not read
  fell back to the dark one, and a missing light scope made the light
  theme the dark one; both are errors now. A colour outside sRGB is
  clipped only within CSS Color 4's just-noticeable difference (ΔE OK
  0.02 — `--del` needs it), and refused beyond. The baseline is an explicit
  input to `generate` rather than a file it read on the side.

**The contradictions, all seven:**

- `no-table-component-adr-0002`, `frame-command-list-is-not-the-products`
  and `frame-j-offers-undo` — the three the redesign kept (2026-09-23).
- `field-has-no-placeholder` — the developer's no-hint decision
  (`3a2bffe`), against frames A–D and J. The audit added the HIG's half:
  "Show hints in text fields."
- `question-panel-insets-are-untokenised` — from the whole-app pass below.
  It is a gap in the token layer rather than two statements that disagree,
  and it stays because the alternative is two unexplained literals in the
  app; it leaves when `layout.css` names the insets.
- `label3-is-below-the-hig-contrast-minimum` — `--label3` is 2.5:1 on
  `--win` (dark) and 2.9:1 (light) against the HIG's 4.5:1, and the design
  marks a not-ready approve by colour alone. The app keeps `label3` (the
  developer's decision) and gives the not-ready approve words as well.
- `long-diff-lines-wrap` — "the code is never broken up" against the HIG's
  "containers may need to grow in height so that text isn't cropped". The
  app follows the HIG: a long diff line wraps.

## Progress (2026-09-24, the threshold is 100)

Stage 5's threshold went from 90 to 95 and then to 100, the developer's
call, made during a whole-app pass (every scene in `--focus`, not one
change's). It is one constant, `report::THRESHOLD`, that `stage5` and the
report both read.

- **What 100 means.** No finding of any severity. At 90 the loop tolerated
  two minors, and the slack was deliberate: "the score is a threshold, not
  a measurement" (Decision 6), and stage 5 is not reproducible — two
  judges on the same frames report different minors. At 100 one run-to-run
  minor fails the loop. That was accepted knowingly; the pass that raised
  it reached 100 on its fourth iteration.
- **The consequence to watch** is `baseline.json`. The fastest way to 100 is
  to record a finding as a contradiction, and the list is a bug list for
  the design, not an escape hatch: an entry must name the design saying
  two things. The pass added one (`question-panel-insets-are-untokenised`),
  for a gap in the token layer rather than a disagreement in the app.
- **Severity is read once.** The score and the report's counts both go
  through `report::severity`, case-insensitively; before, a `Minor` was
  deducted but not counted.

## Progress (2026-09-23, the redesign)

The loop's shape is unchanged; what it measures moved with the design.

- **Stage 3** reads `tokens/colors.css`, `tokens/layout.css`,
  `guidelines/glyphs.html` and the frame. The generator converts OKLCH to
  sRGB itself (there is no hex table to look values up in), mixes the brand
  mark's 108 cells and the context bar's ramp table in OKLCH the way CSS
  `color-mix` does, and checks its gauge arithmetic against the two bars
  the frame draws. Ten roles are uncarried with a reason each; the check
  that every declared role is carried or excused is unchanged.
- **Stage 4** pins thirteen scenes at 80×24, 104×32 and 200×50 — the
  medium size is now a terminal the size of the design's window body — and
  asserts, beyond the snapshot: every colour is a token (or a mark or
  gauge mix), every glyph is in the closed table, no stroke anywhere, the
  three-cell margins on every conversation scene, and the two hue rules
  (the agent's prose is never blue; nothing outside a diff is red or
  green).
- **The scene catalogue** (`scene.rs`) is re-scripted: `launch`,
  `launch_unconfigured`, `plan`, `details`, `question`, `commands`,
  `review`, `saved`, `markdown`, `failure`, `long`, `resume`. Grants are
  gone from `Script`; `permissions.yaml` is not seeded at all. The `review`
  scene is the real dispatcher opening the real review over a really
  staged edit, which is the state the snapshot cannot reach. Three snapshot
  scenes have no counterpart here and say why (open-tasks 30, 31).
- **`baseline.json`** went from seven contradictions to three: ADR 0002's
  table, and two that are the design against a decision.
- **Capture and the caret.** The design's caret blinks (`motion.css`, 1.05s
  stepped), so an app at rest on a field is never finally quiet: the first
  full capture after the redesign failed on its first scene with the parser
  and the picture disagreeing at exactly one cell — the caret, on in one
  and off in the other. `wait_quiet` still settles between blinks; what
  changed is that the shot and the grid are now taken inside one
  half-period (`Proxy::wait_for_change` waits for the edge first), and the
  grid is read back after the shot to prove it — a mismatch retakes on the
  next edge, three times before giving up. A screen with no caret falls
  through the wait and costs 1.3s.
- **The shown half, always** (2026-09-24). The blink is timed from launch,
  so a scene at rest since then lands on the same phase every run: the
  launch and commands frames were always the hidden half, and once the
  field's placeholder was removed a judge reported the launch field as
  having no caret, blocking. `caret_hidden` compares the grid either side
  of an edge — the cells whose ground changed are the caret, and hidden,
  a caret cell is the ground of its left neighbour — and capture skips
  that half-period. Five half-periods before giving up, not three shots.
  The same run found the commands field's `/` unpadded, putting its caret
  and filter on cell 4 rather than the body column.

## Progress (2026-09-20, the rebuild)

This spec replaces `mjolnir-screenshot.md`. What it replaced had grown to
5,731 lines of harness and 3,726 lines of governing documents to check a
7,400-line TUI against a 444-line design reference — apparatus twenty-one
times the size of the thing it enforced, and the ratio was the problem rather
than a symptom of one.

The cause was a single mistake made repeatedly: trying to mechanise "does this
look like the design" against a prose reference that contradicts itself in
fourteen places. Each layer built to cope with that ambiguity became something
else to interpret. Blind judges were added because the question had no
mechanical answer; a three-way classification because their findings arrived
undifferentiated; an errata file because they re-derived the same
contradictions every run; twenty baseline exemptions because the gates fired
on design debt; an assertion suite because the judges were uncalibrated. The
errata's own admission rule then structurally excluded the most re-raised
finding in the catalogue's history.

Deleted: the conformance catalogue (1,572 lines), the errata (149), the
assertion suite (596), the contrast gate and its floors (182), the facts
emitter (245), the region map (178), the acceptance model and its HTML report
(354), the regression focus-set machinery (164), and the twenty exemptions.

Kept: the capture stack — compositor, pty, proxy, vt, png, scenes — which is
the genuinely hard part and which works. Added: token generation, and a slim
cell check with nowhere to put a judgement.

**Found on the first clean run**, by the copy lint that survived the cut: the
permission panel's elision row read `1 more line not shown; deciding doesn't
require scrolling them` — a contraction the design's copy never uses, and a
plural pronoun for a count of one, on every 80×24 frame. It now reads
`1 more line not shown`, which is the shape of the design's own elision row.

## References

- .claude/skills/review/SKILL.md — the loop, and the three judges' prompts.
- crates/review/src/tokens.rs — stage 4, and the roles it does not carry.
- crates/tui/tests/render_snapshot.rs — stage 5's baseline.
- crates/review/baseline.json — the design's own contradictions.
- .claude/design/IMPORT.md — the reference, and its provenance.
