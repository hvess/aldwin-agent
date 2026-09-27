# aldwin-open-tasks

Work that is known, understood and not done. Each entry says what was seen, where the evidence is, and what would close it.

**Status:** active — a ledger, not a spec. Nothing here blocks anything else.
**Scope:** everything outstanding. Emptied on 2026-09-27 by the developer's
decision, which set aside the exit rule below for that one clearing: every
entry until then was removed, with the citations to it, stale or not — on
the expectation that what is still true surfaces again through the review
loop and is entered then. The review loop then surfaced six gaps the specs still stated; the
developer settled each the same day — `explain` over staged edits, the
scenes stage 8 could not judge and Aldwin's ungated commits were fixed; an
MCP tool's writes outside the review became its own change (entry 2,
since closed by ADR 0014); undo was decided against (ADR 0009);
the note that the macOS sandbox had never run on a Mac was dropped, since
the developer runs it there. What is still true after that is entered here
afresh.
**Owner:** Maximilian
**Last Updated:** 2026-09-27

An entry leaves this file by being done, or by being decided against — in
which case the decision goes where it belongs (an ADR, or the spec it
contradicts) and the entry says so before it goes.

## TUI

3. **The model's thinking is not drawn.** Thinking streams to the TUI
   (`Event::ThinkingDelta`) and is carried and saved (ADR 0006), but
   `App::apply_event` drops it (`crates/tui/src/app.rs`), so a turn that
   thinks for a long time shows nothing until its reply starts. ADR 0006
   left it undrawn because the design system has no treatment for it, and
   one is not invented locally. The developer's call, 2026-09-27: draw it
   once the design specifies how — a disclosure, say, as `Read 1 file  ›`
   is.

## Review

4. **Two author's-pass checks are still judgement.** The review skill's
   author's loop (aldwin-review.md Decision 18) asks for an example on
   every public function a diff adds, and for every identifier the diff
   removes or renames to be gone from `crates/`, `docs/`, `.agents/` and `AGENTS.md`. Both are
   mechanical, and judges raised each more than once on 2026-09-27; a
   finding a judge produces twice belongs in a deterministic stage (review
   skill, "When a judge is wrong"). Closing it means two stage-2 checks in
   `crates/review/src/stages.rs` over the staged diff. The developer's
   call, 2026-09-27: its own change, after Decision 18.
