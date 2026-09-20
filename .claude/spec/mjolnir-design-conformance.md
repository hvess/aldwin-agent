# mjolnir-design-conformance

The gap between what `crates/tui` draws and what `.claude/design/` specifies, as measured rather than as remembered — and the triage that decides which half of it is a bug.

**Status:** active — **eighteen Class A findings open**, twelve of them new on
2026-09-19 from the fifth full-catalogue run (`run-1789850385`, six blind
judges, 72 frames). **Eleven are reachable by writing Rust in `crates/tui`**,
which is the first time since this catalogue was opened that Step 4 has a
queue of any size. The six standing findings are unchanged: three wait on a
data source or a token that does not exist, one on a copy question, one on a
design answer, one on a Class B decision. Twenty-seven have been fixed or
decided away across six passes; the dated Progress entries below are where
that history lives.

Three findings closed on 2026-09-19 through a **decision**, not a fix: ADR
0003 settled Class B 2, and items 4, 17 and 18 went with it. That is the
pattern this spec predicted — a Class B answer is what unblocks the Class A
findings sitting downstream of it. Item 16 was the same bet and it did not
pay: the decision unblocked it without fixing it, which is the distinction
this spec's own Model section draws and which the first write-up of that pass
got wrong.

**Scope:** conformance of the shipped TUI's rendering to the imported design
system, for the twelve scenes in the screenshot catalogue at three sizes in
both themes. Covers the deviations, the design debt they sit next to, and the
exemption records that are missing. Excludes the screenshot harness itself
(`.claude/spec/mjolnir-screenshot.md`), the design system's own content, and
every functional question.
**Owner:** Maximilian
**Last Updated:** 2026-09-20

**Progress (2026-09-20, the catalogue stopped needing a judge to find most of
it):** No code changed in `crates/tui`. What changed is the harness: seven
gates instead of six, and a suite of assertions read off `HANDOFF.md`'s screen
sections that is now what the exit condition is computed from. The reasoning
and the cost are in `.claude/spec/mjolnir-screenshot.md`'s entry of the same
date; what belongs here is what it did to this catalogue.

**`run-1789890592`, 2m31s, zero judge tokens: six distinct cited findings.**
Four of them — items 34, 35, 41 and 44 — had cost six blind judges 890K tokens
the day before. Two were new:

- **Item 46**, the dark theme's idle option mark at 1.402:1 where the light
  theme's is at the 2.4:1 the design states. Eighteen blind readings across
  five runs never found it, and it also **corrects a claim this catalogue
  made without measuring** — Class C 5 asserts the app's idle mark sits inside
  the handoff's stated 1.6–2.4:1 band, which is true of light and false of
  dark.
- **Class C 11**, which is item 38 **corrected in its direction**. Three
  judges reported the app's 45% panel dim as a deviation from `5a`'s stated
  35%, and this catalogue entered it as Class A. Measured: 35% puts the dimmed
  body at 2.706:1 dark and 1.975:1 light, against the same document's 3.3:1
  ink floor. The reference's own number is the worse one and no opacity
  satisfies both halves. The app is not wrong; the reference is
  unsatisfiable.

**That is now twice in two days that a finding survived several judges and
failed on first measurement** — item 33 (refuted by the handoff HTML) and item
38. Both were reported by three or more independent readers, which is the
corroboration standard this catalogue adopted on 2026-09-19 to guard against
exactly this. Corroboration between readers of the same prose is not
evidence about the prose. **Measure the reference, not only the frame** — the
second half of CLAUDE.md's first design rule, which this spec has now been
caught skipping twice.

**Class C 12 is new and was found by the gate looking where nobody had.** The
diff tints are below the stated adjacency floor against their own field in
`4a`'s transcript — not the panel, which is Class A 32 — with `--tui-add-row`
on `--tui-diff-box` at 1.025:1.

**What the judge is still for**, measured rather than assumed: it was re-run
on the new twelve-frame set after this, and its findings are recorded below.
An assertion suite finds deviation from what somebody wrote down.


**Progress (2026-09-19, the fifth full-catalogue run — twelve new deviations,
eleven of them reachable in `crates/tui`):** Run `run-1789850385`, all twelve
scenes at three sizes in both themes, **no code changed**. Preflight clear,
**all six gates clean on all 72 frames**, regression clean (0 sections moved).
Scored by **six** independent blind judges on two scenes each — the widest
split this spec has used, chosen because the previous four-judge rounds all
covered the permission family twice and `markdown`, `fenced_diff` and `long`
not at all. Minimum **70 spatial / 64 component** against 90; means 82.6 /
73.6. The spatial mean is the highest recorded (77.6 before) and the component
minimum the lowest, which is the shape of a run whose findings are about ink,
copy and height rather than columns.

