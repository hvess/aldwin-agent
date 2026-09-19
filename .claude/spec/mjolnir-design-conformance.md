# mjolnir-design-conformance

The gap between what `crates/tui` draws and what `.claude/design/` specifies, as measured rather than as remembered — and the triage that decides which half of it is a bug.

**Status:** active — the catalogue is complete and classified; the unblocked
colour, spacing and content deviations are fixed (see the 2026-09-19 Progress
entry), the layout and Class B/C work is not.
**Scope:** conformance of the shipped TUI's rendering to the imported design
system, for the twelve scenes in the screenshot catalogue at three sizes in
both themes. Covers the deviations, the design debt they sit next to, and the
exemption records that are missing. Excludes the screenshot harness itself
(`.claude/spec/mjolnir-screenshot.md`), the design system's own content, and
every functional question.
**Owner:** Maximilian
**Last Updated:** 2026-09-19

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

3. **A judge finding that the source refutes is still recorded.** Four
   independent judges flagged the version in the session top bar; the code
   answers it deliberately at `ui/chrome.rs:109-112`. Deleting the finding
   would guarantee a fifth judge raises it. It is recorded as an undeclared
   exemption, which is what it is.

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

Measured in `run-1789826989`. Frame counts are out of 72. Cell indices are
0-based, and the capture cell is 8×18px, so a cell index times 8 is the pixel
column in the PNGs.

1. **The status row parts its facts by 2 cells.** 36 frames (every scene with
   a bottom bar). `ui/chrome.rs:302,312,322` bake `"  "` into the format
   strings, so `idle`, the model, the turn counter and the message count are
   parted by two cells. The design has exactly two spacings and this is
   neither: ` · ` within a group, `--group-gap` = 6 between groups.
   `ui/grid.rs:58`'s own comment states the distinction and records that
   Mjolnir shipped a misreading of it for three weeks. The four facts read as
   one undifferentiated run. **This is open-tasks entry 2, now measured at
   every size.**

2. **The status row is one ink tier too loud.** 36 frames. Every span is
   `pal.label`, including the right-flush `^c to exit`. `14d` puts both sides
   of that row in `--tui-dim`. Idle chrome currently outranks the agent's own
   quiet labels.

3. **The hardware cursor lands on the placeholder's first character.** 36
   frames. `ui/chrome.rs:437-446`: when the draft is empty the app draws
   `▶  ` plus placeholder text from cell 6, then calls `set_cursor_position`
   at `inner.x + PROMPT_PREFIX_LEN` — also cell 6. Two things claim one cell.
   In an unfocused terminal it is an outline around the `A`; in a focused one
   the block is filled and the `A` is gone. This is the "two runs colliding
   inside the body column" class the `breakages` gate explicitly cannot see
   (open-tasks entry 5), caught here by eye. Independent of the separate
   question of whether a `▌` should be drawn at all, which is Class B below.

4. **The decision panel does not use `OPTION_LABEL_COL`.** 30 frames
   (`approval`, `approval_large`, `prompt`, `prompt_path`, `prompt_scoped`).
   `ui/grid.rs:46` defines the 16-cell option name field and its comment says
   "One width for every list in the system: the provider list, the model list,
   the access list and the command list are one control, so they share it."
   `grep` puts every use of it in `ui/first_run.rs`; `ui/decision.rs` never
   imports it. Measured consequence: the detail column lands at cell 16 in the
   `approval` family (a 10-cell field) and cell 31 in the `prompt` family (a
   25-cell field, widened to fit the string `Allow for this session`). Neither
   is 22, which is what the constant gives. One control, three widths.

