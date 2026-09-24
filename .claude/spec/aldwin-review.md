# aldwin-review

The feedback loop that runs after a change to Aldwin is ready for submission.

**Status:** active — built and in use. Replaced `mjolnir-screenshot` on
2026-09-20; see Progress.
**Scope:** the five-stage review loop — the crate `crates/review`
(`aldwin-review`) that runs the four deterministic stages, and the `review`
skill that drives the loop and owns the fifth. Excludes what the stages
themselves test (that is each crate's own spec) and the design system's
content.
**Owner:** Maximilian
**Last Updated:** 2026-09-24

## Why

A change to `crates/tui` needs two questions answered before it ships: did it
break anything, and does it do what it set out to do. The first is mechanical
and the second is not, and the whole design of this loop is keeping them
apart.

Four stages are deterministic: they run a command, compare against something
committed, and say yes or no. One is a subagent looking at pictures. Nothing
in the deterministic four makes a judgement about whether the UI *looks like*
the design, because the previous harness tried exactly that and the attempt is
what this spec replaces.

## The stages

| stage | answers | how | hermetic |
| --- | --- | --- | --- |
| 0 toolchain | are these results comparable to the last run's | `rustc --version` against the baseline | yes |
| 1 lint | does it build clean | `cargo clippy --workspace --all-targets -- -D warnings` | yes |
| 2 test | does the suite pass | `cargo test --workspace` | yes |
| 3 tokens | is the app's design system still the imported one | regenerate `crates/tui/src/tokens.rs` and diff | yes |
| 4 frames | do the frames match the baseline, and does every cell come from the design | `render_snapshot.rs` against `tests/snapshots/render.snap`, plus colour, glyph and copy conformance | yes |
| 5 confidence | does it match the designs, and did it do what it set out to do | a blind subagent, scored 0–100, threshold 100 | no, and cannot be |

Stages 0–4 take about fifteen seconds and hold no clock, no network, no
subprocess of the app and no compositor. Capture is not a stage: it runs after
them to make the pictures stage 5 looks at, and its non-determinism is
harmless there because a bad frame is something the judge says out loud.

## Decisions

1. **One screenshot baseline, not two.** `render.snap` serialises every
   cell's symbol, foreground, background and modifiers for thirteen scenes at
   three sizes in both themes — 72 sections — in under a second, in-process.
   Capturing the same frames through a real terminal and diffing those too
   would be a second fixture asserting the same thing on a slower clock. The
   real terminal earns its place by producing **pictures for stage 5**, which
   is the one thing `TestBackend` cannot do.

2. **The app's design system is generated, not transcribed.**
   `crates/tui/src/tokens.rs` is emitted from `.claude/design/tokens/*.css`
   and committed; stage 3 regenerates it and fails on any diff. Before this,
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
   whether the app *should* consume one is stage 5's question.

5. **Stage 5 judges only the screens the change touched.** The app has
   deviations that cannot be fixed in `crates/tui` — there is no branch in
   `StatusInfo`, no clock in the workspace, and the design has no
   edit-approval screen. A judge assessing the whole app reports those every
   run and the loop never terminates, which is precisely how the previous
   harness failed: five runs, never once exited.

6. **The score is a threshold, not a measurement.** Two runs will not produce
   the same number. The findings are the output.

7. **Design contradictions live in `crates/review/baseline.json`.** The
   reference disagrees with itself in places, and stage 3 cannot run against
   it without somewhere to record where. Each entry states both halves of what
   the design says and which half the app follows, and is removed when the
   design is fixed upstream — it is a bug list for the design system, not a
   compensation layer for the app.

8. **Everything that reads declared cells is a hermetic test in
   `crates/tui`, not a capture.** Palette membership, the closed glyph table
   and the copy rules all ran through the review harness's real terminal
   until 2026-09-20 — a compositor, a subprocess and 2m45s per run, none of
   it reproducible. A `TestBackend` buffer holds the same declared cells, so
   they moved and now cost under two seconds. What a real terminal uniquely
   gives is a picture, and pictures are stage 5's.

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
    was measured and leaves a marked placeholder; the skill appends the
    judge's section, because the agent that made the change is the one that
    would otherwise write the verdict sentence.

12. **A review is not complete until stage 5 is written, and the exit code
    says so.** `review` exits non-zero after a clean stages 0–4, because that
    is not a review — only `stage5` can exit zero. `--stages-only` is the
    explicit opt-out for the fast check during development.

    This is the fourth fix for the same failure and the first one aimed at
    the cause. Stage 5's section came back empty on **five of seven runs**.
    The first three fixes — providing a command, making that command exit
    non-zero, printing the invocation — all addressed *recall*, and recall
    was never the problem. The problem was that nothing depended on it:
    `review` printed "clean" and exited zero at a point where the work was
    half done, and a step nothing depends on is a step that gets skipped
    under attention pressure. Compare stage 3, which has never been skipped
    once, because skipping it fails the next run.

13. **The judge emits its own JSON, and it goes into the report verbatim.**
    Transcribing prose findings into the report's schema by hand was tedious
    enough to be where the step died — and it routed the judge's conclusions
    through the hands of the agent whose work was being judged. The prompt
    now asks for exactly one fenced `json` block and nothing else.

## Pitfalls

- **Letting the contradictions list grow.** It is two entries. A previous
  version of this idea reached fourteen and then needed its own admission
  rule, at which point it had become the thing it was built to prevent.
- **Running stage 5 on a failing stage 1–4.** A judge looking at frames drawn
  with a drifted palette reports a consequence as a cause.
- **Inferring the focus from the diff.** A change to a shared helper touches
  screens its diff never names.
- **Regenerating a snapshot to make stage 4 pass.** The regeneration is the
  deliberate act; reading the diff first is what makes it one.
- **Reading a clean stage 4 as a correct UI.** Its baseline half proves the
  frames did not change, and its conformance half proves every cell came from
  the design. Neither says a band is in the right place — nothing mechanical
  here does, because the design ships no reference frame to compare against.
- **Putting a check that needs a real terminal into stages 0–4.** They are
  hermetic and the value of that is the whole point; anything needing a
  compositor belongs after them, feeding stage 5.

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

- .claude/skills/review/SKILL.md — the loop, and stage 5's prompt.
- crates/review/src/tokens.rs — stage 3, and the roles it does not carry.
- crates/review/src/cells.rs — stage 3's cell half, and why it has no
  judgement in it.
- crates/tui/tests/render_snapshot.rs — stage 4's baseline.
- crates/review/baseline.json — the design's own contradictions.
- .claude/design/IMPORT.md — the reference, and its provenance.
