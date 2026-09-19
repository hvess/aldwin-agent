# ADR 0003 — A permission option is a sentence that states its own rule

**Status:** accepted, 2026-09-19
**Amends:** ADR 0001 §3 — scope stops being an orthogonal axis the developer
toggles and becomes a property of the row they pick
**Affects:** `mjolnir-tui`, `mjolnir-design-conformance` (Class B 2, B 3, B 4)

## Context

`mjolnir-design-conformance`'s Class B 2. The design system's permission
screen `5a` draws each option as **one sentence** with the matched pattern a
step quieter — `Allow cargo test for this session`, `Always allow cargo * in
this project`. What shipped is a **name + detail pair**:

```
3  Allow for this project  saved to .mjolnir/permissions.yaml
```

That is `5c`'s command-list control, on `5a`'s screen. The count is not the
problem — ADR 0001's three scopes plus once and deny make five where the
reference draws four, and the footer correctly reads `1-5 to pick`. The
*shape* is, and it is the root of three catalogued deviations at once:

- **A4**, one control at three widths, because no field width can be right
  for a control the design has not drawn;
- **A16**, the panel's facts on their own grid rather than the frame's;
- **A17**, two rule literals six cells apart on columns derivable from
  nothing in `cells.css`.

Under the pair shape each of those is unanswerable rather than merely
unfixed. The design has not been re-synced since ADR 0001, so `5a`'s copy
predates the ADR that changed what a grant *is* — but its form does not
depend on the grant unit, and that is what this record settles.

## Decision

### 1. A *Tool prompt's* option row is a sentence

One column. No name field, no detail column. The pattern inside the sentence
draws one step quieter than the rest of it, per `5a` — **one** step, read off
the ink ramp (`semantic.css`: body is neutral-200, `quiet` neutral-300, with
`label` and `dim` two and three rungs down), not by analogy to the detail
column's `dim`.

**Scope, because a first draft of this section did not state it and a blind
judge read it the wider way.** This governs the option list of a
`PromptPayload::Tool` prompt — the screen `5a` actually draws. It does **not**
govern the edit-approval panel's `Approve` / `Deny` rows, which keep their
detail column for now: `edit` is outside the permissions model entirely
(ADR 0001 §2), those rows quote no grant pattern, and the design system has no
edit-approval screen at all. That is `mjolnir-design-conformance`'s Class B 5,
still open, and it is the right place for the question rather than an
inference from this record. Nor does it govern first run's catalogue rows,
which are `5c`'s pair by design.

`OPTION_LABEL_COL` is consequently first run's alone, which is what `grep`
already showed — the constant's comment claimed four lists shared it while
`ui/decision.rs` never imported it. The permission list and the provider list
are now two controls **because the design draws two**, and `OptionRow`'s doc
comment loses the invariant it was asserting.

### 2. Scope is per row, not a toggle

`5a` scopes each sentence independently: its session row quotes the
invocation, its project row quotes the program wildcard. Mjolnir built one
pattern for the whole list, flipped by `Tab`. The list now reads:

| # | sentence | tier | pattern |
| --- | --- | --- | --- |
| 1 | `Allow once` | `Once` | — nothing is saved |
| 2 | `Allow <target> for this session` | `Session` | the exact target |
| 3 | `Always allow <broad> in this project` | `Project` | the broad unit |
| 4 | `Always allow <broad> everywhere` | `Always` | the broad unit |
| 5 | `Deny` | — | — |

`<broad>` is ADR 0001's unit unchanged: the enclosing directory for a
path-shaped tool, `argv[0] *` for `shell`. Where a target has no broader form
— a bare filename, a command with no program token — rows 3 and 4 quote the
target itself, as `broad_pattern` already returns `None` for exactly that.

**`Tab` and `PatternScope` are removed.** They were the mechanism for an axis
that no longer exists.

### 3. The grant-summary row goes with it

