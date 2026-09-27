---
name: records
description: How to keep Aldwin's records true while changing the code — the open-tasks ledger, the specs, the ADRs, and the design contradictions in baseline.json. Says when each must change in the same commit, how an entry leaves the ledger, what needs the developer's call, and how to check that nothing still points at what moved. Use whenever a change completes, discovers, or decides work, and whenever editing anything under .claude/spec/, .claude/adr/ or crates/review/baseline.json.
---

# Records

The records are how the next session knows what is true. A record that
disagrees with the code, or points at something that is gone, is a defect
of the change that left it so. Keep them in the same commit as the code.

## Which record holds what

| Record | Holds | Changes when |
|---|---|---|
| `.claude/spec/aldwin-open-tasks.md` | Known, understood, undone work, each with its evidence | Work is discovered, done, or decided against |
| `.claude/spec/aldwin-<crate>.md` | A crate's design, Decisions, Pitfalls, dated Progress | A change completes a step, changes a stated rule, or settles a question the spec covers |
| `.claude/adr/NNNN-*.md` | One decision that alters a constraint or a persisted format | A new decision (a new ADR), or a factual slip in an existing one |
| `crates/review/baseline.json` `contradictions` | The design disagreeing with itself or with Apple's HIG, and which half the app follows | The design is re-synced or a contradiction is found in it |

## In the same commit

- **Discovered work enters the ledger** with its evidence (file and
  function), when the change does not do it. Never leave it only in a
  commit message, a Progress note or a report.
- **Done work leaves the ledger.** Remove the whole item, or the part the
  change completes, and say which parts remain.
- **A touched spec gets its date.** Bump `**Last Updated:**`, and record
  what changed as a dated Progress entry or a Decision. A dated Progress
  entry is history: add a pointer to the newer rule (`two to four since
  2026-09-27: see Decisions`) rather than rewriting it.
- **A changed fact is changed everywhere it is stated.** Grep the old
  wording across `crates/` and `.claude/` — specs, ADRs, the system prompt,
  tool descriptions, doc comments — and fix each one, or give it a pointer.

## Leaving the ledger

An item leaves by being done or by being decided against. Before it goes:

- **Every part is accounted for.** An item naming two things (`GIT_DIR` and
  `GIT_INDEX_FILE`) is closed only when both are.
- **Decided against is the developer's call.** Keeping behaviour a finding
  called wrong, narrowing a design rule, or recording something as intended
  needs their answer first, asked in one question with a recommendation.
  Their decision then goes where it belongs, dated and attributed ("The
  developer's call, 2026-09-27"): an ADR, or a Decision in the spec it
  touches.
- **Every citation moves with it.** Grep for the entry's number and its
  subject; a pointer to a closed entry is updated in the same change.

## What each record is not for

- **A baseline contradiction is not a place for a finding.** It needs two
  quoted halves: the design against itself, or against the HIG. A finding
  about the code is fixed or entered in the ledger (`aldwin-review.md`
  Decision 7 and Pitfalls).
- **An ADR is not rewritten.** Fix a factual slip in place — a wrong count,
  a wrong section number — and record a changed decision as a new ADR that
  supersedes the old one.
- **A Progress note is not a summary of intent.** It states what landed,
  in words that stay true: "fixed or settled", not "each fixed with a test",
  when some were settled by a decision.

## Checking before `/review`

1. `git diff --stat` touches `crates/`: does the ledger gain or lose
   anything, and does any spec state what changed?
2. Every path, test name, ADR section, Decision number and ledger entry the
   diff writes resolves: grep it.
3. Every closed item's citations are gone or updated.
4. Every "decided against" or "intended" carries the developer's answer.