**This run's purpose was to find deviations, not to move a number**, and the
result is that **Step 4's queue is full again for the first time since it
emptied**: twelve new Class A items, of which eleven need nothing from outside
this repository. That is the opposite of the last three runs, whose repeated
conclusion was that nothing was reachable by writing Rust.

**What the wider split bought.** Every one of the twelve is new, and nine of
them are in scenes the narrower splits had been double-covering away from:
the panel's height (34), the gutter's two numbering spaces (36), the
transcript's dim (38), the elision copy (40), first run's option ramp (42),
the positioning line (45). The three that any split would have found are the
ones three judges hit at once — 34, 35 and 38.

**Corroboration, counted, because this spec has twice entered a single
judge's reading and had to correct it.** Raised by three judges
independently: the panel's height (34), the separator's lost blanks (35), the
transcript's 45% dim (38), and — refuted — the missing `--t-recess`. By two:
the gutter's numbering (36), the tool line's single ink (37), the fact value's
rung (39), the elision copy (40), the panel title (Class B 11). Single-judge
and entered as such: 41, 42, 43, 44, 45.

**Two findings were refuted before they were entered, both by the same rule.**
Three judges reported the permission panel's quoted field as missing
`--t-recess`; `decision.rs:545` already measured the handoff HTML and found
`--t-recess` in the prose twice and the markup zero times. That is item 33,
deleted as *fixed* when it was **refuted** — the distinction Decision 3 exists
to hold — and it is now "What a judge will raise again" item 8. A fourth judge
reported `empty`'s `provider` fact as naming a model where the design names a
provider; `transcript::intro_content` draws `provider · model` whenever the
provider is known, and the scene's fake is reached by a bare `base_url`, so
the catalogue cannot name it. **That one is not a finding about the app at
all** — it is a scene that cannot exercise the row it renders, and it is now
`mjolnir-open-tasks` entry 15.

**A harness gap the judges found by its absence.** `prompt_scoped`'s panel is
byte-identical to `prompt_path`'s, and `decision::queue_note`'s
`(+N more pending)` row appears in **none of the 72 frames**: the scene's
second tool call is logged as running before its prompt is queued, so the
count is 1 when the panel draws. The screenshot skill's own disclosure says
that scene "covers a *queued* second prompt". It does not. Class B 9 records
the design question; open-tasks entry 14 records the branch no scene reaches.

**One correction to this catalogue's own bookkeeping.** The Class A index
named items 16, 30, 31, 32 and 33 as "reachable by writing Rust" in the same
paragraph where the table above it listed 6, 19, 22, 28, 29 and 32 — four of
the five had been fixed and deleted by the time the paragraph was written.
The index is rebuilt below.

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

Items 6–32 were measured in `run-1789826989` unless the entry says otherwise;
**items 34–45 in `run-1789850385`**, the fifth full-catalogue run. Frame counts
are out of 72. Cell indices are 0-based, and the capture cell is 8×18px, so a
cell index times 8 is the pixel column in the PNGs.

**A fixed deviation is deleted from this list.** What it was and how it was
answered is in the Progress entry for the pass that built it; what belongs
here is work, and a list that keeps its own history stops being readable as
one. The exception is a finding that was *not* fixed but answered some other
way — refuted, reclassified, or decided against the design — because a blind
judge will raise it again and the answer has to be somewhere. Those are at
the end of this spec, not here.

| | item | reachable in `crates/tui` |
| --- | --- | --- |
| 6 | nothing caps the body measure | no — Class C 3 |
| 19 | a call blocked on a permission is drawn as running | no — a `ToolActivityStatus` the app does not have |
| 22 | no timestamp row under the speaker label | no — a clock the workspace does not have |
| 28 | the empty state's status row copy | no — a live copy question (`idle` against `ready`) |
| 29 | a tool line's target and summary | no — a `ToolActivityEntry` that carries neither |
| 32 | a hunk's tints are near-invisible on the panel's bar | no — Class B 5 |
| 34 | the panel is content-sized, not 18 rows | **yes** |
| 35 | a separator drops its blanks on a frame with rows to spare | **yes** |
| 36 | a hunk's gutter numbers both sides | **yes** |
| 37 | a tool line's name and target share one colour | **yes** |
| ~~38~~ | ~~the panel dim~~ — reclassified to Class C 11; the reference's own number is worse | no |
| 39 | a panel fact's value is `--tui-body`, not `--tui-value` | **yes** |
| 40 | the elision row's copy disagrees with its own count | **yes** |
| 41 | `empty`'s access values are parted by three cells | **yes** |
| 42 | first run's option rows are two rungs above `more` | **yes** |
| 43 | `approval_large` at 80×24 elides the only added line | **yes** |
| 44 | no blank row between a panel's sentence and its field | **yes** |
| 45 | first run's positioning line is `--tui-dim` | measure the HTML first |
| 46 | the dark theme's idle mark is at 1.402:1, not 2.4:1 | **yes** |

