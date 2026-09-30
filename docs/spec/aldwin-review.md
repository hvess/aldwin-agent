# aldwin-review

The feedback loop every agent commit to Aldwin runs, and cannot land without.

**Status:** active — built and in use. Replaced `mjolnir-screenshot` on
2026-09-20; became ten stages with a commit gate on 2026-09-27. See
Progress.
**Scope:** the ten-stage review loop — the crate `crates/review`
(`aldwin-review`) that runs the five deterministic stages, decides which
judges a change needs, writes their verdicts and keeps the pass record; the
`review` skill that drives the loop and owns the three judges; and the hooks
that make an agent's commit depend on it (`.githooks/`). Excludes what the
stages themselves test (that is
each crate's own spec) and the design system's content.
**Owner:** Maximilian
**Last Updated:** 2026-09-30

## Why

A change needs two questions answered before it lands: did it break
anything, and does it hold to what this project has decided — its
architecture, its Rust, its design. The first is mechanical and the second
mostly is not, and the whole design of this loop is keeping them apart.

Five stages are deterministic: they run a command, compare against something
committed, and say yes or no. Three are judges — subagents, each judge
reading one thing against one set of sources, the code judge with two
readers. Nothing in the deterministic five makes a
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
| 2 lint | is it formatted, does it build clean, and does it keep the checkable rules of the `rust`, `big-o` and `data-structures` skills | `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets -- -D warnings` with the workspace lints (`missing_docs`, `missing_debug_implementations`, `clippy::missing_errors_doc`, `clippy::missing_panics_doc`; `clippy::linkedlist`, `stable_sort_primitive`, `large_stack_arrays`, `inefficient_to_string`) | yes |
| 3 test | does the suite pass | `cargo test --workspace` | yes |
| 4 tokens | is the app's design system still the imported one | regenerate `crates/tui/src/tokens.rs` and diff | yes |
| 5 frames | do the frames match the baseline, and does every cell come from the design | `render_snapshot.rs` against `tests/snapshots/render.snap`, plus colour, glyph and copy conformance | yes |
| 6 code judge | does the diff hold to `quality-gate` (with the `comments`, `big-o` and `data-structures` skills it names), the Key Constraints and the ADRs, and do the system prompt and tool descriptions still describe the code | two blind subagents over `change.diff`, their findings merged into one verdict; runs when a crate, the workspace manifest or the gate's own hooks (`.githooks/`) changed | no |
| 7 Rust judge | does the diff hold to the `rust` skill's rules no lint checks | a blind subagent over `change.diff`; runs when Rust source changed | no |
| 8 frames judge | do the changed scenes look like the design | a blind subagent over the captured frames of the scenes whose snapshot changed | no |
| 9 iterate | — | any failure: fix, run again from stage 1, fresh judges for what changed; at most five passes | — |
| 10 gate | was exactly this tree reviewed, and did it pass | `aldwin-review gate`, run by `.githooks/pre-commit` for an agent's commit | yes |

Stages 1–5 take about fifteen seconds and hold no clock, no network, no
subprocess of the app and no compositor. Capture is not a stage: it runs
after them, only for stage 8's scenes, and its non-determinism is harmless
there because a bad frame is something the judge says out loud.

Which judges run is read off the staged diff, never chosen, and the report
states each one's reason. No judge runs until stages 1–5 pass (Decision
18); the frames judge also needs the captured frames. A judge whose inputs are unchanged since it passed keeps
that pass (Decision 17). Every snapshot scene is also a capture scene —
`scene.rs` has a test that fails if the two lists drift — so a change that
moves any scene's snapshot has frames for stage 8 to judge.

## Decisions

1. **One screenshot baseline, not two.** `render.snap` serialises every
   cell's symbol, foreground, background and modifiers for twenty-three
   scenes at three sizes in both themes — 138 sections — in under a second,
   in-process.
   Capturing the same frames through a real terminal and diffing those too
   would be a second fixture asserting the same thing on a slower clock. The
   real terminal earns its place by producing **pictures for stage 8**, which
   is the one thing `TestBackend` cannot do.

2. **The app's design system is generated, not transcribed.**
   `crates/tui/src/tokens.rs` is emitted from `docs/design/tokens/*.css`
   (and, for what only they state, the glyph card and the frame) and
   committed; stage 4 regenerates it and fails on any diff. Before this,
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
    was measured and leaves a marked placeholder per judge that is pending
    and can run in this run (a carried judge gets a note naming the run it
    passed in, and only a pass written there carries); `judge` writes each
    verdict, verbatim, because the agent that made the change is
    the one that would otherwise write the verdict sentence.

