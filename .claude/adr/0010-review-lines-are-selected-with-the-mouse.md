# ADR 0010 — Review lines are selected with the mouse

**Status:** accepted, 2026-09-24
**Amends:** the "Mouse support — keyboard-only" out-of-scope entry in
`aldwin-tui.md`, and the review's keys as `aldwin-tui.md` and the README
state them. ADR 0009 §4 decides that the review exists and what it gates,
not its keys, and is unchanged.
**Affects:** `aldwin-tui` (`review.rs`, `ui/review.rs`, `run.rs`, `app.rs`)

## Context

The review's diff had a keyboard line cursor: `↑↓` moved it, `⇧↑↓` grew a
selection from it, `Space` opened the fold under it. The only mark it had
was its gutter number, drawn in `label` where every other number is
`label3`.

The design has no such cursor. Frames G, H and I draw every gutter number in
`label3` (`tokens/colors.css`: `--label3` is "line numbers"), and frame H
shows a selection as the accent `▎` edge and brighter code and nothing else.
The whole-app review pass of 2026-09-24 reported the brighter number as a
major finding. Drawing the number in `label3` alone would have left the
cursor with no mark at all.

The developer's answer was that the cursor was the wrong model: "selection
should be done with one's mouse. A user can drag a selection of rows, doing
so will highlight them for a comment. Clicking on a single row would select
just that row."

The mouse has a history here. Capture was switched on once (2026-09-02) so
the wheel could scroll the log. It was reverted the next day, because a
terminal gives the mouse either to the application or to its own text
selection and never to both, and losing click-drag copy out of the
transcript cost more than the wheel was worth (`run.rs`, `aldwin-tui.md`).

## Decision

1. **There is no line cursor in the diff.** A row is marked only when it is
   selected. Every gutter number is `label3`.
2. **A press on a line selects it; a drag carries the selection to the row
   under the pointer**, in either direction. A drag past the pane's top or
   bottom stops at the first or last row shown, and a drag that ends on a
   fold takes every line the fold hides. A press on a fold opens it. A
   press anywhere but a diff row does nothing.
3. **A selection is lines, not rows.** It is kept in the file's unfolded
   rows, so opening a fold — which renumbers every drawn row after it —
   leaves it on the lines it was made on.
4. **The mouse is captured only while a review is open,** and only for
   presses, releases and drags (xterm modes 1000, 1002 and 1006; not 1003,
   which reports every movement of the pointer and would repaint the review
   each time). `run.rs` asks when the review opens and releases when it
   closes; `restore_terminal` releases on every way out, a panic included.
   The conversation keeps the terminal's own selection, so the 2026-09-03
   reversal stands everywhere it was about: the transcript is not on screen
   during a review, so there is nothing of it to copy.
5. **The keyboard reaches every line too** (amended the same day, from the
   UX gate's "usable by keyboard alone" and the HIG's "Keyboards": Shift and
   an arrow extends a selection). `Shift ↑↓` with nothing selected selects
   the first line shown; `↑↓` then move the selection and `Shift ↑↓`
   extend it, and the pane follows. With nothing selected, `↑↓` scroll.
   This is not the cursor back: nothing is marked until you select, and
   what is marked is the selection, drawn as frame H draws one.
6. **The keys:** `↑↓`, `PgUp`/`PgDn` and the wheel scroll the diff, or move
   a selection. `Shift ↑↓` selects. `Space` opens every fold in the file —
   the keyboard's way to the lines a click would unfold. `↩` comments on
   the selection. `⌃↩` approves, or sends the comments. `Tab`/`→` and
   `Shift Tab`/`←` move between files. `⎋` clears the selection and then
   asks before discarding. `?` shows the keys. Shift and Tab are named in
   words, as Space is: the glyph table has no mark for either.

## Consequences

- Everything the review does can be done from the keyboard alone; the
  mouse is the quicker way to a run of lines, not the only one.
- In a terminal that ignores capture — or a multiplexer that does not pass
  it through — a drag is the terminal's own selection. `Shift ↑↓` still
  selects, so nothing is lost but the pointer.
- While a review is open, diff text cannot be copied with a plain drag. Most
  terminals still allow it with a modifier (Shift in xterm, VTE, kitty,
  WezTerm).