**Eleven of these are reachable by writing Rust in `crates/tui`** — 34 through
37 and 39 through 44 from `run-1789850385`, plus 46 from `run-1789890592`;
38 left the list on 2026-09-20 when it was measured and turned out to be the
reference's problem. They are the whole of Step
4's queue, and it is the first time since this catalogue was opened that the
queue has had more than five entries in it. Item 45 is in the queue only if
the handoff HTML agrees with the handoff prose; measure before building.

Of the other seven, three wait on a data source or a token that does not
exist, one on a copy question, one on a design answer, one on a Class B
decision, and one on a measurement. None waits on a Class B decision that is
not already open.

Items 30-32 are new, added 2026-09-19 from `run-1789844710`'s three blind
judges; each was raised independently by all three, which is why they are
entered without the usual single-judge caution.

Twenty-seven of this catalogue's findings are no longer listed here: 1, 2, 3,
5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 20, 21, 23, 24, 25, 26, 27, 30 and 31
were fixed; 4, 17 and 18 were closed by ADR 0003 deciding the Class B question
they sat downstream of; **33 was not fixed but refuted**, by the handoff HTML,
and is corrected below.
The numbers are not reused — the dated Progress entries above cite them, and
a reader following one of those citations should find a gap rather than a
different finding standing in the deleted one's place. Four of them left
something behind that is still live: item 7's target slot is now part of item
29, and items 10, 11 and 33 left three of the records under "What a judge will
raise again" below.

**Item 33's deletion was miscategorised, and three judges found it within the
day.** It was entered as fixed; what happened is that `decision.rs:545`
measured `Agent TUI v2.dc.html` and found the design's own markup uses
`--t-ground` where its prose says `--t-recess` twice. Decision 3 draws exactly
this line — a fixed deviation is gone from the frames, a refuted one is still
in them — so it belongs under "What a judge will raise again", where it is now
item 8. It is the second finding this catalogue has had to move in that
direction and the first it entered in the wrong one.

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

34. **The permission panel is content-sized where the design fixes it at 18
    rows.** 30 frames. `cells.css:80` states
    `--panel-permission-h: calc(var(--cell-h) * 18)` and `HANDOFF.md:271`
    restates it as prose — the one panel dimension the design gives a token.
    Measured on the 120×36 design frame: the tool prompt runs rows 19–35, **17
    rows**; `approval` runs rows 24–35, **12 rows**; `approval_large` the same
    12. The count is constant across widths, so nothing is reflowing it — the
    panel is simply as tall as what it holds.

    It is not the clamp. `decision::max_height` at 36 rows yields a budget of
    21 body rows against a 24-row ceiling, so every one of these panels fits
    inside its own cap with room over; the design's number is a *height*, and
    the app has no concept of one. Three judges measured it independently on
    three different scenes. ADR 0003's Consequences already say `max_height`
    "should be re-examined against the design's number" — this is that
    re-examination, with the measurement attached, and it is the wrong knob:
    the panel needs a floor, not a lower ceiling.

35. **A separator gives up its blank rows on a frame that had the rows to
    spare.** 8 frames measured, and the rule is shared by two surfaces. At
    80×24 `markdown-small` draws `you` on row 5, the break band on row 6 and
    `harness` on row 7 — the band pressed between two text rows with neither
    of `HANDOFF.md:258`'s flanking blanks — while **rows 3 and 4 sit empty**.
    The arithmetic is exact and the rows are there: the ground band is rows
    3–18, sixteen rows; the content with both blanks restored is sixteen rows.
    `fenced_diff-small` is the same shape (`you` 4, band 5, `harness` 6, row 3
    empty), and `prompt_path-small` / `prompt_scoped-small` the same again.

    The degradation rule itself is deliberate and documented twice —
    `transcript::Transcript::viewport` and `decision::panel_lines`, both
    reasoning that "spacing is cheaper than structure". The defect is the
    *trigger*, not the rule. `viewport` spends its freed rows by slicing the
    block above verbatim, and when what that slice returns is itself blank the
    frame ends up with the same two blank rows in the wrong place — above the
    turn instead of around its boundary. `long-medium` is the same failure
    with the other symptom: two viewport rows left empty at the top of the
    body band while whole blocks were dropped to make room.

36. **A hunk's gutter numbers both sides in one column.** 24 frames
    (`fenced_diff-*`, `approval*-*`). `SYNC.md:84` states the review rule
    outright — "a hunk numbers one side only". `diff::number_lines` gives a
    removed row the old number and an added row the new one, so the column
    reads `12, 13, 12, 13, 14, 15` in `fenced_diff` and `1, 2, 3, 4, 5, 6, 2,
    3` in `approval_large`: it counts down the removals, then restarts. A
    reader cannot tell which space a number is in, because nothing on the row
    says. Two judges on two different scenes.

    Note what is *not* the finding: `fenced_diff`'s `@@ -12,7 +12,9 @@` counts
    disagree with the rows drawn, and that header is fixture text inside a
    fenced block, not the widget's (see Pitfalls). The numbering is the
    widget's.

