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
**Last Updated:** 2026-09-29

An entry leaves this file by being done, or by being decided against — in
which case the decision goes where it belongs (an ADR, or the spec it
contradicts) and the entry says so before it goes.

## Login

8. **`/connect` has never run against a live subscription.** The sign-in
   is built and tested against a fake (aldwin-login.md, Steps, "Live
   run"), but three things only a real account shows are unconfirmed: the
   token response's `expires_in`, the 403 some SuperGrok tiers get after
   a successful sign-in, and whether a stale token draws the 401 the
   retry is built for. Closing it means one sign-in and a few requests on
   the developer's xAI subscription, and fixing what they show. The
   developer's call, 2026-09-29: another time.

9. **`/connect`'s screens are not pinned.** No snapshot or capture scene
   draws the account list, the sign-in sentence or the "Connected"
   notice (aldwin-login.md, Steps, "Review scenes"), so a change to them
   passes stages 5 and 8 unseen. Closing it means scenes for the three,
   in `render_snapshot.rs` and `scene.rs`. The developer's call,
   2026-09-29: another time.

## TUI

10. **A stop leaves an undecided review on screen.** `⌃C` in a review
    that is not yet decided cancels the turn, and the turn's wait on the
    decision (`DispatchContext::review`) is dropped with it. But
    `Review::closes_at_turn_end` keeps a `Round::Open` review open, so it
    stays with no turn behind it, and an approve there reaches no one.
    Seen while building frame K: a queue held across that stop stays
    queued rather than going into the review's field
    (`a_stop_over_an_open_review_keeps_the_queue_out_of_its_field`), and
    since no turn follows, it stays queued until `esc` takes it back.
    Meanwhile a new message is sent ahead of it, and `/clear` or
    `/resume` leave it on screen (`App::reset_conversation` keeps
    `queued`). Closing the review on the stop closes these too.
    Closing it means the review closing on a cancelled turn, as a waiting
    one does, or an ADR saying why it stays.

## Review

6. **Frame capture runs only on Linux.** Capture drives foot under sway,
   neither of which runs on macOS, so `Pty::open` refuses there
   (`crates/review/src/pty.rs`) and the compositor fails to start first.
   The crate, the gate and stages 1–5 build and run on a Mac, but a change
   that moves a scene's snapshot needs captured frames for stage 8
   (aldwin-review.md, "The stages"), so a contributor on a Mac cannot pass
   the review for it. Closing it means a terminal and compositor that run
   on macOS behind the same capture, or deciding it against. The
   developer's call, 2026-09-27: kept open, for later.