12. **A review is not complete until every required judge has passed in
    it or is carried into it, and the exit code says so.** `review` exits non-zero after a clean stages 1–5
    whenever a judge is left to run, because that is not a review — only the
    `judge` that completes the run exits zero, and it is what writes the pass
    record. When no judge is left — none called for, or each one carried
    (Decision 17) — `review` records the pass itself. `--stages-only` is the explicit opt-out for the fast check during
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
    `map_err` only where `From` cannot convert, grouped imports, no doc
    examples (since 2026-09-29; before, an example on each new public
    function), a fix with its test — is stage 7's, and a finding stage 7
    makes twice graduates to a lint or a test. Enabling the lints cost 608
    fixes, made in the commit that enabled them. The `big-o` and
   `data-structures` skills split the same way (Progress, 2026-09-29, cost
   and structure): four lints in stage 2, the rest in stage 6.

15. **The pass is recorded against the staged tree, and the commit checks
    it.** `judge` writes the record into the repository's git directory,
    `aldwin-review/<tree>.json`, when the last required judge passes and the
    index still is the tree the run reviewed. `gate` — stage 10 — refuses a
    commit whose tree has no passing record. Keyed by tree because the
    commit does not exist yet when the hook runs, and because any edit after
    the review changes the tree, so a record cannot be carried over to code
    it did not see. (A judge's *verdict* can be, when every byte that judge
    reads is identical — Decision 17 — and the record is still of this
    tree.) A review requires the working tree to equal the index:
    it builds and judges one and records the other, so they have to be the
    same tree. A change with no judge left to run — docs only, or every
    judge carried — is recorded by `review` itself once stages 1–5 pass.