37. **A tool line's name and target share one colour, and the split is the
    design's only statement about that row's ink.** 36 frames.
    `HANDOFF.md:261` and `:264` split the row three ways — the padded name,
    then "the target — path in primary text for mutations, neutral-300 for
    reads", then the right-flush summary. `transcript.rs:487` builds the name
    and the target as **one span**: `pal.accent_text` while running,
    `pal.body` when done. So `read  (call-1)` is a single run of `#e3dfeb`
    (measured) on a finished call and a single run of `#dfd1fb` on a running
    one, and the mutation/read distinction the design draws has nowhere to
    live. Two judges, one of them solving through the panel's dim to get
    there.

    The rung is a separate question and belongs with Class C 5: the design
    says a running name is *accent-300*, which is `--tui-speaker-you` — the
    user's step — and no ink role resolves to it for an agent row. The split
    is Class A because it needs no ramp decision at all.

38. ~~**The transcript behind a panel dims to 45% where `5a` states 35%.**~~
    **Reclassified 2026-09-20 to Class C 11, and the original entry was wrong
    in its direction.** 30 frames. `HANDOFF.md:269` does say 35% and
    `palette::PANEL_TRANSCRIPT_OPACITY` is `0.45`, and three judges solved the
    blend back to α = 0.450 without seeing the constant. What none of them
    did — and what the first write-up of this entry did not either — is
    measure what 35% would actually produce.

    Computed against the design's own tokens, and now pinned by
    `contrast::tests::the_designs_stated_dim_is_darker_than_the_apps`:

    | opacity | dimmed body, dark | dimmed body, light |
    | --- | --- | --- |
    | `5a`'s stated 35% | **2.706:1** | **1.975:1** |
    | what ships, 45% | 3.566:1 | 2.489:1 |

    So the reference's own number is *further* below the reference's own
    3.3:1 ink floor than the app is, and "fixing" the app to 35% would put
    the only rows on screen that say what the decision is about at 1.975:1 in
    the light theme. The two halves of the reference are not jointly
    satisfiable and the design states no third number. That is Class C, and
    it is entered as Class C item 11.

    **Kept here rather than deleted** because the frames still show a 45% dim
    and a judge holding `HANDOFF.md:269` will score it every time — which is
    what three of them did. The lesson is the one this catalogue keeps
    relearning and got wrong again here: measure the reference before
    reporting a deviation from it.

39. **A panel fact's value is `--tui-body` where `--tui-value` names the
    slot.** 18 frames. `semantic.css:21` defines `--tui-value` (neutral-300 /
    ink-light-3) for "right-flush facts and permission \"off\" values"; the
    `in` row's `~/proj` measures `#e3dfeb` / `#35303e`, which is `--tui-body`,
    neutral-200. The label beside it is correctly `--tui-label`, so the row is
    one rung short of the label/value step every other key/value surface in
    the frame holds. Two judges. `transcript::intro_content` already uses
    `pal.value` for exactly these facts on the empty screen, so the two
    surfaces disagree with each other as well as with the token.

40. **The elision row's copy disagrees with its own count, and editorialises.**
    8 frames (every `prompt*-small`). `decision.rs:685` formats
    `{hidden} more line{s} not shown; deciding doesn't require scrolling
    them` — at `hidden == 1`, which is what every small frame shows, that
    reads *"1 more line not shown; deciding doesn't require scrolling them"*.
    Three defects in one string: the pronoun disagrees with the count, the
    contraction is a register the reference never uses, and the second clause
    tells the developer what their decision requires rather than stating a
    fact. The design's nearest analogue is `HANDOFF.md:262`'s `81 more lines`,
    which is a count and nothing else. Two judges. `diff.rs:357`'s marker is
    already just the count, so the two elision rows disagree with each other
    too.

41. **`empty`'s access values are parted by three cells where the design's
    within-group separator is ` · `.** 6 frames. `read:deny`, `shell:deny`
    and `edit:deny` land on cells 13, 25 and 38 — a constant 3-cell gap
    (`access_spans`' trailing space plus `Span::raw("  ")`). `HANDOFF.md:396`
    parts within-group facts with ` · ` and `--group-gap` parts groups with 6;
    three cells is neither. These are three facts in one group, so the
    reference's own answer applies without inventing anything. (The *content*
    of this row — three states against `14d`'s single tier word — is answered
    deliberately and stays under "What a judge will raise again"; only the
    separator is open. The `--tui-del` half of that entry is fixed: the row
    now measures `--tui-value`.)

