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
**Last Updated:** 2026-09-28

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

7. **A question asked mid-round closes a waiting review.** Since
   2026-09-28 a review stays on screen while the agent addresses its
   comments, and the next changeset replaces it in place
   (`Review::carry_from`, aldwin-tui.md Progress 2026-09-28). But
   `Event::QuestionAsked` sets `Mode::Question` whatever holds the screen
   (`App::apply_event`, `crates/tui/src/app.rs`), so an `ask` in that turn
   drops the waiting review, the answer lands in the conversation, and the
   review that follows opens fresh with nothing carried. Closing it means
   drawing the agent's question in the review's bottom band, where the
   discard question already draws (`ui::review::draw`), and keeping the
   review underneath.

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

6. **Frame capture runs only on Linux.** Capture drives foot under sway,
   neither of which runs on macOS, so `Pty::open` refuses there
   (`crates/review/src/pty.rs`) and the compositor fails to start first.
   The crate, the gate and stages 1–5 build and run on a Mac, but a change
   that moves a scene's snapshot needs captured frames for stage 8
   (aldwin-review.md, "The stages"), so a contributor on a Mac cannot pass
   the review for it. Closing it means a terminal and compositor that run
   on macOS behind the same capture, or deciding it against. The
   developer's call, 2026-09-27: kept open, for later.
