# mjolnir-design-conformance

The gap between what `crates/tui` draws and what `.claude/design/` specifies, as measured rather than as remembered — and the triage that decides which half of it is a bug.

**Status:** active — **six Class A findings open, none of them fixable in
`crates/tui`**: three wait on a data source or a token that does not exist,
one on a copy question, one on a design answer, one on a Class B decision.
Twenty-seven have been fixed or decided away across five passes; the dated
Progress entries below are where that history lives.

The three that closed on 2026-09-19 did so through a **decision**, not a fix:
ADR 0003 settled Class B 2, and items 4, 17 and 18 went with it. That is the
pattern this spec predicted — a Class B answer is what unblocks the Class A
findings sitting downstream of it — and it is the first time the prediction
has been tested. Item 16 was the same bet and it did not pay: the decision
unblocked it without fixing it, which is the distinction this spec's own Model
section draws and which the first write-up of that pass got wrong.
**Scope:** conformance of the shipped TUI's rendering to the imported design
system, for the twelve scenes in the screenshot catalogue at three sizes in
both themes. Covers the deviations, the design debt they sit next to, and the
exemption records that are missing. Excludes the screenshot harness itself
(`.claude/spec/mjolnir-screenshot.md`), the design system's own content, and
every functional question.
**Owner:** Maximilian
**Last Updated:** 2026-09-19

**Progress (2026-09-19, the handoff HTML, and four deviations it closed):**
The first pass in this catalogue's history to measure the *frame* rather than
the prose. `Agent TUI v2.dc.html` was fetched with `DesignSync` from the
discussion project `25845063-…` (the live one; `list_projects` does not return
it, so it is addressed by UUID — see the `design-sync` skill). It is not in
`.claude/design/`, which is why every previous pass read `HANDOFF.md` instead.

**CLAUDE.md's first design rule was right, and it cost this catalogue more
than anything else in it.** Measuring the markup refuted four separate
findings that judges had scored as defects and that this spec had recorded as
real:

- The option row's tones. The frame draws the number `--t-accent-text` when
  selected and **`--t-label`** otherwise, the idle mark **`--t-mark-idle`**,
  and the quoted pattern **`--t-quiet`** — exactly what ships.
  `HANDOFF.md:278-279`'s "accent-300 … neutral-600 … neutral-800" is stale in
  all three places. All three judges scored the app as defective against it.
- The command field is `background:var(--t-ground)`, **not** `--t-recess` as
  `HANDOFF.md:275` says. There is no `--t-recess` anywhere in `5a`: the prose
  uses the word twice and the markup zero times.
- There is **no separator band** above the options — `HANDOFF.md:277`'s "one
  row of the recessed tone" is a single blank row in the frame. The app was
  drawing a band because the prose said to, which no judge could ever have
  caught: they read the same prose.
- The inline diff's gutter is `flex: 0 0 var(--gutter-line-no-inline);
  text-align:right; padding-right: var(--cell-w)` — a 5-cell field holding a
  4-cell number and one cell of separation, which is what the code does and
  what four judges have now reported as a defect.

Fixed and deleted from the Class A list: **16, 30, 31, 33.**

- **30** — every prompt kind now gets `5a`'s field, not just a shell command.
  A path used to render as a dim `kind: target` line at the margin, so on the
  majority of prompts the panel had no quoted object at all. The `$` sigil
  stays shell-only; in front of a file path it would claim the file runs.
  The `kind` is not lost — it is the title row's right-flush badge, which is
  where `5a` puts it and the only place the frame has it.
- **16** — the panel's facts now sit on the frame's own columns. `5a`'s table
  is `in` / `writes` / `network`; only `in` is sourceable here, and the other
  two are **not invented** — nothing in the workspace knows what a command
  writes or whether the network is live. Measured on `run-1789848617`:
  `   in        ~/proj`, label at the 3-cell margin, value on **cell 13**.
- **33** — the separator band is gone.
- **31** — resolved by arithmetic rather than by an edit. The panel now draws
  **17 rows** at 120×36 and 160×44, and 17 is exactly right: the frame's own
  18 decompose as title + blank + sentence + blank + 3 field + blank + 3
  facts + blank + 4 options + blank + footer, and Mjolnir has two fewer fact
  rows and one more option. **`max_height` needed no change** — at 120×36 the
  content is 14 clampable rows against a budget of 21, so nothing was ever
  being clamped there; the quarter cap bites only at 80×24 and below, where
  it drops the fact row and then the field while keeping the options and the
  footer. ADR 0003 flagged this for re-examination; this is the examination,
  and the answer is that the number was never the problem.

Reclassified rather than fixed: **32**, which the markup showed was not a
missing field at all. See its entry.

Verification: `run-1789848617`, five scenes at three sizes in both themes —
30 frames, all six gates clean, regression clean (40 sections moved, all in
focus). Unit suite green across all 21 binaries, clippy clean.

**A harness gap found the hard way.** The first attempt at this run captured
an hour-old binary and reported "preflight clear" over it: `preflight` checks
that `target/debug/mjolnir` *exists*, never that it is newer than the sources
it was built from. The frames showed the old panel and the gates passed them
happily. Recorded in `mjolnir-screenshot.md`; until it is fixed, build before
every run and read the first frame before trusting any of them.

**Progress (2026-09-19, ADR 0003 — the permission list's shape):** The first
Class B question taken off Step 2's ordered list, and the one it named first:
**what shape a permission option row is.** Decided as `5a`'s sentence, with
`5a`'s per-row scoping rather than only its row shape. Recorded as ADR 0003,
which amends ADR 0001 §3.

Closed, and deleted from the Class A list — **three, not the four first
claimed here.** The retracted one is instructive and is recorded rather than
quietly corrected: item 16 was written up as closed on the reasoning that "the
rows that competed for their own grid are gone". That is an inference, and the
frames refute it. `prompt_scoped-medium-dark.txt` row 25 still reads
`   read: crates/tools/src/dispatcher.rs` — label at cell 3, value at cell 9,
which is the exact measurement item 16 recorded. **Nothing in ADR 0003 touched
the panel's card rows.** This is the spec's own pitfall — "a measurement cited
from a different screen is a reading, not a measurement" — committed against
the very frames that were sitting on disk at the time.

- **4** (the decision panel does not use `OPTION_LABEL_COL`) — *dissolved*
  rather than fixed. A sentence row has no name field, so there is no width to
  get right; `OPTION_LABEL_COL` is first run's alone, which is what `grep`
  always showed. The constant's comment claiming four lists shared it is
  corrected.
- **17** (the two rule literals do not form a column) — there is one rule
  literal per option row now, each inside its own sentence. The two rows that
  were six cells apart no longer exist.
- **18** (the in-panel `Tab` hint's idiom and tones) — the `Tab` row is gone
  with the toggle it described.

Class B **2**, **3** (the grant-summary row) and **4** (the `Tab` scope-toggle
row) are closed by the same ADR: 3 and 4 were inventions with no design
counterpart, and both were mechanisms for the axis ADR 0003 removed.

**What it cost, recorded here because it is not visible in a frame.** The
session tier now grants the exact target rather than ADR 0001's broad unit, so
running two different `cargo` invocations in one session prompts twice — the
friction ADR 0001 existed to remove, reintroduced on one row and bounded by
rows 3 and 4 still granting `cargo *`. And the panel no longer names the file
a grant lands in: `saved to ~/.mjolnir/permissions.yaml` became `everywhere`.
Both are in ADR 0003's consequences; both are pinned by a test so they stay
decisions rather than drift.

**One new entry under "What a judge will raise again"** (item 7): `5a`'s
fourth option reads `Deny and tell the agent why`, and Mjolnir's reads `Deny`,
because no step collects a reason. A blind judge holding `5a` will score it.

Verification: run `run-1789844710`, the five permission scenes at all three
sizes in both themes — 30 frames. Preflight clear, **all six gates clean on
all 30**, regression clean (24 sections moved, all in focus). Scored blind by
three judges, none seeing the source or the diff.

**Scores: minimum 61 against 90** — spatial 70–84, component 61–68. The
sentence list itself was verified against ADR 0003 by all three and penalised
by none; two independently noted that the option row's columns no longer drift
with frame width, which is §1's claim borne out. What holds the number down is
almost entirely *other* findings, and separating them is the point of this
entry.

**One defect this pass introduced, found by the judges and fixed:** the quoted
pattern was drawn in `--tui-dim`. `5a` says "one step quieter" than body, and
the ramp puts `quiet` at neutral-300 one rung under body's neutral-200 — `dim`
is neutral-500, three rungs down. Two judges measured it independently and
cited the same line. The cause was reasoning by analogy rather than off the
ramp: the code took the detail column's `body`/`dim` pair as the model, and
that pair is right for `5c`'s description text and wrong for this. Fixed to
`--tui-quiet`, re-captured as `run-1789845369` (12 frames, gates clean,
regression clean), declared cell now `#c9c5d2`.