42. **First run's real option rows sit two rungs above `more`, where the
    design says one.** 6 frames. `HANDOFF.md:336-337` states the relation
    rather than the values: `more`'s name is `--t-label` and its purpose
    `--t-dim`, "**one step quieter than a real option**". Measured: `more` is
    `#b1adbb` / `#9a95a4` (label / dim, correct), and a real option is
    `#e3dfeb` / `#c9c5d2` — body / quiet, two rungs up each. One step above
    label is quiet; one above dim is label. `5c`'s own unselected name is
    neutral-300, which points the same way. Light mirrors it exactly. The
    relation is stated, so this needs no ramp decision.

43. **`approval_large` at 80×24 elides the only added line.** 2 frames. The
    panel shows `1`, `2 -`, `3 -`, `4 -` and then `4 more lines not shown` —
    arithmetic correct, rows wrong: the hidden four include the single `+` row
    that *is* the change. The developer is asked to approve an edit while
    looking only at its deletions. This is item 26's shape a second time — the
    panel eliding what it is asking about — on the scene built to force the
    clamp. `diff::boxed`'s budget elides from the middle outward; what it
    needs is to keep the added rows, since a hunk with no `+` row visible has
    lost its subject.

44. **No blank row between a panel's sentence and its target field.**
    12 frames (`approval` and `approval_large`). `HANDOFF.md:274-275` is explicit about both
    separators — "Blank row, then the sentence … Blank row, then the command
    block". Measured on `approval-medium`: row 25 blank, row 26 the sentence,
    row 27 the path, row 28 blank. The blank above the sentence and the blank
    below the field are both present; the one between them is not. The tool
    prompt draws all three correctly (rows 20/21/22/23), so the two panel
    builders disagree.

45. **First run's positioning line is `--tui-dim` against a stated
    neutral-300.** 6 frames. `HANDOFF.md:310` — "One line of neutral-300
    prose"; Turn 13's supersession rebuilds first run as "wordmark,
    positioning line, two steps" without restating the tone, so the value
    stands unamended. Measured `#9a95a4` / `#5c5568` = `--tui-dim`,
    neutral-500, two rungs under. `.claude/spec/mjolnir-tui.md` asserts the
    `--tui-dim` choice is faithful to the design, which is the half that
    wants settling first: **measure it in `Agent TUI v2.dc.html` before
    changing anything**, because this is exactly the shape of finding the
    handoff HTML has twice refuted (items 11 and 33). If the HTML says
    `--t-dim`, this is not a Class A item at all — it is a third entry under
    "What a judge will raise again".

46. **The dark theme's idle option mark is at 1.402:1, where the light
    theme's is at the 2.4:1 the design states.** 15 frames, every dark
    permission panel. `--tui-mark-idle` `#5d576a` on `--tui-bar` `#474251`
    measures **1.402:1**; the light theme's `#9a93a5` on `#e8e4ee` measures
    **2.364:1**, which is `HANDOFF.md:109`'s "~2.4:1 on the bar" to two
    decimal places.

    **This corrects a claim this catalogue made without measuring it.** Class
    C item 5 states that the app's idle mark sits "inside the 1.6–2.4:1 band
    the handoff itself states for this glyph at line 109". That is true of the
    light theme and false of the dark one, and line 109 is *the light theme's
    paragraph* — the design states this glyph's band for light and says
    nothing about dark. So the design's floor exists, the light theme meets
    it, and the dark theme is 40% under it on the same element.

    Found by the `contrast` gate on `run-1789890592`, in the first run after
    it was built; no judge has ever reported it, across five runs and
    eighteen blind readings.

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

9. **Nothing says a second decision is queued.** 6 frames
   (`prompt_scoped-*`). The scene issues two tool calls in one assistant
   message; the transcript draws two `◐` rows, and the panel below is
   **byte-identical** to `prompt_path`'s. A developer about to press `⏎`
   cannot tell another decision is behind this one. The design's nearest
   analogue is `5c`'s `commands` title row with `7 of 22` right-flushed
   (`HANDOFF.md:300`); `5a`'s title row spends that slot on the tool name, so
   the design has no answer for a queue depth on a permission panel.

   `decision::queue_note` already writes `(+N more pending)` when
   `pending_prompts.len() > 1`, and it renders in none of the 72 frames —
   the second call is logged as running before its prompt is queued, so the
   count is 1 when the panel draws. That makes this a Class B question with a
   harness gap underneath it: **the branch is unreachable from any scene**,
   which is `mjolnir-open-tasks` entry 14 and not something this catalogue
   can close.

10. **A fenced code block has no language caption and no line count.**
    12 frames (`fenced_diff-*`). `SYNC.md`'s Turn 15 section lists, under
    "Not applied", a `CodeBlock` carrying "language caption, right-flush line
    count, 5-cell gutter, five syntax roles". Two of the four ship — the
    gutter and the `--tui-diff-box` ground — and the component itself was
    never imported, so there is no frame to measure the other two against.
    This is the surface the catalogue's own Out of Scope section has flagged
    as unexercised since the first run: no emphasis, no inline code, no
    headings, no lists, and all five `--tui-syn-*` roles unscored in both
    themes. **It wants a re-sync before it wants a decision**, which makes it
    the cheapest Class B item on this list.