16. *Since 2026-09-27 (the developer's call, ahead of open-sourcing) the
    repository ships no agent-specific configuration: Claude Code's
    project settings, the `SessionStart` hook and the `PreToolUse` guard
    described below live on the developer's machine only, untracked, and
    the guard's test left with them. What the repository enforces is
    `.githooks/` alone, turned on by pointing git's hooks setting at it
    (`AGENTS.md`). The rest of this Decision describes that local setup as
    it stands on the developer's machine.*
    **An agent's commit is gated; the developer's is not.** An agent is
    anything that sets `AGENT` (or Claude Code's own `CLAUDECODE`, below):
    Claude Code through the project settings'
    `env`, Aldwin in every process it starts (`sandbox::command`), and
    `.githooks/pre-commit` checks for it. It was `CLAUDECODE` until
    2026-09-27, which Aldwin never set, so Aldwin's own commits passed
    ungated. `CLAUDECODE` still counts beside it, as a backstop: Claude Code
    sets it whatever its settings, so a session started without the
    project's settings is gated too (the developer's call, 2026-09-27). Any value of `AGENT` counts: a developer whose own shell exports
    it for another tool has their commits gated too, and sees the gate's
    sentence say so — accepted as rare and visible (the developer's call).
    The guard below is a Claude Code hook and binds Claude Code only: an
    Aldwin session's commits meet the gate, but nothing stops it from
    passing `--no-verify`. Guarding Aldwin would mean Aldwin refusing git
    flags in users' own projects, which they must never be forced into
    (the developer's call, 2026-09-27).
    The hooks directory is set by a `SessionStart` hook; a
    `PreToolUse` guard refuses `--no-verify` and `-n` on `commit` and
    `--no-verify` on `merge` and `pull` (and the abbreviations git takes
    for it), anything that names `AGENT` other than to read it (`$AGENT`,
    `${AGENT}`), `env -i` and `exec -c`, anything naming the hooks setting
    (in any case, as git reads it) or the record directory, a git alias
    whether defined with `-c` or saved with `git config`, and the git
    commands that write commits without `pre-commit` (`commit-tree`,
    `cherry-pick`, `revert`, `rebase`, `am`). Each is matched with the
    command's quotes taken out and its backslash-newlines joined, as the
    shell does. A merge commit runs
    `pre-merge-commit`, which is the same gate. Edits to the settings, the
    hooks and the record are denied.
    **What the guard is for, and what it is not.** It refuses the plain
    spellings an agent reaches for by habit — the ways a commit skips the
    gate by accident. It is not a parser of every shell a command could be
    written in, and it is not a finding when it misses a spelling built to
    get past it: brace expansion around a name, a quote split inside an
    option, an `include.path` that loads the hooks setting from a file. A
    command like that is the deliberate act the gate exists to make visible,
    and whoever wrote it chose to skip the loop. Iterations 5, 7 and 8 of
    the loop's first run each failed on new spellings of that kind; this
    paragraph is the answer, so a judge is not argued with a fourth time
    (the developer's call, 2026-09-27).

17. **A pass is kept for as long as what it judged is unchanged; a pass is
    one round, not one reading.** Three rules, from the first change the
    loop ran on (2026-09-27, twenty passes):
    - *A judge's pass carries.* Each judge reads part of the staged diff
      (`Judge::reads`: the code judge all of it; the Rust judge the Rust,
      its skill and the review skill that holds its prompt; the frames
      judge the snapshot, the review crate that captures them, the design,
      the ADRs and the review skill), fingerprinted with `HEAD` in git's
      own hash (`git::Fingerprint`), so a commit voids every carried pass;
      a one-theme capture is not fingerprinted, so a frames pass never
      carries into or out of one.
      A judge whose fingerprint matches a pass in one of the last three
      runs (the ones kept) keeps that
      pass, named in the report, and is not spawned; a carried frames judge
      skips capture. Only a pass carries: a finding is always re-read. On
      that change most passes changed only docs, and the Rust and frames
      judges re-read identical code and frames each time.
    - *The code judge has two readers* (`Judge::readers`). On a large diff
      each fresh reader found one or two different minors, so one reader
      cost a pass per finding; two readers' findings are merged into one
      verdict, and `judge --stage 6` refuses anything but two files.
    - *A judge waits only for what it reads.* The code and Rust judges
      need the workspace to build; a failing fmt, test, token or frame
      check no longer holds them back, and their findings are fixed in the
      same round. The frames judge still waits for a clean 1–5.
      (Superseded by Decision 18: every judge waits for a clean 1–5.)
    This makes a run depend on earlier runs in `target/review-frames/`, not
    only on the staged tree: a carried verdict is as sound as the
    fingerprint that keys it, and the pass record is still of the tree.

18. **No judge runs until stages 1–5 pass, and the author checks before
    the judges do.** The developer's call, 2026-09-27. `Run::reaches`
    leaves a placeholder only on a clean 1–5, so `review` prints no judge
    command and `judge` has nowhere to write on a tree that fails a stage.
    The skill's author's loop comes first: `review --stages-only` and the
    author's pass (quality-gate, a grep of what the diff removes across
    `crates/`, `docs/`, `.agents/` and `AGENTS.md`, a test at the producing layer) are repeated
    until clean, and only then is the tree staged for the judges. A
    finding is fixed with its siblings, not alone.

## Pitfalls

- **Clearing `GIT_DIR` or `GIT_INDEX_FILE` from the crate's git.** Inside
  the pre-commit hook they name the repository (a worktree's own included)
  and the index the commit records, `git commit -a`'s temporary one
  included, so `git::staged_tree` and `gate::write_record` must inherit
  them. The cost: the gate's tests, which run them on a throwaway
  repository, would read the hook's if `cargo test` ran inside a hook. The
  loop runs tests from `review`, never from a hook; the fixture's own git
  clears both.
- **Letting the contradictions list grow.** It is nine entries, each
  accounted for in Progress below. A previous version of this idea reached
  fourteen and then needed its own admission rule, at which point it had
  become the thing it was built to prevent.
- **Running a judge on what it cannot read soundly.** A judge looking at
  frames drawn with a drifted palette, or at code that does not build,
  reports a consequence as a cause — one reason no judge runs until stages
  1–5 pass (Decision 18; Decision 17 had each wait only on its own inputs).
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
  deliberate act, never an oversight; no list of refusals here is claimed
  complete (Decision 16). An agent that deletes `.githooks/`, or points
  git's hooks setting away from it, gets past it; in the repository nothing
  refuses that, and on the developer's machine the local guard and
  permission denials (Decision 16's note) make it harder, not impossible.
- **Expecting the guard to tell data from commands.** This concerns the
  guard on the developer's machine (Decision 16's note). It reads flags from the
  command's words, so a commit *message* that mentions `-n` passes — but it
  still refuses a Bash command that merely *mentions* the hooks setting,
  the record directory or `alias.`, or that names `AGENT` (the gate's
  variable) other than as an expansion — a commit message included — and it reads every line of a heredoc as a command, since it
  cannot know what the heredoc feeds. Write such text with the file tools;
  do not loosen the guard to let a shell do it.

## Progress (2026-09-29, cost and structure)

The developer's call: the review checks time and space complexity and the
choice of data structure, against two new skills, `big-o` and
`data-structures`, which quality-gate §5 names. The split is Decision 14's.

- **Stage 2 gets what a lint can tell apart:** `clippy::linkedlist`,
  `stable_sort_primitive`, `large_stack_arrays` and
  `inefficient_to_string`, none of which fired; Clippy's default `perf`
  lints were already in it. A trial of about forty lints left the
  rest out: `format_collect`'s thirteen hits were test fixtures and a
  32-byte hash, where the big-o skill asks for the plainest code;
  `large_futures`' one hit sits under `#[async_trait]`, which already
  boxes; and nursery lints (`needless_collect`, `redundant_clone`,
  `set_contains_or_insert`, `large_stack_frames`) are Clippy's own
  admission of false positives, which would fail correct code.
- **Stage 6 gets the rest:** an O(n²) search, `remove(0)` in a loop,
  per-frame work over the whole transcript, recursion as deep as its
  input, a structure that serves the wrong operation. Each looks like its
  harmless twin until n's bound and the code's frequency are known — the
  removed-identifier entry's exception, a finding no machine can tell from
  its lookalikes. A cost finding names n, why it is unbounded and how
  often the code runs; a small bounded n is not one. `Judge::reads` is
  unchanged: the code judge reads the whole diff, so a skill edit voids its
  carried pass.

## Progress (2026-09-30, frame P)

The frame re-synced with frame `P plan card` (aldwin-tui.md, same date),
and scene `drafting` joins both lists: two edits staged and the turn held,
so the plan is docked. `tokens.rs` regenerates unchanged. `drafting` stays
out of `the_agents_prose_is_never_blue_and_nothing_outside_a_diff_is_red`,
since its counts are green and red (baseline
`staged-counts-are-green-and-red`). Baseline `a-plan-step-has-no-note` is
now `a-plan-step-note-is-the-cards`: the developer reversed the 2026-09-27
call for the card.

## Progress (2026-09-29, frame K)

The frame re-synced with frame `K queued` (aldwin-tui.md, same date), and
scene `queued` joins both lists. Frame K draws its context bar in a
spelling stage 4 could not read: the full segment is `var(--fill)`, not a
100% mix, and no monospace span wraps the bar. `frame_gauge` now reads each
segment span's colour in either spelling, from the span holding the
window's first `var(--track)` to the printed percentage. A bar it cannot
read fails the stage (`a_context_bar_that_cannot_be_read_fails_the_check`).
Before, such a bar was skipped, and nothing checked it.

## Progress (2026-09-29, a colour only the frame declares)

The frame re-synced with `--code` (aldwin-tui.md, same date), which its
script sets per theme and `tokens/colors.css` does not declare. Stage 4
reads it: `frame_roles` finds each `setProperty('--role', this.light() ?
light : dark)` and adds the role to both scopes where `colors.css` has
none, so the role is generated, converted and listed in `*_VALUES` like
any other; once `colors.css` declares it, `colors.css` wins. Baseline
`code-ink-is-the-frames-not-colors-css` records the disagreement.

## Progress (2026-09-29, no check for removed identifiers)

The developer's call, on the evidence: open-tasks 4, a stage-2 check that
every Rust item a diff deletes is named nowhere in `crates/`, `docs/`,
`.agents/` or `AGENTS.md`, is not built. A prototype over the last 150
commits found one surviving name that was not also a word or a variable —
`caret_hidden`, in a dated Progress entry, where history keeps it on
purpose — and about 150 false hits: items named with English words
(`refuses`, `guard`) matched prose, and names that live on as variables or
fields (`step_id`, `run_dir`) matched code. Code that still uses a removed
item does not compile, so what is left is comments and docs, where a
stale current statement and a history entry with its pointer differ only
by reading. That is a judge's work: the exception to "a finding a judge
produces twice belongs in a deterministic stage" is a finding no machine
can tell from its lookalikes. The author's pass keeps its grep.

## Progress (2026-09-29, no doc examples)

The developer's call: no `# Examples` sections — the code is the example,
and a behaviour worth pinning is a unit test. The rule leaves the `rust`,
`comments` and `review` skills (the author's pass and stage 7's prompt),
and every example in `crates/` goes; where one pinned behaviour no other
test had, a unit test keeps it (a stage's label text is not behaviour). Of the two stage-2 checks the 2026-09-27
entry below names as next, only the removed-identifier check remains
(open-tasks 4; decided against the same day: see the entry above).

## Progress (2026-09-29, the working line)

The frame re-synced with a working line that moves at 10 frames a second
(aldwin-tui.md, same date), and two parts of the loop assumed a still
footer.

- **Stage 4 reads the new footer.** The context bar lost its `Context`
  label, which `frame_gauges` had anchored on; a bar is now found by its
  `var(--track)`, the only thing drawn over it, and the segment glyph is
  read rather than assumed — `GAUGE_CELL`, `█` where every bar used to be
  `━`, and a bar drawing two glyphs fails the stage. The working line's
  highlight, two `color-mix` tones of `--label` over `--label2` in frame
  `W2`, is generated as `HIGHLIGHT_{DARK,LIGHT}` the way the gauge's ramp
  is. `…` left the glyph table with `Working…`; the app still shortens
  text with it, so baseline `ellipsis-marks-shortened-text` licenses it,
  and the snapshot's glyph test now licenses every recorded exception in
  every scene — ADR 0002's table glyphs are held to `markdown` by
  `nothing_inside_a_frame_is_stroked`, which now walks every size as the
  glyph test does.
- **Capture runs with reduced motion.** A held turn's footer never goes
  quiet for `wait_quiet`, so every scene seeds `motion: reduced` in
  `tui.yaml`: the caret and the working line hold still and only the
  line's timer moves, once a second. The shot still waits for an edge and
  reads the grid back after it, now against that timer. With no caret
  blinking in any scene, `caret_hidden` and its skip went.

## Progress (2026-09-27, macOS)

CI's first macOS run found the crate did not build there: `pty.rs` named
the slave with `ptsname_r`, which libc has only on Linux-likes, and passed
`TIOCSCTTY` where macOS takes a `c_ulong`. The gate runs this crate, so a
contributor on a Mac could not commit through it. The second run built it
and failed opening a pty (`ENOTTY`). A pty here only feeds frame capture,
which drives foot under sway, and neither runs on macOS; so capture is
Linux-only. On macOS `Pty::open` is a one-line refusal rather than
syscalls that fail, and each platform has a test of its own answer; a
capture there meets the compositor failing to start sway first. The crate
builds on both, and the gate and stages 1–5 open no pty. Stage 8 on a Mac is
open-tasks entry 6.

## Progress (2026-09-27, the author checks first)

A small change — a failed turn's typed kind, which closed open-tasks 1 — used all five
iterations, and stages 1–5 passed in every one. Each round's findings were
things the skills already state and the author had not checked: a public
function without an example, a magic `attempts: 0` beside a new field, one
rule decided in two places, a stale sentence in a spec and then in an ADR,
a behaviour untested where it was produced. The judges were doing the
author's search one layer at a time. Decision 18 reverses Decision 17's
third rule — the code and Rust judges no longer run on a failing tree —
and puts the author's loop in front of the judges
(`a_failing_test_holds_back_every_judge`). Checks that would move more of
the author's pass into stage 2 — an example on every new public function,
a removed identifier still named in `.claude/` — are the next change (both
decided against 2026-09-29: see that day's entries).

## Progress (2026-09-29, the model's instructions)

The code judge has a fifth source: `crates/core/src/prompt.md` and the
description of each tool the diff touches. A change that leaves them
stating what the code no longer does is major; one that leaves them silent
on something new the model must act on is minor. The developer's call,
2026-09-29: the prompt is checked after every feature and fix and kept
strong. The prompt's own review that day found it describing the review
wrongly — it said to stage edits again after comments, when a commented
changeset stays staged — which no test or judge had caught, because no
source named it. The judge's inputs (`Judge::reads`) are unchanged: it
runs on every change under `crates/`, where the prompt and the tool
descriptions live.

## Progress (2026-09-27, comments)

The code judge's third source now includes `.agents/skills/comments/SKILL.md`,
which quality-gate's comment rule names: every comment is judged as written
for an LLM reader. The rework it drove landed one crate per commit, all
eight by 2026-09-27. What it found in the code, where a comment and its
code disagreed, was fixed or settled the same day (open-tasks entry 5,
now closed).

## Progress (2026-09-27, rounds, not readings)

The first change through the loop took twenty passes, and almost none of
that was stages blocking stages: stages 1–5 already all ran, and the judges
already ran in parallel. The passes went to re-reading and to variance.
Decision 17 is the answer: passes carry on unchanged inputs, the code judge
has two readers, and the code and Rust judges no longer wait on failures
they do not read. Tests pin each rule (`a_pass_carries_only_onto_the_same_inputs`,
`a_fingerprint_moves_only_with_what_it_covers`,
`a_failing_test_holds_back_only_the_frames_judge` — since Decision 18
`a_failing_test_holds_back_every_judge` —,
`a_carried_judge_has_nowhere_to_be_written`,
`a_code_verdict_takes_both_readers_and_keeps_every_finding`).

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
    change adds (the rule itself dropped 2026-09-29). Unscoped, the lint backlog's ~70 new docs on existing items
    each needed one, against the same skill's "keep changes minimal".
  - *quality-gate §6 over §7*: `aldwin-review` reported every failure as an
    `io::Error` with a sentence, and §7's "same error shape as the crate"
    let new code keep doing so against §6's typed errors. The crate now has
    one `thiserror` enum, `aldwin_review::Error`: every `io::Error` built
    from a sentence and `keys::parse`'s string error became one of its
    variants, so the two sections agree again. `tokens::check` keeps an inner
    `Result<usize, String>` on purpose — a stale file is the stage's answer,
    not a failure to run it.
- **The guard reads commands, not text** (the same day, after its first
  commit). Matching the raw text refused an ordinary commit whose message
  said `-n`. It now splits the command into words as the shell would
  (Python's `shlex`, so the hook needs `python3`; without it every command
  takes a refusing text fallback), and reads a command run from inside
  another — `sh -c`, `eval`, backticks, `$(…)` — as a command of its own.
  The first rewrite missed that last part and let `bash -c 'git commit -n'`
  through; the code judge caught it. `--no-verify`'s abbreviations are
  refused too, since git accepts them. `crates/review/tests/commit_guard.rs`
  pinned both directions, until it left the repository with the guard
  (Decision 16's note, 2026-09-27).
- **The two scene lists agree** (the same day). Seven snapshot scenes had no
  capture script and three capture scenes had no snapshot, so a change that
  moved only those was never judged against the design. The three got
  snapshot scenes and the seven got capture scripts — `selecting`
  and `commented` through `Shift ↓`, `working` and `running` on a reply held
  open (`fake::held`). `scene.rs` has a test that fails if the lists drift
  apart. `stopping` now snapshots the settled state — `Stopped.`, then
  ready — which is what the real app holds still for, and is captured too
  (the developer's calls, 2026-09-27: snapshot the settled state, and let
  `Stopping.` become `Stopped.` rather than stay beside it).
- **The guard refuses naming `AGENT`, not spellings of emptying it** (the
  same day). It listed `AGENT=`, `unset`, `env -u`, `export -n` and
  `declare +x`; the code judge emptied it with `read` and `printf -v`,
  which the list lacked. Any mention other than `$AGENT` or `${AGENT}` is
  now refused, a commit message's included.
- **The first change past the cap.** The fifth iteration ended with six
  findings; the developer signed off a sixth (2026-09-27) rather than
  commit with them open, and so on to a tenth. Two things kept a pass from
  going clean. Spellings built to get past the guard, which Decision 16 now
  places out of its scope. And MCP capture, whose put-back
  wrote outside the sandbox: passes 6, 7 and 10 each found a new way a path
  check in it could be fooled, so after the tenth the developer took it out
  of the change, to come back with the put-back confined by the kernel
  (open-tasks 2, since closed by ADR 0014 without a put-back). The saved review's `›` and frame D's step note were
  decided against on the way (`baseline.json`,
  `saved-review-is-not-reopened` and `a-plan-step-has-no-note`, the note
  since 2026-09-30 `a-plan-step-note-is-the-cards`). The cap stays five:
  going past it is the developer's call each time, never the loop's.

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
- **`cargo fmt --check` is back in stage 1**: the developer
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
  scenes have no counterpart here and say why.
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
  (Gone 2026-09-29: capture runs with reduced motion, so the caret never
  hides.)
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

- .agents/skills/review/SKILL.md — the loop, and the three judges' prompts.
- crates/review/src/tokens.rs — stage 4, and the roles it does not carry.
- crates/tui/tests/render_snapshot.rs — stage 5's baseline.
- crates/review/baseline.json — the design's own contradictions.
- docs/design/IMPORT.md — the reference, and its provenance.