**Three findings corroborated by all three judges, all new to this catalogue
and none of them caused by ADR 0003** — see items 30, 31 and 32 below.

**And one scoping defect in the ADR itself, not in the frames.** A judge read
§1's "the option row is a sentence" as governing the *edit-approval* panel's
`Approve` / `Deny` rows too, which still carry a detail column. That reading
was available because the section did not say otherwise; the ADR now scopes
itself to a Tool prompt's list and points the edit panel at Class B 5, where
the question already lived. Worth recording as evidence for Step 1: the judge
was not wrong to read it that way, and no amount of blindness would have
helped — the text was ambiguous.

**Progress (2026-09-19, full-catalogue conformance run):** Run
`run-1789826989`, all twelve scenes at 80×24, 120×36 and 200×50 in both
themes — 72 frames, the first session to score the whole catalogue. Preflight
clear, **all six gates clean on all 72 frames**, regression clean. Scored by
four blind judges, each taking three scenes, none seeing the source, the diff
or each other. Verdict: **below threshold, minimum 58 against 90**; mean
spatial 75.4, mean component 67.8, **zero frames at 90**.

The headline is not the number. It is that a zero-violation gate run and a
58 sit on the same 72 frames: every deviation catalogued below is invisible to
all six gates, by construction rather than by oversight. A clean gate run is
evidence about painted cells and declared colours. It is not evidence about
design fidelity, and this spec exists because that distinction stopped being
theoretical.

The second finding is structural and changed this spec's shape. A first pass
at the findings treated them as one list of bugs. Reading the source
afterwards — which the judges must not do and the author must —
**several of the loudest findings are deviations the code already decided,
with the reasoning written in a doc comment and nowhere else.** They are
absent from `crates/screenshot/baseline.json`, whose entire job is to hold
deviations that are correct by decision. So every future session will
rediscover them, score them as defects, and spend a judge's attention on a
question that was answered months ago. That is the most expensive defect this
run found, and it is a defect in the *records*, not in the pixels.

**Progress (2026-09-19, first fix pass):** The Class A items that were
neither blocked on a Class B decision nor a layout redesign are built:
the status row's rhythm, tone and labels (items 1, 2, 20); the composer's
cursor/placeholder collision (3); the selected option's contrast (5); the
tool-call line's spacing (7, layout half); the diff gutter's tone (11, tone
half); first run's cwd tone (13); the panel sentence's full stop (21); the
access row's foreign hue; and two findings no judge made (23, 24).
`crates/tui` is clippy-clean with 300 tests passing and `render.snap` reblessed.

Reading the source and the handoff to *place* these findings — which the
judges are forbidden to do and the author must — changed four of them, and
that is the entry's real content:

* **One was refuted.** The diff gutter is not a cell narrow. Two judges read
  the handoff's prose ("a 5-cell right-aligned line number"); the code
  implements the handoff's HTML (`padding-right: 9px`), which is a 4-cell
  number in a 5-cell field. Fixing it would have broken correct code to match
  a misreading. CLAUDE.md's first design trap, met in the wild.
* **One was mis-attributed.** Context-row code is `--tui-context`, not
  `--tui-dim` — but `semantic.css` defines both as `--color-neutral-500`, so
  the judges' measurement was right and their inference was not. The role is
  a deliberate de-emphasis; what made it a defect was collapsing into the
  gutter, which fixing the gutter's tone resolves.
* **Two were invisible to the method.** The inline diff's sign/code split
  (item 23) needs the handoff's *reasoning*, not a measurement — a judge
  cannot see that 2.7:1 is the number the design was avoiding. And
  `justified_line`'s overflow (item 24) was caught only by reading the
  snapshot diff line by line before blessing it: the fix for item 7 widened a
  field by two cells, which was one more than an 80-column tool row had, and
  the result was a summary clipped at the frame edge with no ellipsis. No
  gate, no test and no judge would have reported it.