11. **The edit-approval panel is titled `permission`.** 12 frames. ADR 0001
    §2 puts `edit` outside the permissions model entirely — "not a tier, not
    an option, and not expressible as a grant" — and ADR 0003 §1 confirms the
    design has no edit-approval screen at all. The panel nevertheless carries
    `permission` at cell 3 with `edit` right-flushed, so the title names the
    exact model its own ADR says this screen is outside of. Two judges reached
    it independently from the ADRs rather than from the design.

    The rows are *right* and should survive any redesign — `Approve` / `Deny`
    with no tier, exactly what ADR 0001 §2 requires. It is one word, and it
    belongs with Class B 5 rather than beside it: whatever answers the panel
    answers its title.

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

4. ~~**The `5a` recess separator fails in opposite directions per theme.**~~
   **Void on 2026-09-19: the row it measures does not exist.** `--tui-recess`
   is painted in **zero** of `run-1789850385`'s 72 region maps — nowhere in
   the shipped TUI, in either theme. The entry measured a separator the app
   drew when it was written and has since stopped drawing, for the reason
   under "What a judge will raise again" item 8: the handoff's markup has no
   `--t-recess` in `5a` at all, so there is no separator to get wrong.

   Kept rather than deleted because the *token* is still in `semantic.css`
   and still unused, and because the ratios stand if anything ever paints it:
   `#0f0b15` on `#474251` is 2.01:1 and `#ded9e6` on `#e8e4ee` is 1.105:1 —
   one specified row, two opposite failures. The live version of that problem
   is item 9 below, which is the surface the design actually uses.

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

6. **The screen prose is a pre-token colour layer throughout, not in one
   paragraph.** Class C 5 recorded this for `5a`'s option row. Six judges on
   this run, reading four different screens, hit the same wall on seven more
   values — every one of them a place where `HANDOFF.md`'s screen prose names
   a ramp rung and `semantic.css` names a different one, and every one of
   them scored as an app defect by whoever read the prose first:

   | prose | states | token | ships |
   | --- | --- | --- | --- |
   | `:243`, `:258` | the turn break is "the composer's tone" | `--tui-break` | ground-2, two rungs off the composer's ground-4 |
   | `:259` | agent prose is neutral-300 | `--tui-body` | neutral-200, and `:88`'s own table agrees with the token |
   | `:257` | `harness` is neutral-400 | `--tui-speaker-agent` | neutral-300 |
   | `:262` | the inline gutter is `--t-label` **and**, nine lines later, neutral-700 | `--tui-label` | neutral-400 |
   | `:262` | the inline diff is "a recessed field on `--t-recess`" | `--tui-diff-box` | and `semantic.css:57-59` says in as many words that a block does **NOT** use `--tui-recess` |
   | `:264` | the result summary is neutral-700 | `--tui-dim` | neutral-700 is the idle-mark rung and is not ink in the current role set |
   | `:265` | a running tool name is accent-300 | — | accent-300 is `--tui-speaker-you`; no ink role resolves to it for an agent row |

   The frames render the token in all seven. What this wants is not seven
   amendments but the same fix Class C 5 asks for: **the screen prose
   rewritten in semantic roles like the rest of the handoff.** Until then
   every judge spends part of a run rediscovering it, which is now measured
   across four separate runs.

7. **The closed glyph table is contradicted by the design's own copy.**
   Three judges, unprompted. `HANDOFF.md:232-241` closes the table at
   `▌ ● ◐ ○ ✔ ▶ █ + -` and `IMPORT.md` restates it as closed — yet the same
   document mandates `·` as the top bar's within-group separator (`:142`,
   `:254-255`), `→` on first run's `more` row (`:336`), `⏎` and `↑↓` in two
   footers (`:280`, `:341`), and presupposes `…` for clipping. All five are
   drawn, correctly, by frames that are therefore violating the table their
   own reference closed. `baseline.json` already carries them as an exception
   citing the screen prose, which is the right local answer; the
   contradiction upstream is unrecorded, and it is the same shape ADR 0002
   had to carve out by hand for the box-drawing set.

8. **The brand-to-cwd gap is specified twice, six cells apart.**
   `HANDOFF.md:254` (`4a`) says `mjolnir`, then "6 cells", then the working
   directory; `--group-gap` is 6 and `IMPORT.md` says "the 6-cell gap survives
   only between the brand and everything else". `HANDOFF.md:389` (`14d`) says
   `mjolnir`, "three spaces", cwd. Those land the cwd on cell 16 and cell 13
   respectively. Three judges measured cell 13 and split on whether it is
   right. It is — `mjolnir` is 7 cells from the 3-cell margin, so the label
   column ends at 11 and cell 13 is the body column every other row in the
   frame uses — but the reference states both and only one can hold.