5. **The selected option's detail is a neutral on the accent band.** 30
   frames, binding in the 15 dark ones. Measured `--tui-dim` `#9a95a4` on
   `--tui-band` `#604788` = **2.62:1**, against a palette that states dim
   holds 3.3:1. The design's role is `--tui-accent-text` `#dfd1fb` = 5.32:1 on
   the same band, and the Turn 14 note is explicit that "the selected row's
   purpose text is `--t-accent-text`, not `--t-quiet`". Light escapes at
   4.64:1, so this is a dark-theme failure — the reverse of the usual pattern.
   Two judges measured it independently to the same hundredth.

6. **Nothing caps the body measure.** 18 frames (every `*-large-*`), with the
   200×50 size the only one that shows it. Assistant prose runs cols 13–197 as
   a single 183–185 character line; a diff field spans 184 cells for 62 cells
   of code; in `tools-large` a call ends at cell 27 and its own right-flushed
   summary occupies 180–196, 152 cells away, reading as two unrelated facts.
   All four judges raised it. **This is open-tasks entry 1**, which measured it
   in one scene; it is in every scene that renders prose. Note the design
   frames are 120 cells and state no maximum measure, so the *token* is Class
   C (item 3 below) while the unbounded layout is Class A.

7. **The tool-call line is one space short and unpadded.** 36 frames
   (`tools`, `approval`, `approval_large`, `prompt` family). Renders glyph at
   cell 13, name at 15, target at 20. `HANDOFF.md:260` is "glyph, 2 spaces,
   tool name padded to 6 characters … then the target", which puts them at 16
   and 22. Two judges, identical cells, different scene sets. **Fixed
   2026-09-19**, with the 6 treated as a minimum rather than a width: a name
   of 6 or more characters would otherwise touch its own target.

   What the judges *also* raised — that the target field holds `(call-1)`, an
   internal identifier with no counterpart in the design, where the reference
   shows the file or command — is **not a layout fix and is deferred**.
   `log.rs:49`'s `ToolActivityEntry` carries `call_id`, `name` and `status`
   and nothing else, so the real target is not available to render. Same shape
   as Class B item 6: a data source, not a rendering defect.

8. **The agent's tool call is grouped into the developer's turn.** 18 frames.
   The call renders above the `--tui-break` band, so it belongs to the `you`
   turn and its label column is empty; `4a` makes the tool group part of the
   agent's turn. Visible as an unattributed row at cells 3–10.

9. **A fenced diff block is run through the inline-diff numbering.** 12 frames
   (`fenced_diff`, `long`). `ui/diff.rs:64` states that
   `mjolnir_tools::diff::unified` emits no `@@` header, "so there's no
   absolute file offset to anchor on" — and `number_lines` therefore numbers
   from 1 relative to the shown diff, per explicit developer request. That
   reasoning is sound for the inline diff widget. These frames are not that:
   the `@@ -12,7 +12,9 @@` is literal text inside a ```` ```diff ```` fence in
   assistant prose. The result is a gutter number `1` printed on a header row
   that is not a line of the file, and a gutter reading 1, 2, 3, 2, 3, 4, 5
   with nothing saying which side each indexes. The header also renders
   `--tui-dim`, the same tone as the gutter, where the inline diff's header
   role is `--tui-hunk-header`. **The fix is not a colour**; it is that two
   different surfaces are sharing one numbering path.

10. **Diff context rows render code at `--tui-context`.** 12 frames.
    ~~Class A~~ — **reclassified 2026-09-19 to a live disagreement.** The
    judges measured `#9a95a4` and read it as `--tui-dim`; the role actually
    used is `--tui-context`, which `semantic.css` defines as
    `var(--color-neutral-500)` — *the same value as `--tui-dim`*, so the
    measurement was right and the inference was not. `ui/diff.rs`'s own
    comment gives a reason ("only the changed lines should compete for
    attention"), and the design never draws an inline diff's context rows at
    all: `5b`'s hunk is an all-added new file. What made it a defect was the
    measured collapse — context code the identical colour to the gutter
    number beside it — and that is **resolved by item 11**, which moves the
    gutter to `--tui-label`. Revisit only with evidence from a frame, not
    from the token name.

11. **The diff gutter is one rung too quiet.** 24 frames. Renders
    `--tui-dim`; `HANDOFF.md` names the role *and* its reason — "a 5-cell
    right-aligned line number in `--t-label` … the neutral label step holds
    3.5:1 for the gutter". **Fixed 2026-09-19.**

    The **width** half of this finding is **refuted**. Two judges reported the
    number as one cell narrow — right-flushing at cell 6/16 where a 5-cell
    gutter would put it at 7/17. They were reading the handoff's prose ("a
    5-cell right-aligned line number"); `ui/diff.rs:160` cites the handoff's
    *HTML* — `flex: 0 0 45px; text-align: right; padding-right: 9px` — which
    is a 5-cell field carrying a 4-cell number and one cell of separation.
    The code is correct and the prose is loose, which is exactly the trap
    CLAUDE.md names: **measure the handoff HTML; reading it is not enough.**
    A judge given only the prose will keep reporting this; it belongs in
    whatever record the judge is eventually given (see Steps, item 1).

12. **`first_run`'s selection band is bounded by the frame, not by its list.**
    6 frames, severity scaling with width: cols 30–76 at small, 30–116 at
    medium, **30–196 at large** — 167 cells of `--tui-band` for an option row
    whose content ends at cell 63, making it the largest coloured area in the
    frame. The rule it breaks is that the accent is "a mark or a line, never a
    filled field", and the band's only analogue in the system (`5c`'s command
    list) is bounded by `--pane-commands-w`. The `more →` affordance on row 15
    has the same cause and is 127 cells from its own label at large.