Also corrected here: Step 1 as originally written ("promote the undeclared
exemptions to `baseline.json`") would not have worked, because the judge never
reads that file. The step is now the protocol question it actually is.

One thing to watch on the next run: **`HANDOFF.md`'s prose is partly
pre-Turn-13.** Line 257 — the line both judges cited for the missing timestamp
row (item 22) — also specifies "a 12-cell label column" and "the content
column starting at cell 17", which Turn 13 superseded with 8 and 13. Half that
line is known-stale, which is reason to confirm item 22 against the frames
before building it rather than after.

**Progress (2026-09-19, first fix pass scored):** Run `run-1789829088`, the
whole catalogue again, scored by four fresh blind judges on the same splits.
**The acceptance number went the wrong way: minimum 58 -> 48.** Means rose
(spatial 75.4 -> 79.3, component 67.8 -> 68.3) and the scenes the fixes touched
rose sharply on spatial — `tools` +17, `first_run` +16, `markdown` +15,
`fenced_diff` +13 — with three judges independently confirming specific fixes
(the tool line on cells 13/16/22, the gutter at `--tui-label`, the inline
diff's unified sign/code, and "right-edge collisions: none … clean on the
recurring defect"). But the minimum is set by `approval_large-small`, which
this pass did not touch and which a different judge marked 64 -> 48 on the
same defect it had described before.

Two things follow, and the second matters more than the first.

**The A3 fix was half-right.** Moving the placeholder off the caret cell
removed the collision, and left a worse-framed defect: the caret is still the
terminal's hardware cursor, so cell 6 is now a lone `#ffffff` hollow box — a
colour in neither palette, a *stroked* object in a frame where nothing is
stroked, measuring 1.07:1 on the light composer (invisible) and the brightest
mark in the frame in dark. It also pushed the draft to cell 8, which is not a
landmark. Three judges raised it. The right fix needs no Class B decision and
was available all along: draw `▌` in `--tui-mark` and hide the hardware
cursor — `first_run` already hides it, so the mechanism exists in the crate.
Choosing the minimal change to avoid pre-empting the placeholder question was
the wrong call; the caret and the placeholder are separable.

**A minimum across 72 frames is the noisiest statistic available, and it is
the acceptance metric.** One judge's harshest call on one untouched frame
moved the number that decides whether the loop may stop, by more than every
fix in this pass moved it the other way. This run cannot distinguish "we
regressed `approval_large`" from "a different judge marked the same defect
harder", because the frames changed and the judges changed at once. That is
not an argument for a mean — the skill is right that a mean hides the frame a
developer would actually reject. It is an argument that the *variance* is
unmeasured: nobody has ever scored one unchanged run twice. Until someone
does, a minimum's movement between runs cannot be read as signal. Cheap
experiment: rescore `run-1789826989` with fresh judges and see how far 58
moves on its own.

**Progress (2026-09-19, the layout pass, scored):** Run `run-1789832849`,
the whole catalogue, five captures and two scoring rounds. Final state:
**preflight clear, all six gates clean on all 72 frames, regression clean**
(76 sections moved, all in focus). Second scoring round, four fresh blind
judges on the same splits: **minimum 56 spatial / 58 component against 90**,
means 77.6 / 71.3 (from 75.4 / 67.8 on the first full run and 79.3 / 68.3 on
the second), **zero frames at 90**. Below threshold; the loop may not stop.

Built: Class A items 3 (the caret, second pass), 8, 9, 12, 14 and 15 — the
whole unblocked half of Steps 6 and 7 — plus items 25, 26 and 27, all three
of which this session created or exposed. The catalogue entries carry the
measurements; three things belong here instead.

**Two of the three new items are regressions from this session's own
fixes**, and both were found by the harness rather than by a reader. Item 15's
cap made the panel elide, which fired the `breakages` gate on an elision
marker that had drawn `⋯` and an em dash since it was written (item 25) —
neither in the closed table, neither exempted, and invisible through three
clean runs because no frame had ever been short enough to elide. The same cap
then made the panel elide *what it was asking about* (item 26), which a judge
measured. Item 9's own fix put the hunk header on the code column (item 27),
which a different judge measured. **The lesson is not "be more careful"** —
each of the three was invisible to the author by construction. It is that a
fix that changes what a frame *contains* changes which code paths a frame
exercises, and the harness is the only thing that sees the second effect.

**One classification was corrected in the other direction.** Item 12 named
`--pane-commands-w` as the bound for first run's option list; 48 cells elides
`claude models · ANTHROPIC_API_KEY`, the reference's own row, at the
reference's own frame width. The list is sized by its content instead — and
two independent judges then measured the `more` row's `→` against the
handoff's "flush to the 3-cell right margin", which is the one thing the
design states about that row's right edge. The arrow went back to the margin
and the band stayed bounded by the list, so each half cites something; what
the design does **not** state is how wide the list is, and that is now a
Class B entry rather than a number invented twice.

**A degradation rule emerged, and it is now used in two places.** At 80×24
neither the transcript nor the permission panel can hold everything the
design's 120×36 frame holds. Both now give up *spacing* before *structure*:
a turn break drops its two blank rows and keeps its band (item 14), and the
panel's options separator does the same. The alternative — dropping the whole
separator — is what a judge scored as "the panel's facts run straight into
its option list", and dropping the content instead is what item 26 was.

Where the remaining gap is, from 72 frames of judging: the permission panel
has no key/value table and no recessed field (Class B item 2 and item 5, with
Class A items 4, 16, 17 and 18 downstream of them); the top bar has no gauge,
no cost and no branch (Class B item 6); the light ladder collapses the diff
box against the panel ground at 1.01:1 (Class C item 2, measured again in
three approval frames); and nothing caps the measure at 200 columns (Class C
item 3). **None of those is reachable by writing Rust**, which is the same
conclusion the previous pass reached, now with a second set of judges and a
third run behind it.

## Why

The design system is imported, not invented here, so "does this match" is a
question with an answer rather than a preference. But the answer is only
cheap to get once. A deviation that is noticed, reasoned about and then left
in a doc comment costs the full price of rediscovery every time anyone looks
at the frames — and the judge who rediscovers it is deliberately denied the
context that would let it recognise the decision.

Three things therefore have to be distinguishable on sight, and today they
are not:

- the app disagrees with the design and the app is wrong;
- the app disagrees with the design and the *design* has no answer, because
  the app draws something the design system never drew;
- the app disagrees with the design and the disagreement is correct, because
  someone decided it deliberately for a stated reason.

Only the first is loop work. The skill is explicit that the second is a stop-
and-ask, and that widening an exemption for the third "is a conversation with
the developer, not a step in the loop". Neither rule can be followed while all
three arrive as one undifferentiated list of low scores.

## Vocabulary

- **Deviation** — the design specifies a value or position, the app draws a
  different one, and no record says why. Class A. Loop work.
- **Design debt** — the app draws a component the design system has no
  counterpart for. Class B. Needs a decision (an ADR, or a re-sync that adds
  the component), never a patch invented here.
- **Token debt** — the design system contradicts itself, or undercuts a floor
  it states. Class C. The fix belongs upstream at `claude.ai/design`; the app
  records it and does not compensate locally.
- **Exemption** — a deviation that is correct by decision, recorded in
  `crates/screenshot/baseline.json` with an `authority` citing the ADR or spec
  entry that made it one. An exemption with no authority is an unfixed bug
  wearing a costume.
- **Undeclared exemption** — the state this run found: a decision that exists,
  is reasoned, and is recorded only in a doc comment. Indistinguishable from a
  Class A deviation to anything that reads the frames.

## Model

Every finding lands in exactly one class, and the class determines who may act
on it:

| class | the app | the design | who acts |
| --- | --- | --- | --- |
| A — deviation | wrong | has an answer | the loop |
| B — design debt | drew something new | has no answer | the developer, via ADR or re-sync |
| C — token debt | conformant | wrong or self-contradictory | upstream, then re-import |
| exemption | deliberately different | has an answer | nobody; it is closed |

The classification is not a judgement call made per session. It is a property
of the finding, and it is recorded once. The judge cannot make it — the judge
is blind by design and sees only that a frame disagrees with a reference. So
**classification is the author's job, done after the scores land and before
any fix is attempted**, and this spec's catalogue is the durable result.

The rule that falls out of it: a Class A fix may change `crates/tui`. A Class
B or C finding may not, in either direction — not by patching the app toward
an answer the design does not give, and not by widening a baseline entry until
the gate stops asking. Both are the loop optimising its own scorer.

## Decisions

1. **The catalogue lives here, not in the run directory.** Run directories are
   pruned to the last five by `start`, so `run-1789826989`'s numbers would age
   out within five sessions. Every measurement this spec relies on is restated
   inline for that reason, with the run named as provenance rather than as a
   pointer.

2. **Undeclared exemptions are promoted to `baseline.json`, not deleted from
   the catalogue.** The doc comments stay where they are — they explain the
   code to a reader of the code. The baseline entry is what makes the decision
   visible to the harness, and its `note` is where the reasoning gets
   restated for someone reading frames rather than source.

3. **A fixed deviation is deleted; an *answered* one is kept.** The two are
   not the same thing and the difference is what a blind judge will do next.
   A fixed deviation is gone from the frames, so no judge can raise it and
   keeping it only makes the list of work harder to read: it goes, and the
   dated Progress entry for the pass that built it is where it lives
   afterwards. A finding that was answered some *other* way — refuted by the
   reference, reclassified, or decided against the design — is still in the
   frames. Four independent judges flagged the version in the session top
   bar; the code answers it deliberately at `ui/chrome.rs:109-112`, and
   deleting that would guarantee a fifth judge raises it. Those live under
   "What a judge will raise again", which is a different list with a
   different job.

   Numbers are not reused. The Progress entries cite findings by number, so a
   reader following one of those citations finds a gap rather than a
   different finding wearing the deleted one's number.

4. **The three-way split is prior to any priority ordering.** An earlier
   summary of this run ranked the findings by frames affected and put "drop the
   version from the top bar" first at 66 frames. That ranking was wrong in kind,
   not in arithmetic: the top item was not a bug. Frame counts order work
   *within* Class A and say nothing across classes.

5. **Scenes that reach no composer do not get a composer finding.** The
   `prompt*` family renders the permission panel, where input is disabled and
   `5a` draws no prompt row at all. A finding must name the screen it belongs
   to; "the caret is missing" is false on a screen that has no caret.

## Class A — deviations

Measured in `run-1789826989` unless an entry says otherwise. Frame counts are
out of 72. Cell indices are 0-based, and the capture cell is 8×18px, so a cell
index times 8 is the pixel column in the PNGs.

**A fixed deviation is deleted from this list.** What it was and how it was
answered is in the Progress entry for the pass that built it; what belongs
here is work, and a list that keeps its own history stops being readable as
one. The exception is a finding that was *not* fixed but answered some other
way — refuted, reclassified, or decided against the design — because a blind
judge will raise it again and the answer has to be somewhere. Those are at
the end of this spec, not here.

| | item | open on |
| --- | --- | --- |
| 6 | nothing caps the body measure | Class C 3 |
| 19 | a call blocked on a permission is drawn as running | a `ToolActivityStatus` the app does not have |
| 22 | no timestamp row under the speaker label | a clock the workspace does not have |
| 28 | the empty state's status row copy | a live copy question (`idle` against `ready`) |
| 29 | a tool line's target and summary | a `ToolActivityEntry` that carries neither |
| 32 | a hunk's tints are near-invisible on the panel's bar | Class B 5 |

**Five of these are reachable by writing Rust in `crates/tui`** — 16, 30, 31,
32 and 33, all found or re-measured on `run-1789844710` and all in the permission
panel. They are the whole of Step 4's queue, and for the first time since this
catalogue was opened that queue is not empty. Of the other five, three wait on a data
source or a token that does not exist, one on a copy question, one on a design
answer. None waits on a Class B decision any more — ADR 0003 took the last of
those.

Items 30-32 are new, added 2026-09-19 from `run-1789844710`'s three blind
judges; each was raised independently by all three, which is why they are
entered without the usual single-judge caution.

Twenty-three of this catalogue's findings are no longer listed here: 1, 2, 3,
5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 20, 21, 23, 24, 25, 26, 27 were fixed;
4, 17 and 18 were closed by ADR 0003 deciding the Class B question they sat
downstream of.
The numbers are not reused — the dated Progress entries above cite them, and
a reader following one of those citations should find a gap rather than a
different finding standing in the deleted one's place. Three of the twenty
left something behind that is still live: item 7's target slot is now part of
item 29, and items 10 and 11 left the two records under "What a judge will
raise again" below.

6. **Nothing caps the body measure.** 18 frames (every `*-large-*`), with the
   200×50 size the only one that shows it. Assistant prose runs cols 13–197 as
   a single 183–185 character line; a diff field spans 184 cells for 62 cells
   of code; in `tools-large` a call ends at cell 27 and its own right-flushed
   summary occupies 180–196, 152 cells away, reading as two unrelated facts.
   All four judges raised it. **This is open-tasks entry 1**, which measured it
   in one scene; it is in every scene that renders prose. Note the design
   frames are 120 cells and state no maximum measure, so the *token* is Class
   C (item 3 below) while the unbounded layout is Class A.

19. **A call blocked on a permission is drawn as running.** 30 frames. The
    glyph is `◐` ("tool call / process running") on a call that has not run
    and cannot until the open panel is answered; `○` ("pending") is already in
    the closed vocabulary for exactly this state. `prompt_scoped` makes it
    plain by drawing two `◐` against a single open permission. **Deferred, and
    heavier than it looks:** `log.rs:56`'s `ToolActivityStatus` has exactly two
    variants, `Running` and `Completed`, so the app cannot currently tell a
    blocked call from a running one. The TUI *can* know — `app.pending_
    approvals` names the call being decided — but it means matching activity
    entries against pending decisions at render time, not swapping a glyph.

22. **No timestamp row under the speaker label.** Every frame that draws a
    label. `HANDOFF.md:257` puts the speaker on the label column's first row
    and the time on its second; the second row is empty in every frame, so a
    turn reads one row tall where the design makes it two. **This is
    open-tasks entry 4**, which asked whether it was deliberate or never
    built. Nothing in Turns 13–15 retires it and `09:42` fits the narrowed
    8-cell column, so it is unbuilt rather than decided against — but that
    conclusion is from the design side only and wants confirming before it is
    built.

    **Attempted and stopped 2026-09-19, on the data rather than on the
    design.** There is no clock anywhere in the workspace: no `LogEntry`
    carries a time, no crate depends on `chrono`, `time` or `jiff`, and local
    wall-clock time is not reachable from `std` alone. So this is a
    dependency decision, plus a change to the log's data model, plus a
    fixture pin in `render.snap` — on evidence that is half-superseded, since
    `HANDOFF.md:257` also states the 12-cell label column Turn 13 replaced.
    It belongs with Class B item 6, the branch marker: **a data source that
    does not exist, not a rendering defect.** Two judges have gone on scoring
    the empty second row of the label column, so the finding is real; what is
    wrong is its class.

28. **The empty state's status row is not `14d`'s copy.** 6 frames. `14d`
    specifies `ready` left and `^d closes` right; the app draws `idle` and
    `^c to exit`. **Half of this is not a deviation to fix**: `^c` is the key
    Mjolnir actually binds, and a hint naming a key that does nothing is
    worse than one that disagrees with the reference. `idle` against `ready`
    is a live copy question — `idle` is the app's own activity vocabulary
    (`idle`/`thinking`/`working`) and `ready` is the design's word for the
    same state on the one screen that names it. Decide it rather than
    silently keeping either.

29. **A tool line's target and summary are the wrong facts.** 12 frames, and
    the half of item 7 that survived it: the *columns* of this row are
    correct — glyph at 13, name at 16, target at 22 — and the content in two
    of its three slots comes from somewhere the design never intended.

    The target slot holds `(call-1)`. `log.rs:49`'s `ToolActivityEntry`
    carries `call_id`, `name` and `status` and nothing else, so the file or
    command `4a` shows there is not available to render. The right-flush slot
    holds the tool result's first line — `// the dispatcher` for a read —
    where `4a` shows a fact *about* the result (`412 lines`, `7 hits in 3
    files`); `log::summarise` takes the first line, which is the right shape
    for a shell command and the wrong one for a read. Four judges across two
    rounds have now scored one or both.

    Neither is a rendering fix. Same shape as Class B item 6, the branch
    marker: a data source that does not exist, and a decision about what
    `ToolActivityEntry` should carry.

32. **A hunk's tinted rows are near-invisible on the panel's bar.**
    12 frames (`approval-*`), measured `#3d4b42` on `#474251` = **1.054:1**
    dark and `#d3ead6` on `#e8e4ee` = **1.015:1** light.

    **Reclassified 2026-09-19, from "buildable" to blocked on Class B 5.**
    The first write-up called this a missing field and put it in Step 4's
    queue. The handoff HTML says otherwise: `4a`'s inline diff is a
    container on `--t-diff-bg` whose *changed* rows each override that
    ground, with no padding rows of its own — so an all-changed hunk shows
    no unoverridden field ground in the reference either, and
    `Row::field(pal.diff_box).inset(MARGIN_X, pal.bar)` already implements
    exactly that structure. `approval_large` looks right only because it has
    context rows, which are what paint the ground.

    What is actually wrong is the *adjacency*, and it has no local fix. The
    two diff tints are designed against the transcript's dark ground
    (`--t-ground` `#27232f`), which is what `4a` sits them on. Mjolnir shows
    them inside a permission panel, whose surface is `--tui-bar` `#474251` —
    a lighter rung, against which those same tints all but vanish. No
    arrangement of the field changes that: insetting it leaves the tint
    beside `bar`, and not insetting it leaves the tint above and below
    `bar`. **The design has no edit-approval panel at all**, so it has never
    had to answer what surface a diff sits on inside one. That is Class B 5,
    and per the screenshot skill a rendered component with no counterpart in
    the design system is a stop-and-ask, not loop work.

## Class B — design debt

The app draws these; the design system has no counterpart. Per the skill,
each is a stop-and-ask. None may be patched toward an invented answer.

1. **The composer placeholder.** `ui/chrome.rs:432-440` draws
   `Ask Mjolnir anything`, or `waiting on your decision above…` when blocked,
   with a stated reason: "an empty filled box gave no hint at all that this was
   where a message goes." `14d`'s empty composer is `▶  ▌` and carries no
   placeholder. The blocked variant is the stronger case of the two — it
   reports a state the design never drew. Note the copy is also second-person
   imperative naming the product, against the third-person Content
   Fundamental, so a decision to keep it is also a decision about its words.

2. ~~**The five-option permission list.**~~ **Closed 2026-09-19 by ADR 0003.**
   `5a` draws four options as single sentences with the matched pattern
   quieter; what shipped was name + description pairs, which is `5c`'s
   command-list shape. Decided as the sentence — and as `5a`'s *per-row*
   scoping, not only its row shape, which is what carried 3 and 4 out with it.
   ADR 0001's five options stay, and the footer still correctly reads
   `1-5 to pick`. Kept in this list rather than deleted, because it is the
   worked example of what answering a Class B question does: four Class A
   findings closed without one of them being fixed. **The design still has not
   been re-synced since ADR 0001** — the sentences for a global tier and a
   five-option list are Mjolnir's own words, which is the debt this leaves.

3. ~~**The grant-summary row.**~~ **Closed 2026-09-19 by ADR 0003.** It
   existed because ADR 0001 required the prompt to "state the grant it would
   write, not the command that triggered it", and the design had no component
   for it. Every allow row now states its own rule in its own sentence, which
   discharges that requirement on the row being picked rather than two rows
   above it at `--tui-dim` — the inversion this entry recorded.

4. ~~**The `Tab` scope-toggle row.**~~ **Closed 2026-09-19 by ADR 0003.** A
   local invention for an axis that no longer exists: scope is a property of
   each row now, so both scopes are on screen at once and neither needs a
   keypress to reach. `Tab` is unbound in the panel.

5. **An edit-approval panel carrying an inline hunk.** `5a` is a shell prompt:
   sentence, `$`-prefixed command in a recessed field, `in`/`writes`/`network`
   facts, four options. Nothing in the handoff designs a permission surface
   that shows a diff; `approval_large` composes `4a`'s inline diff and `5b`'s
   hunk geometry into a panel. Two things about it are *correct* and should
   survive any redesign: the 2-option set, and dropping `saved to …` from the
   footer, both because ADR 0001 never writes an `edit` grant.

6. **No branch or dirty marker in the top bar.** All four judges raised it
   against `14d`. `ui/transcript.rs:548` answers it deliberately — nothing in
   `StatusInfo` tracks a branch, and the same call is made about the
   reference's context gauge and session cost, neither of which is fabricated.
   So this is not a rendering defect: it is a data source that does not exist.
   Building it is real work outside this spec; deciding not to is an exemption.

7. **No continuation treatment for a scrolled turn.** 4 frames
   (`long-medium-*`, `long-small-*`): 25 rows of transcript with the 8-cell
   label column entirely empty, because `harness` scrolled off the top. The
   label column is dead space and nothing on screen names who is speaking. The
   design has no answer for a turn taller than the viewport.

8. **How wide first run's option list is.** The design states the row's
   *internals* — mark at cell 29, name at 32 in the shared 16-cell field,
   detail at 48 — and one thing about its right edge: the `more` row's `→` is
   "flush to the 3-cell right margin". It states nothing about where the list
   itself ends, which is the width the selection band paints.

   That gap has now been filled twice by invention and measured as a defect
   both times. Filling to the frame's right margin made the band 167 cells of
   accent at 200 columns — the deviation this started as, Class A item 12,
   fixed and deleted 2026-09-19 — against the rule that the accent is "a mark
   or a line, never a filled field". Sizing it to the list's own
   widest row fixed that and drew a different complaint from a fresh judge —
   "the band ends on the last glyph of `ANTHROPIC_API_KEY`, so it is sized by
   string length, not by a region". Both readings are right, which is the
   signature of a question the design has not answered.

   `5c`'s 48-cell command list is the nearest stated number and is **not**
   this list: it is four cells too narrow for `anthropic` plus `claude models
   · ANTHROPIC_API_KEY`, the reference's own row, at the reference's own frame
   width. What ships is the content-sized list with the `→` at the margin, so
   every part of the row cites something; the band's edge is the part that
   cites nothing, and it wants a token or a decision rather than a third
   invention.

## Class C — token debt

Conformant renders of a design system that contradicts itself. Fix upstream
via `DesignSync` (read the `design-sync` skill first), then re-import; do not
compensate in `palette.rs`.

1. **ADR 0002 contradicts the ramp it cites.** The ADR calls `--tui-quiet`
   "the tier below `dim`" and states the intent that "the structure is present
   without competing with the cells for the eye". In the ramp, `quiet` is
   neutral-300 and `dim` is neutral-500 — `quiet` is *brighter*. Rendering the
   ADR's letter faithfully therefore produces table rules that outrank the
   header cells they frame, which is the stated intent inverted. 10 frames.
   The fix is a token decision (`--tui-dim` or `--tui-mark-idle` for rules) and
   an ADR amendment; it is not a `markdown.rs` change.

2. **The light ladder undercuts its own stated floor.** *(Re-measured
   2026-09-19 by two judges on `run-1789844710`, both unprompted, with a new
   rung: dark `--tui-break` `#1e1a26` against `--tui-ground` `#27232f` is
   **1.111:1**, so this is not a light-theme-only failure as first written.
   One judge reports the consequence in a frame: in `prompt-medium-light` the
   option separator and the whole footer band are invisible by eye.)* The handoff states
   "every adjacency in the five screens is now at least 1.15:1" and a narrowest
   light rung of 1.127:1. Measured: `--tui-bar-bottom` against ground
   **1.079:1** and against the panel **1.072:1**; `--tui-diff-box` against
   `--tui-bar` **1.01:1**. In a design where "nothing inside a frame is
   stroked", a boundary at 1.01:1 is not a faint boundary — it is no boundary,
   with nothing to fall back on. 9+ light frames, worst in
   `approval_large-*-light`, where the hunk's first and last rows float with no
   field at all. Note `--tui-diff-box` is specified for the *transcript's*
   quoted-code ground (1.146:1 there); the design's answer for a payload field
   inside a panel is `--tui-recess`, so part of this may resolve as a Class A
   role choice once the Class B panel question is settled.

3. **There is no maximum-measure token.** The design frames are 120 cells and
   the body column is 104; nothing states what a 200-cell frame should do.
   Class A item 6 is the app's unbounded behaviour, but no conformant value
   exists to fix it *to*. This wants a token, not a constant invented in
   `grid.rs`.

4. **The `5a` recess separator fails in opposite directions per theme.**
   `#0f0b15` on `#474251` is 2.01:1 in dark and reads as a black gash across
   the panel; `#ded9e6` on `#e8e4ee` is 1.105:1 in light and is nearly
   invisible. One specified row, two opposite failures, so no single local
   adjustment is right.

5. **`5a`'s colour paragraph predates the token layer — now proven at
   source, not inferred.** Two judges reached this independently from
   different evidence, and the handoff HTML then settled it outright
   (fetched 2026-09-19 via `DesignSync`). The frame's own option rows read:

   ```html
   <span style="color:var(--t-mark-idle)">▌</span><span>  </span>
   <span style="color:var(--t-label)">2</span><span>  </span>
   <span style="color:var(--t-body)">Allow </span>
   <span style="color:var(--t-quiet)">cargo test</span>
   <span style="color:var(--t-body)"> for this session</span>
   ```

   So the number is `--t-accent-text` selected and **`--t-label`** otherwise,
   the idle mark is **`--t-mark-idle`**, and the quoted pattern is
   **`--t-quiet`** — exactly what ships. `HANDOFF.md:278-279`'s "accent-300
   … neutral-600 … neutral-800" is stale in all three places, and all three
   judges scored the app as defective against it. **Nothing here is a Class A
   finding; the paragraph is.**

   `HANDOFF.md:278-279` states the option row's tones as raw ramp names:
   the band "accent-900", the idle mark "neutral-800", the number "accent-300
   on the selected row and neutral-600 on the rest". Measured against the
   shipped token files, two of those are unusable and one is contradicted:

   - **neutral-800 for the idle `▌` is `#474251`, which is the panel's own
     ground — 1.00:1, an invisible glyph.** The app draws `--tui-mark-idle`
     `#5d576a`, inside the 1.6–2.4:1 band the handoff itself states for this
     glyph at line 109. The app is right and the line is stale.
   - **accent-900 for the band is contradicted by `palette.css` in its own
     words** — "a real accent fill now, not a faint tint", `--color-band-dark:
     #604788`, which is what ships.
   - The number's rungs sit in the same sentence as those two. All three
     judges measured `--tui-accent-text` / `--tui-label` against the stated
     accent-300 / neutral-600, and one explicitly held the finding at low
     confidence for exactly this reason.

   **So the number's tone is Class C, not Class A**, and it is not entered in
   the Class A list. What it wants is the paragraph rewritten in semantic
   roles like the rest of the handoff. Until then a judge will keep measuring
   it, which is why it is recorded here.

   A related self-contradiction in the same paragraph, raised by all three:
   "one cell after the mark and two cells before the label" yields text at
   cell 5, and the very next clause says "option text starts at cell 6". All
   three scored against the stated 6, which is what ships.

## What a judge will raise again

Findings that are answered but not *fixed*: decisions recorded only in doc
comments, a measurement the reference refutes, a role whose name misled the
measurer. Every one of them is scored as a defect by every blind judge and
will be again, because the frames still show what the judges measured.
**Getting these in front of the judge is the highest-value work in this
spec** — it is cheap, it is not a code change, and it is what stops the next
four judges spending their attention on closed questions. See Step 1: where
that record lives is a protocol question, not a `baseline.json` entry.

1. **The inline diff's gutter is not a cell narrow.** ~~Class A item 11's
   width half.~~ **A fourth judge reported it on 2026-09-19**, this time
   citing `cells.css`'s `--gutter-line-no-inline: 5` rather than the prose —
   which makes it the single most re-discovered finding in the catalogue and
   the strongest evidence for Step 1. Three judges across three runs before
   that reported the line number
   right-aligning to cell 16 where a 5-cell gutter would put it at 17. They
   are reading `HANDOFF.md`'s prose ("a 5-cell right-aligned line number");
   `ui/diff.rs:160` implements the same handoff's *HTML* — `flex: 0 0 45px;
   text-align: right; padding-right: 9px` — which is a 5-cell field carrying
   a 4-cell number and one cell of separation. The code is correct and the
   prose is loose, which is exactly the trap CLAUDE.md's first design rule
   names: **measure the handoff HTML; reading it is not enough.**

2. **A diff context row's code is `--tui-context`, not `--tui-dim`.**
   ~~Class A item 10.~~ Judges measure `#9a95a4` and infer `--tui-dim`;
   `semantic.css` defines `--tui-context` as the same `--color-neutral-500`,
   so the measurement is right and the inference is not. The role is a
   deliberate de-emphasis — `ui/diff.rs`: "only the changed lines should
   compete for attention" — and the design never draws an inline diff's
   context rows at all, since `5b`'s hunk is an all-added new file. What made
   it a defect was the measured *collapse*, context code the identical colour
   to the gutter number beside it, and moving the gutter to `--tui-label`
   resolved that. Revisit only with evidence from a frame, never from a token
   name.

3. **The version in the session top bar.** All four judges flagged it, citing
   `14d`: "There is no version and no commit on this screen." The code answers
   it at `ui/chrome.rs:109-112`: the right group is "the model name and the
   running build version — the closest real facts Mjolnir has to the
   reference's `model · gauge · cost` group", with a gauge and a cost
   deliberately not fabricated. `ui/transcript.rs:541-546` records the Turn 14
   change and notes that the screen it took version *off* is the intro, and
   that `the_top_bar_reports_the_running_builds_version` still pins it.
   The tone the judges called "the brightest ink in the bar" is also
   deliberate: `chrome.rs:127-131` states the reference's right group as
   quiet / dim / text, with `text` for the last fact in the group.
   **This is a live disagreement, not a settled one** — the design says the
   session bar carries the model *instead of* the version — but it is a
   disagreement with a reasoned position on both sides, which is a
   conversation, not a bug. Record it, decide it, then either drop the version
   or cite the decision.

4. **The `access` row's three permission states.** Judges scored
   `read:deny  shell:deny  edit:deny` against `14d`'s single tier word.
   `ui/transcript.rs:552-559` refuses the tier deliberately: "A tier is what
   first run *writes*; it is not what is stored, and a `permissions.yaml`
   edited by hand need not correspond to any tier at all. Reporting one would
   be a guess printed as a fact on the screen whose whole job is to say what
   this directory permits." That reasoning is stronger than the reference.
   **What survives as Class A is the colour, not the content**: each `deny`
   renders `--tui-del`, the reserved diff-removed hue, and the handoff permits
   the two diff hues as its only foreign colours precisely because they mean
   *diff*. An access tier painted in it reads as a deleted line, and in the
   light theme it is the most saturated thing in a deliberately shallow frame.

5. **The footer's right-flush provenance note.** `5a` ends its footer with
   `saved to .harness/permissions.toml`; Mjolnir's footer ends at the key
   hints. `decision.rs` removed it deliberately: it was true of exactly one
   of the five tiers on offer, and "allow once" and "allow for this session"
   save nothing at all. Two judges have now scored the empty right half of
   that row as a defect. The reasoning is sound and the record is missing.

6. **The option row's mark and number inside the 3-cell margin.** Already a
   baseline entry for the `layout` gate, cited to `5a`. Listed here because it
   is the model the other entries should copy: scope named, authority cited,
   and an explicit boundary ("Scoped to the left margin only: nothing licences
   running off the right").

7. **`Deny` against `5a`'s `Deny and tell the agent why`.** New with ADR 0003,
   which adopted `5a`'s option copy verbatim everywhere except here.
   `PromptResponse::Tool` carries a decision, a tier and a pattern — there is
   no reason field and no step that collects one, so the reference's sentence
   would name a thing the product does not do. This is the A28 precedent
   applied a second time: a hint naming a key that does nothing is worse than
   one that disagrees with the reference. A judge holding `5a` will score it,
   in all 30 permission frames.

   Note what would retire it rather than paper over it: a deny reason is a
   real feature the design has already drawn, and building it would close the
   gap in the direction the reference points.

## Steps

1. **Give the decided deviations somewhere the judge will actually see them.**
   This step was first written as "add `baseline.json` entries", which is
   **wrong and would not have worked**: `baseline.json` is consulted by the
   six gates, and a judge is handed "the frames, the criteria, the focus set
   and the design reference" — it never reads that file. Writing the entries
   there records the decisions for the harness and changes nothing about the
   rediscovery this spec opens with. The refuted gutter width (Class A item
   11) is the same problem from the other side: a judge working from the
   handoff's prose will keep reporting a defect the HTML refutes.

   So the question is a **protocol** one and wants deciding before the work:
   may a judge be given a standing list of decided deviations and known-stale
   prose? The case for is that it is an addendum to the *design reference*,
   not context about this change — the defence the skill actually names is
   that the judge must not know the diff, the source, or its own earlier
   scores, and a standing list is none of the three. The case against is that
   it is the thin end of "just a bit of context so its feedback is
   actionable", which is the exact phrasing the skill warns about. Decide it,
   then write the list wherever the decision puts it. Until then the cost is
   real and recurring: four judges spent a run's attention on closed
   questions, and one of them was refuted by the reference itself.

2. **Decide the Class B questions, in this order.** ~~(2) the permission
   list's shape~~ — **done 2026-09-19, ADR 0003**, and it closed Class A 4,
   16, 17 and 18 plus Class B 3 and 4 exactly as this step predicted. Next
   is (1) the placeholder;
   then (5) the edit panel; then (7) the scrolled-turn continuation; then (8)
   first run's list width, which is the cheapest of them and has now been
   invented twice. Each ends in an ADR or a design re-sync, per the CLAUDE.md
   rule. (6), the branch, is a `StatusInfo` question and can be taken
   independently — and Class A items 19, 22 and 29 are the same shape as it,
   so whatever settles (6) settles how those are approached too.

   **This is the whole of the remaining work.** Every Class A item that is
   not waiting on one of these is built; see the index at the head of Class A.

3. **Raise the Class C items upstream.** ADR 0002's ramp contradiction (C1) is
   the one that is purely ours to amend; C2 and C4 want the light ladder
   looked at as a whole rather than rung by rung, and C3 wants a measure token.
   Read the `design-sync` skill before any `DesignSync` call.

4. **Fix what is left in `crates/tui`.** Nothing is, as of 2026-09-19 —
   every Class A item that did not wait on something outside this repository
   is built, across three passes, and ADR 0003 then closed the four that were
   waiting on a Class B answer. The four dated Progress entries carry what
   each pass changed and what it cost. This step stays as the place the next
   Class A finding lands.

5. **Re-run the full catalogue and rescore.** Four runs so far —
   `run-1789826989`, `run-1789829088`, `run-1789832849`, `run-1789844710`;
   the last covers only the five permission scenes, so the next full-catalogue
   run is still owed and is what the minimum should next be read from. The loop has never iterated
   (open-tasks entry 10), so this is also the first real exercise of the fix
   half of the harness. Expect the minimum to move in steps rather than
   smoothly: it is a minimum across 72 frames, so it only rises when the *worst*
   frame does, and `approval_large-small-light` at 58 is gated on Steps 2 and 7.

6. **Retire the superseded ledger entries.** Open-tasks 1–4 came here as
    Class A items 6, 1, 14 and 22, measured across the whole catalogue
    instead of two scenes. Two of those (1, the status row's separators, and
    14, the orphan break band) are now fixed and deleted; 6 and 22 are still
    open above. The ledger points here rather than restating them, so it
    wants the same correction.

## Pitfalls

- **Reading a clean gate run as design conformance.** 72 frames, zero
  violations, minimum 58. The gates read declared cells and declared colours;
  nothing in them is a judgement about whether a band is in the right place.
- **Taking a stated number for the right number without measuring it against
  the copy it has to hold.** Item 12's entry named `--pane-commands-w` as the
  bound for first run's option list; 48 cells elides `claude models ·
  ANTHROPIC_API_KEY`, which is the reference's own row, at the reference's own
  120-cell frame. The catalogue is measurements, and a measurement cited from
  a *different* screen is a reading, not a measurement.
- **Scoring a frame for a state the capture script put it in.** A judge read
  `prompt_scoped`'s banded row as the product pre-arming `Always allow` — the
  broadest grant — under a default-deny prompt. It is not: `App`'s
  `decision_selected` starts at 0 and that scene's key script presses `Down`
  three times (`Tab` does not take effect through injected input, which the
  screenshot skill discloses). The frame is honest about what it shows and
  says nothing about the default, so a judge brief for these scenes has to
  say which selection is scripted.
- **Assuming a gate that has never fired is a rule that holds.** The elision
  marker drew `⋯` and `—` — neither in the closed table, neither exempted —
  from the day it was written, through three clean runs, because no frame had
  ever been short enough to elide. Item 15's cap made eight frames elide and
  the gate fired immediately. A clean gate run is evidence about the frames
  that were captured, not about the code that drew them.
- **Ranking across classes by frame count.** It puts "drop the version" —
  which is a live design disagreement, not a bug — above every real defect,
  because it happens to touch 66 frames. Order within a class, never across.
- **Fixing a Class A item that a Class B decision will move.** The option field
  width, the panel's columns and the in-panel key hint — items 4, 16, 17 and
  18, which is every open item but four — are all downstream of a control the
  design has not drawn. Fixing them now is fixing them twice, and the second
  fix will look like a regression.
- **Letting the judge see this file.** It is the classification, the source
  reasoning and the previous scores in one place — precisely the three things
  the blind judge must not have. The judge's allowlist is `.claude/design/` and
  the ADRs.
- **Promoting an undeclared exemption without deciding it.** Writing a baseline
  entry for the top bar's version *records* the disagreement; it does not
  settle it. An entry whose `note` says "we disagree with the design here" and
  cites nothing is the suppression dump the baseline's citation rule exists to
  prevent.
- **Treating a doc comment as a record.** It explains code to a reader of code.
  Nothing that reads frames can see it, which is the whole defect this spec
  opens with.
- **Assuming the `@@` in `fenced_diff` comes from the diff widget.** It does
  not — `ui/diff.rs:64` says the widget emits no hunk header at all. It is
  literal text in a fenced block, and the finding is about two surfaces sharing
  one numbering path, not about a colour.
- **Scoring a screen for a component it does not have.** The `prompt*` family
  has no composer; `5a` disables input deliberately. A brief that tells a judge
  otherwise produces findings about absent things — this run's own brief did,
  and the judge caught it rather than the author.
- **Compensating for token debt in `palette.rs`.** The light wordmark entry
  already in `baseline.json` is the precedent: recorded as an upstream defect
  and "explicitly not to be fixed in palette.rs".
- **Believing this catalogue is the whole gap.** See Out of Scope: three
  substantial surfaces were never rendered in this run.

## Out of Scope

- **Everything the run did not reach.** The `markdown` scene exercises only a
  table — no emphasis, inline code, headings, lists or blockquotes — so
  `--tui-diff-box`'s inline-code role and all five syntax roles
  (`--tui-syn-keyword|call|type|string|number`) are unscored in both themes.
  No shell or read prompt ran, so `5a`'s four-option layout and ADR 0001's
  per-program copy are untested. `✔` and `○` appear in no frame. No running
  tool with stdout, and no gauge. A conformance catalogue built from this run
  is silent about all of it, and that silence is not a pass.
- **The harness's own gaps.** Open-tasks 5–10 — the `breakages` blind spot,
  `role pairing`'s weakness, the unbuilt spatial reference, Tab through
  injected input, the Anthropic adapter, the never-iterated loop. They belong
  to `.claude/spec/mjolnir-screenshot.md`.
- **Functional behaviour.** Nothing here is a claim about what the TUI does,
  only about what it draws.
- **Key encoding.** Untested by construction; a clean run is never evidence
  about it.
- **The 200×50 size as a design target.** The design frame is 120×36. Class A
  item 6 and C3 are about not degrading badly past it, not about designing for
  it.

## References

- .claude/spec/mjolnir-screenshot.md — the harness that produced the
  measurements, its acceptance model, and its disclosed blind spots.
- .claude/spec/mjolnir-tui.md — the surface under test; the 2026-09-06
  Progress entry records what was measured and what it corrected.
- .claude/spec/mjolnir-open-tasks.md — entries 1–4 came here as Class A items
  6, 1, 14 and 22; 1 and 14 are fixed and deleted, 6 and 22 are still open.
- .claude/design/HANDOFF.md — the reference; `IMPORT.md` first, then
  `tokens/cells.css` for the grid. Measure the HTML, never the prose.
- .claude/adr/0001-tool-level-permission-grants.md — what forces the
  five-option list the design has not drawn.
- .claude/adr/0002-markdown-tables-are-drawn.md — and Class C item 1, which is
  a contradiction inside it.
- .claude/skills/screenshot/SKILL.md — the loop's two prohibitions, which this
  spec's three-way split exists to make followable.
- crates/screenshot/baseline.json — where a decided deviation belongs.
- crates/tui/src/ui/grid.rs — the derived constants, and `GROUP_GAP`'s record
  of a three-week misreading.