9. **`5a`'s command field is `--t-ground` inside a `--t-bar` panel, which
   inverts in the light theme.** Measured off the handoff HTML, not the prose
   (see "What a judge will raise again" item 8): the field is the transcript's
   own ground on the panel's chrome-bar tone. In dark that steps *down*
   (`#27232f` on `#474251`, 1.58:1) and reads recessed. In light it steps
   **up** — `#f7f5fa` is the lightest rung in the ladder, on a `#e8e4ee`
   panel — so the design's own markup produces a white card floating on the
   panel, in a system where "nothing inside a frame is stroked" makes that
   step the entire boundary. Two judges reported it as an app defect and a
   third reported it as the light theme reading backwards. The app is
   conformant; the reference has one surface doing opposite work in its two
   themes. Same family as Class C 2 and 4, and it wants the light ladder
   looked at as a whole rather than rung by rung.

10. **The quoted-code field puts three roles under the stated contrast
    floor.** `SYNC.md`'s Turn 15 note treats **4.67:1** as "the project's
    minimum" when rejecting an added-`+` at 4.51:1. On `--tui-diff-box`
    `#3a3648` the hunk header `--tui-hunk-header` `#a081d5` measures
    **3.66:1**, a context row's code `--tui-context` `#9a95a4` **4.00:1**, and
    a line number `--tui-label` `#b1adbb` on the added row `#3d4b42`
    **4.19:1**. Each role is the *right* role; it is the pairing that is
    unmeasured, because the design never draws a hunk header or a context row
    on the quoted-code ground — `5b`'s hunk is an all-added new file on the
    transcript ground. Note `HANDOFF.md:91` calibrates `--t-dim` at 3.3:1, so
    4.67 may not be a global floor at all, which is itself the thing to
    settle.

11. **The panel dim's stated opacity and the stated ink floor are not jointly
    satisfiable.** `HANDOFF.md:269` — the transcript behind a permission panel
    "stays in place at 35%". `HANDOFF.md:109` calibrates the dimmest ink role
    at 3.3:1. At 35% the dimmed body measures 2.706:1 dark and **1.975:1**
    light; no opacity satisfies both, and the design states no third number.
    Class A item 38 was opened against the app for this and is closed into
    here. What the design needs to say is what a receded-but-still-readable
    surface answers to, since it has one and has never given it a floor.

12. **The diff tints sit below the stated adjacency floor against their own
    field, in the transcript the design itself draws.** 30 frames.
    `--tui-add-row` against `--tui-diff-box` measures **1.025:1**, and
    `--tui-del-row` against it 1.101:1 and 1.123:1. This is not the panel
    case — that is Class A 32 and Class C 2, and it is about a surface the
    design never drew. This is `4a`'s own inline diff on `4a`'s own quoted-code
    field, where the design puts them, below the 1.15:1 `HANDOFF.md:74` claims
    for "every adjacency in the five screens".

    Note what carries the boundary instead, because it is why this is debt
    rather than a defect: the sign and the code colour. `HANDOFF.md:262`
    specifies both for exactly this field. So a changed row is legible — but
    its *ground step* is doing none of the work Turn 13's no-stroke rule
    assigns to it, and `--tui-diff-box` against `--tui-ground` at **1.146:1**
    is the design's own stated value for the field itself, 0.004 under its own
    floor. The ladder wants re-measuring as a whole; see item 2.

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
   ~~**What survives as Class A is the colour, not the content**: each `deny`
   renders `--tui-del`.~~ **Fixed and re-measured 2026-09-19.**
   `transcript::access_spans` now draws both words in `--tui-value`, with its
   own record of why: the two diff hues are the handoff's only foreign
   colours precisely because they mean *diff*, "a screenshot judge read this
   row as deleted lines", and `--tui-value` is the role named for "permission
   \"off\" values". Six judges on `run-1789850385` raised the row's content and
   none raised its colour, which is the confirmation. What is open on this row
   is now its *separator* — three cells where the design parts within-group
   facts with ` · ` — and that is Class A item 41.

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

8. **There is no `--t-recess` anywhere in `5a`'s markup.** ~~Class A item
   33.~~ The single most re-discovered finding of this run: **three of six
   judges raised it**, on three different scenes, each citing
   `HANDOFF.md:275` ("the command block: a recessed field on `--t-recess`")
   and `:277` ("One row of the recessed tone, blank row"). `decision.rs:545`
   answers both from the frame rather than the prose — "the prose uses the
   word twice and the markup zero times (the command field is `--t-ground`)",
   measured off `Agent TUI v2.dc.html` on 2026-09-19. The app is right.

   Two things follow. Item 33 was deleted as *fixed* when what actually
   happened is that it was **refuted**, which is what Decision 3 draws the
   line about — a refuted finding stays visible or the next judge raises it,
   and three did within the day. And the light-theme consequence of the
   design's own choice is real and is now Class C 9; do not let a judge's
   report of it be mistaken for this entry.