13. **`first_run` renders the cwd one rung quieter than every other screen.**
    6 frames. `#9a95a4` (`--tui-dim`) there against `#c9c5d2` (`--tui-quiet`)
    in `empty` and `conversation`, for the identical string; light mirrors it.
    One component, two tones.

14. **A dropped turn keeps its break band.** 4–6 frames at 80×24. In
    `markdown-small` the transcript band is 16 rows and the content needs
    exactly 16, yet row 3 renders blank and the break band sits at row 5 —
    the layout reserved a row for a `you` turn it then did not draw, so the
    frame opens on two dead rows and a band separating nothing from nothing.
    In `fenced_diff-small` the same shape is genuinely one row over budget,
    and dropping the band with the turn it belongs to is what recovers it.
    **This is open-tasks entry 3**, now with the row arithmetic.

15. **At 80×24 the approval panel leaves one row of transcript.**
    2 frames (`approval_large-small-*`). Panel occupies rows 4–23, 83% of the
    frame; the transcript band is `rows 3..3`, holding a dimmed tool-call row
    with the `you` turn clipped away entirely. `5a`'s stated reason for being
    a bottom panel rather than a modal is that "the transcript above stays in
    place at 35%".

16. **The permission panel ignores the frame's own columns.** 30 frames. No
    content inside the panel sits at cell 13: the target value lands at cell 9
    in the `prompt` family (inside the label column's own run) and at cell 3 in
    the `approval` family, while the transcript three rows above uses the body
    column correctly. `5a` specifies the panel's facts as a key/value table on
    the frame's columns. The panel and the frame read as two grids.

17. **The two rule literals do not form a column.** 12 frames (`prompt_path`,
    `prompt_scoped`). Adjacent rows state the rule a saved answer would write
    (cell 35) and the rule Tab would narrow it to (cell 41) — two lines whose
    entire purpose is to be compared, misaligned by 6 cells, on columns
    derivable from nothing in `cells.css`. No gate sees it: nothing overprints
    and nothing crosses the right margin.

18. **The in-panel `Tab` hint uses neither the footer's idiom nor its tones.**
    12 frames. Every span measures `--tui-dim` at 3.33:1, the ramp's floor,
    with the key in a 5-cell field matching no constant — while the footer two
    rows below draws key hints correctly, keys in `--tui-mark` and verbs in
    `--tui-quiet`. The same product draws one control two ways, two rows apart.

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

20. **`T1` and `-` in the status row.** 36 frames. `ui/chrome.rs:306-310`
    formats the turn/step as `T{t} S{st}`, falling back to `"-"`. The Content
    Fundamentals require lowercase labels, and `-` borrows a diff sign to mean
    "no value".

21. **The panel's sentence has no terminal period.** 30 frames. Renders `The
    agent wants to read a file`; `5a`'s copy is `The agent wants to run a
    shell command.` Third person and sentence case are both correct — the stop
    is the only thing missing.

22. **No timestamp row under the speaker label.** Every frame that draws a
    label. `HANDOFF.md:257` puts the speaker on the label column's first row
    and the time on its second; the second row is empty in every frame, so a
    turn reads one row tall where the design makes it two. **This is
    open-tasks entry 4**, which asked whether it was deliberate or never
    built. Nothing in Turns 13–15 retires it and `09:42` fits the narrowed
    8-cell column, so it is unbuilt rather than decided against — but that
    conclusion is from the design side only and wants confirming before it is
    built.

23. **The inline diff splits the sign from the code.** 24 frames, binding in
    the light ones — and **missed by every judge**, because it needs the
    handoff's own reasoning rather than a measurement. `HANDOFF.md`: "Both the
    sign and the code take `--t-add-code` here rather than the sign/code split
    the review pane uses, because a tinted row over the recessed field is the
    darkest backdrop in the light theme and the mid-lightness sign green
    measures only **2.7:1** on it; the code colour holds 4.8:1 light and 5.9:1
    dark. The review pane's own hunk keeps the full three-role split … because
    its diff sits on the much lighter transcript ground." `ui/diff.rs`
    implemented the review pane's rule on the inline diff, and its doc comment
    asserted the opposite of the handoff in as many words. The review pane
    (`5b`) is not built, so nothing needed that branch. **Fixed 2026-09-19.**

24. **`justified_line` overflowed its width and was clipped by the frame.**
    Found while fixing item 7, not by a judge. `ui/grid.rs`'s right-flush
    helper fell back to a one-space gap and returned a line *longer* than the
    width it was given; ratatui then clipped it at the frame edge, so at 80
    columns a tool row read `42 matches across 17 fil` — well-formed output
    with its tail silently gone, and no `…` to say so. The margin was one
    cell, so widening the name field by the two cells item 7 asks for was
    enough to cross it. This is the **third** time this codebase has recorded
    that two groups sized independently cannot keep a gap between them
    (`chrome::identity_bar_row`, `draw_status_line`). The right group now
    elides. **Fixed 2026-09-19.**

    Note what caught it: not a gate, not a test, and not a judge — a line-by-
    line read of the snapshot diff before blessing it. The `breakages` gate
    cannot see text truncated with a well-formed ellipsis (open-tasks entry
    5); it equally cannot see text truncated *without* one when the clip
    happens at the frame boundary.

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

2. **The five-option permission list.** `5a` draws four options as single
   sentences with the matched pattern quieter. ADR 0001's three scopes plus
   "once" and "deny" force five, and the footer correctly reads `1-5 to pick`.
   What ships is additionally a *different control*: name + description pairs,
   which is `5c`'s command-list shape, not `5a`'s. Class A item 4 is the
   direct consequence — a field width cannot be got right for a control the
   design has not drawn. **The design has not been re-synced since ADR 0001.**

3. **The grant-summary row.** ADR 0001 requires the prompt to "state the grant
   it would write, not the command that triggered it". The design system has
   no grant-summary component, so the row exists because an ADR mandates it and
   is drawn at `--tui-dim`, 3.33:1 — the literal rule about to be persisted is
   the quietest text in the panel, which reads as the opposite of the ADR's
   intent.

4. **The `Tab` scope-toggle row.** ADR 0001 makes scope directory × duration;
   the design has no representation of widening or narrowing a grant, and its
   key hints are footer-only. Both the widen and narrow variants are local
   inventions. Class A item 18 is how that shows up.

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

2. **The light ladder undercuts its own stated floor.** The handoff states
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

## Undeclared exemptions

Decisions that exist, are reasoned, and are recorded only in doc comments.
Each is scored as a defect by every blind judge and will be again. **Promoting
these to `baseline.json` is the highest-value work in this spec** — it is
cheap, it is not a code change, and it is what stops the next four judges
spending their attention on closed questions.

1. **The version in the session top bar.** All four judges flagged it, citing
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

2. **The `access` row's three permission states.** Judges scored
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

3. **The option row's mark and number inside the 3-cell margin.** Already a
   baseline entry for the `layout` gate, cited to `5a`. Listed here because it
   is the model the other entries should copy: scope named, authority cited,
   and an explicit boundary ("Scoped to the left margin only: nothing licences
   running off the right").

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

2. **Decide the Class B questions, in this order.** (2) the permission list's
   shape, because Class A items 4, 17 and 18 all resolve downstream of it and
   fixing them first means fixing them twice; then (1) the placeholder; then
   (5) the edit panel; then (7) the scrolled-turn continuation. Each ends in an
   ADR or a design re-sync, per the CLAUDE.md rule. (6), the branch, is a
   `StatusInfo` question and can be taken independently.

3. **Raise the Class C items upstream.** ADR 0002's ramp contradiction (C1) is
   the one that is purely ours to amend; C2 and C4 want the light ladder
   looked at as a whole rather than rung by rung, and C3 wants a measure token.
   Read the `design-sync` skill before any `DesignSync` call.

4. **Fix the colour and tone deviations.** Class A items 2, 5, 10, 11 and the
   `--tui-del` half of undeclared exemption 2. These are role changes with no
   layout consequence, they are the cheapest thing in the spec, and item 5 in
   particular is one token against a measured 2.62:1.

5. **Fix the spacing and column deviations.** Class A items 1, 4, 7, 11, 13,
   17, 20, 21. Item 4 waits on Step 2.

6. **Fix the composer cursor collision.** Class A item 3, independent of the
   Class B placeholder decision — whatever the composer draws, two things must
   not claim cell 6.

7. **Fix the layout deviations.** Class A items 8, 12, 14, 15, 16, 19, 22.
   Item 16 waits on Step 2; item 22 wants its "unbuilt, not decided against"
   reading confirmed first.

8. **Cap the measure.** Class A item 6, once C3 gives it a value to cap to.

9. **Re-run the full catalogue and rescore.** The loop has never iterated
   (open-tasks entry 10), so this is also the first real exercise of the fix
   half of the harness. Expect the minimum to move in steps rather than
   smoothly: it is a minimum across 72 frames, so it only rises when the *worst*
   frame does, and `approval_large-small-light` at 58 is gated on Steps 2 and 7.

10. **Retire the superseded ledger entries.** Open-tasks 1–4 are this spec's
    Class A items 6, 1, 14 and 22, measured across the whole catalogue instead
    of two scenes. Point them here rather than restating them.

## Pitfalls

- **Reading a clean gate run as design conformance.** 72 frames, zero
  violations, minimum 58. The gates read declared cells and declared colours;
  nothing in them is a judgement about whether a band is in the right place.
- **Ranking across classes by frame count.** It puts "drop the version" —
  which is a live design disagreement, not a bug — above every real defect,
  because it happens to touch 66 frames. Order within a class, never across.
- **Fixing a Class A item that a Class B decision will move.** The option field
  width, the panel's columns and the in-panel key hint are all downstream of a
  control the design has not drawn. Fixing them now is fixing them twice, and
  the second fix will look like a regression.
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
- .claude/spec/mjolnir-open-tasks.md — entries 1–4 are superseded by this
  spec's Class A items 6, 1, 14 and 22.
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
