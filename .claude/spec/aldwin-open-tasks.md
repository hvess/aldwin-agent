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
MCP tool's writes outside the review became its own change (entry 2); undo was decided against (ADR 0009);
the note that the macOS sandbox had never run on a Mac was dropped, since
the developer runs it there. What is still true after that is entered here
afresh.
**Owner:** Maximilian
**Last Updated:** 2026-09-27

An entry leaves this file by being done, or by being decided against — in
which case the decision goes where it belongs (an ADR, or the spec it
contradicts) and the entry says so before it goes.

## TUI

1. **A failed turn's kind reaches the TUI as text.** `TurnEndReason::Error`
   carries a `String` (`crates/core/src/event.rs`), and
   `log::failure_sentence` / `provider_sentence` (`crates/tui/src/log.rs`)
   choose the sentence the developer reads by matching its prefixes
   (`network error:`, `provider error 429:`, `terminal error after 0
   attempts:`). A change to the core's wording silently drops the TUI to its
   generic sentence. Closing it means carrying the failure's kind as a typed
   value beside the text, and choosing the sentence from the kind. The
   developer's call, 2026-09-27: fix it, as its own change after the comment
   rework.

3. **The model's thinking is not drawn.** Thinking streams to the TUI
   (`Event::ThinkingDelta`) and is carried and saved (ADR 0006), but
   `App::apply_event` drops it (`crates/tui/src/app.rs`), so a turn that
   thinks for a long time shows nothing until its reply starts. ADR 0006
   left it undrawn because the design system has no treatment for it, and
   one is not invented locally. The developer's call, 2026-09-27: draw it
   once the design specifies how — a disclosure, say, as `Read 1 file  ›`
   is.

## Tools

2. **An MCP tool's writes bypass the review.** An MCP call runs in its own
   process over the real tree. The dispatcher opens the review before it
   (ADR 0009 §4), but what the server writes is never staged: it lands on
   disk unreviewed, inside the workspace the sandbox allows (ADR 0011),
   which bounds the gap without closing it. A first answer — record the
   workspace around each call, put back what changed, stage it — was built
   on 2026-09-27 and taken out of that change before it landed: its
   put-back is Aldwin's own write, outside the sandbox, and each review
   pass found another way a path check in it could be fooled (a swapped
   link, an unreadable file, a name that is not UTF-8). The developer's
   call, 2026-09-27: its own change, with the put-back run inside the
   sandbox so the kernel refuses a write outside the workspace rather than
   a check in Aldwin.

## Workspace

4. **Comments predate the comments skill.** `.claude/skills/comments/SKILL.md`
   holds every comment to what an LLM reader needs; the crates were written
   before it. The rework lands one crate per commit, each through the review
   loop and changing comments only: login (landed with the skill), llm and
   config are done; core, tools, cli, tui and review follow, in that
   order. Entry 1 waits on it. Closing it is the last crate's commit.