ADR 0001 required the prompt to "state the grant it would write, not the
command that triggered it". Under §2 **every allow row states its own rule in
its own sentence**, which discharges that requirement more directly than a
separate row ever did — the developer reads the rule on the option they are
about to pick, not two rows above it in `--tui-dim`, the quietest text in the
panel. `GrantSummary`, `GrantUnit`, `decision_grant` and `GRANT_RULE_MAX` are
removed along with the two rows they fed.

This closes Class B 3 and B 4, neither of which had a design counterpart, and
A17 and A18 with them — both were measurements of rows that no longer exist.

### 4. Deny keeps Mjolnir's copy, not `5a`'s

`5a`'s fourth option is `Deny and tell the agent why`. `PromptResponse::Tool`
carries a decision, a tier and a pattern — there is no reason field and no
step that collects one (`app.rs:1152`). Shipping that sentence would name a
thing the product does not do.

This is the A28 precedent applied a second time: a hint naming a key that
does nothing is worse than one that disagrees with the reference. The row
reads `Deny`, and the gap is recorded under "What a judge will raise again"
rather than closed, because a blind judge holding `5a` will score it.

## Consequences

**The session tier gets narrower, and this is the real cost.** Row 2 grants
the exact target, so a developer who runs `cargo test -p tui` and then
`cargo test -p llm` is asked twice in one session — the per-invocation
friction ADR 0001 was written to remove, reintroduced on one row. It is
bounded: rows 3 and 4 still grant `cargo *`, so the only want it frustrates
is broad-but-not-persisted. Taken deliberately, with `5a`'s own copy as the
authority; if it bites in use, the fix is to widen row 2 to the broad unit
and accept that rows 2–4 then differ only in duration.

**The panel stops naming the file a grant lands in.** The detail column said
`saved to .mjolnir/permissions.yaml` and `saved to ~/.mjolnir/permissions.yaml`;
the sentences say `in this project` and `everywhere`. The two tiers stay
distinguishable — by reach, which is the fact that governs what the agent may
do — but the literal path is gone from the screen, and `5a` has no place to
put it: its footer's right-flush `saved to …` is the slot, and Mjolnir
dropped that deliberately because it was true of one tier out of five. Taken
as the price of the sentence form rather than fixed by re-inventing the
column. `ui/tests.rs`'s `every_option_states_its_own_rule_and_its_reach` pins
it so it stays a decision.

**Two scope/duration combinations are no longer expressible**: session over
the broad unit, and project over the exact target. Neither had a row of its
own before either — they were reachable only by pressing `Tab` first, which
is the affordance being removed.

**The panel's row budget drops by three** (the summary row, the `Tab` row and
their padding blank). `ui/decision.rs:42-48` justifies capping the panel at a
quarter of the frame rather than `5a`'s stated half *because* those rows made
Mjolnir's content need ~20 rows where the reference needs 18. That argument is
now spent, and `max_height` should be re-examined against the design's number.
**Deliberately not changed in the same pass**: two geometry changes at once
make the next screenshot delta unreadable about which caused what.

**`broad_pattern`, `directory_glob` and `program_glob` are unchanged**, and so
is the engine's `kind:pattern` matching. Nothing about persisted grants moves;
this is a change to what the TUI offers, which is the same surface ADR 0001
said it was changing.

## Alternatives rejected

- **Sentence form, keeping `Tab`.** Closes B2 and A4 but leaves B3, B4, A17
  and A18 open, and produces a panel where four sentences quote a pattern that
  a fifth row says is about to change — the rule stated twice, in two
  registers, one of them mutable.
- **Shorten the labels to fit `OPTION_LABEL_COL`** (`once` / `this session` /
  `this project` / `always` / `deny`). Cheapest, and makes the 16-cell field
  honestly shared — but it is a copy decision taken to satisfy a constant,
  and it keeps the pair control the design does not draw on this screen.
- **Declare the pair shape and give it a width, via re-sync.** Honest, but it
  invents a token locally to do it: `Allow for this session` is 21 cells
  against the constant's 16, which is the failure mode Class C 3 already
  records.