9. **The selected option row leaves the ink ramp.** New with this run, and
   the ADR is what makes it re-discoverable. ADR 0003 §1 says the quoted
   pattern draws "**one** step, read off the ink ramp … not by analogy to the
   detail column's `dim`", and states no exception. `decision.rs:627` makes
   one: on the selection band the label is `--tui-text` and the pattern is
   `--tui-accent-text`, "for the reason the detail column already had to move
   off `dim`: on the `band` field it measures 2.62:1". The reasoning is sound
   and the record is in the wrong place — a judge holding ADR 0003 will read
   the selected row as breaking the rule the ADR states, which is what
   happened here. **This wants an amendment to ADR 0003, not a code change**;
   it is the third time a decision recorded only in a doc comment has cost a
   judge's attention.

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

4. **Fix what is left in `crates/tui`.** Eleven items, all new on
   2026-09-19 from `run-1789850385`: **34, 35, 36, 37, 38, 39, 40, 41, 42, 43
   and 44**, with 45 behind a measurement. This is the largest queue this step
   has ever carried, and it arrived on a run that changed no code — the
   previous three passes each concluded that nothing was reachable by writing
   Rust, and what changed was the judging split, not the app.

   Take them in this order, because two of them will move the others:
   **34** (the panel's height) resizes every panel frame and should land
   before 44 and 43, which are rows inside a panel whose height is about to
   change; **35** (the separator's trigger) is shared by the transcript and
   the panel, so it lands once and fixes eight frames in both. Then the ink
   and copy items — 37, 38, 39, 40, 41, 42 — which are independent of each
   other and of everything above. **36** is `diff::number_lines` alone.
   **45** is not work until the handoff HTML is measured; if it agrees with
   `mjolnir-tui.md` rather than the prose, it becomes a "judge will raise
   again" entry instead.

5. **Re-run the full catalogue and rescore.** Five runs so far —
   `run-1789826989`, `run-1789829088`, `run-1789832849`, `run-1789844710`
   (permission scenes only) and `run-1789850385`. The last is the
   full-catalogue run the previous entry said was owed, and its **minimum 70
   spatial / 64 component** is what the next one should be read against.

   The loop has still never iterated (open-tasks entry 10): five runs, and not
   one of them has gone capture → fix → recapture → rescore inside a single
   session. Step 4's queue is now large enough to make that worth doing, and
   it is the first time that has been true.

   Expect the minimum to move in steps rather than smoothly: it is a minimum
   across 72 frames, so it only rises when the *worst* frame does. The worst
   is now `empty` at 64 component in all six of its frames, and three of the
   four findings holding it there — the version, the placeholder, the status
   copy — are decided or Class B rather than buildable. **Fixing all eleven of
   Step 4's items will not by itself reach 90**, and saying so before the pass
   rather than after it is the point of writing this down.

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
- **Reading a scene's fixture as the app's behaviour.** `empty`'s `provider`
  row draws only the model, which looks exactly like the app dropping the
  vendor; `intro_content` draws `provider · model` whenever the provider is
  known, and the scene reaches its fake through a bare `base_url`, so there is
  no vendor to name. A judge cannot tell those apart and should not be asked
  to. Check the scene before entering a content finding.
- **Reading a finding's absence from the frames as its absence from the
  code.** `decision::queue_note` writes `(+N more pending)` and appears in
  none of the 72 frames, because no scene ever has two prompts pending at
  once. A row no capture reaches is not a row that works.
- **Deleting a refuted finding as a fixed one.** Item 33 was refuted by the
  handoff HTML and deleted as fixed; three judges raised it again within the
  day, because the frames still show what they were measuring. Decision 3 is
  the rule and this is the case that proved it needs enforcing.
- **Believing this catalogue is the whole gap.** See Out of Scope: three
  substantial surfaces were never rendered in this run.

## Out of Scope

- **Everything the run did not reach.** The `markdown` scene exercises only a
  table — no emphasis, inline code, headings, lists or blockquotes — so
  `--tui-diff-box`'s inline-code role and all five syntax roles
  (`--tui-syn-keyword|call|type|string|number`) are unscored in both themes.
  No shell or read prompt ran, so `5a`'s four-option layout and ADR 0001's
  per-program copy are untested. Glyph census on `run-1789850385`: `✔`, `█`
  and the diff signs appear in no frame, and `○` now appears in six (first
  run's idle rows) — so this sentence was half stale within four runs, and the
  census is worth re-running each pass rather than remembering. No running
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
