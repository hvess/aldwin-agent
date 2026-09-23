# aldwin-tui

ratatui frontend — renders the core event stream, submits commands, and is
the one place a change reaches disk: the full-window review.

**Status:** active. Rebuilt 2026-09-23 against the Aldwin Design System
(ADR 0009). Everything below the first Progress entry describes the crate as
it was under the Mjolnir system and is kept as history — the label column,
the top bar, the permission panel and first run are gone, and the entries
that measured them are no longer claims about the code.
**Scope:** crates/tui
**Owner:** Maximilian
**Last Updated:** 2026-09-23

**Progress (2026-09-23, the Aldwin Design System — a replacement):** The
design system was replaced whole (`.claude/design/IMPORT.md`, "None of
Mjolnir's values carry over") and with it the product's behaviour (ADR
0009). This crate was rewritten rather than retoned. What is here now, and
what each thing was measured against:

- **The grid lost its label column.** `tokens/layout.css`: a 3-cell margin,
  a 2-cell mark column, prose on cell 5 (`grid::BODY_X`, declared by the
  design and checked against the sum). The echoed prompt (`UserEcho`) is a
  `tint` band inset by the margin with a `›` in the mark column; the agent's
  prose is unlabelled at `BODY_X`. Speaker labels are gone.
- **There is no top bar.** The design's title bar is the terminal's;
  `run.rs` sets it (`gateway — aldwin`) with `SetTitle` and draws nothing.
  The window opens on one blank row (`padding: 24px 0`) and the transcript
  is top-anchored, as frames `B`–`F` are.
- **The launch card** (`ui::launch`) is the brand mark beside four facts.
  The mark is 18×6 `▀` cells with independent fg/bg, generated into
  `tokens::MARK_{DARK,LIGHT}` from the frame's `linear-gradient` cells —
  the README's "half-block cells" was the only prose about it, and it was
  right. Facts are centred four-in-six against the mark, labels in
  `--fact-col`.
- **The bottom band** (`ui::chrome::Bottom`) is measured once and drawn
  once: blank / field / blank / footer / blank for the conversation; the
  question panel in place of the field; the command menu above it. The
  footer's status word sits on the body column with the running `●` in the
  mark column (amber, and the one thing besides the caret that blinks);
  key groups are `--group-gap` apart; the context bar is flush right, ten
  `━` segments, the filled run ramping through `tokens::GAUGE_*` —
  `ContextBar.jsx`'s own arithmetic, `round(pct/10)` segments and a
  `60/n` step, checked against the two bars the frame draws.
- **The field** carries a `›` in accent and a blinking `label` caret; its
  right edge carries an action only in the review (`Approve  ⌃↩` grey until
  every file is read, `Send N Comments  ⌃↩` in accent). The placeholder
  says what the field is for in each state, in the design's words.
- **The plan** is `LogEntry::Plan`, one per turn, replaced in place:
  `✓` accent over `label2`, `●` amber over `label`, `○` `label3` over
  `label3`. **Work** is `LogEntry::Work`, a disclosure whose summary counts
  by verb (`Read 3 files · Ran 1 program`) and whose rows are verb /
  target / right-flush fact; Space on an empty field opens every
  disclosure of the current turn.
- **A question** (`ui::question`) is the one list control (`list.rs`) on
  the `panel` ground: question in weight 600, detail, blank, numbered
  options with the current on `field`. Four things ask through it — the
  agent's `ask`, the provider and model questions when nothing is
  configured, `/resume` — and the command menu is the same rows on the
  window ground above the field.
- **The review** (`review.rs`, `ui::review`) is the whole window: header
  with the last request as title and the agent's last sentence as summary;
  the tree 28 cells wide on `tint` from the frame's left edge with reading
  dots and `✓`/`›` file rows; the diff with a 5-cell gutter, a 2-cell sign
  column, `⋯  N lines` folds keeping one context line each side, `▎`
  selection in the gutter's first cell, and `◆ comment` riding at the end
  of its row. The comment field is two rows on `select` and `field`.
  Approve is gated on every file's bottom having been on screen. `⌃↩`
  exists on the wire only under the Kitty keyboard protocol (see the
  2026-09-20 Shift+Enter entry), so a bare `↩` with nothing selected and
  nothing typed does the same — approve, or send the comments — where the
  terminal cannot tell the two apart; with a draft it is still the comment.
- **A follow-up turn is echoed.** `Event::FollowUp` arrives before the
  `TurnStarted` of a turn the developer did not type — their review
  comments, or a discard, started as the next message (ADR 0009 §4) — and
  the TUI pushes it as a `UserMessage` the way it echoes a typed one, so
  the transcript on screen matches the one the model has.
- **Failures are sentences** (ADR 0009 §5): `LogEntry::Failure` in
  `label` with the detail in `label2` one disclosure below. No `✗`, no `!`,
  no red outside a diff; `render_snapshot.rs` asserts it.
- **Markdown keeps fences and tables and loses highlighting.** Fences are
  `label2` on `tint` under a `label3` caption; tables draw per ADR 0002;
  `syntect` is gone from the manifest. Inline code is `label` on `tint`.
- **Deleted:** `first_run.rs`, `highlight.rs`, `picker.rs`, `ui/decision.rs`,
  `ui/diff.rs`, `ui/first_run.rs`, `ui/picker.rs`, `examples/snapshot.rs`,
  and the dependency on `aldwin-permissions`.

Measured and rendered: every position above is a token lookup in
`layout.css` checked against the frame's markup, and the frame was
rendered with Firefox headless before any of it was drawn. Two things the
render corrected that the prose had not stated: the plan's running dot is
amber, and the footer's status word sits on the body column, not the
margin.

Tests: `ui/tests.rs` was rewritten around the new rules (eleven tests);
`tests/render_snapshot.rs` pins thirteen scenes at 80×24, 104×32 and 200×50
in both themes and asserts colour conformance, the closed glyph table, the
margins, no strokes, and the two hue rules. The snapshot was re-recorded
deliberately.

**Progress (2026-09-21, the lantern-gold repaint):** The design system was
replaced, not adjusted — warm greys under one brand colour where it was a
single 300° hue — and the eleven frames were renumbered into one file. Read
`.claude/design/IMPORT.md`'s entry of this date before touching anything
visual here; it is the record of what arrived and what was deliberately not
followed.

What this crate did, beyond taking new values:

- **`--tui-step-done` and `--tui-glyph-done` collapsed into `--tui-done`.**
  The two existed because on the old violet light ground a settled step and a
  finished tool call had to recede from the mark in *opposite* directions.
  With a neutral `●` and a gold mark there is nothing to recede from. The
  paired test went with them; `a_settled_glyph_is_a_neutral_and_never_the_mark`
  replaces it, and asserts the thing that is now load-bearing — gold means
  open, never finished.
- **The transcript no longer fades behind a panel, it recolours.**
  `semantic.css`: "a recolour, never alpha". `palette::fade` and
  `PANEL_TRANSCRIPT_OPACITY` are deleted and `Palette::scrimmed` maps ink to
  the three `--tui-scrim-*` roles. This *strengthened* stage 4: the colour
  conformance test used to allow every ink composited over every ground — 48
  × 48 blended values — and now allows the palette and nothing else, so a
  dimmed cell is held to exactly the standard an undimmed one is.
  `scrimmed` maps by colour, which is sound only while `scrim_quiet` and
  `scrim_mark` are one value; `the_two_quiet_scrim_roles_coincide` fails with
  an explanation if the design ever parts them.
- **The syntax ramp went five roles to three.** `syn_type` and `syn_number`
  are gone — a type or a number is now simply `code`, which is what the
  design says — and the rule behind it is worth keeping: no status hue may
  appear inside a code block. The old ramp broke that on its face, because
  its string colour *was* the diff green.
- **Four status hues arrived, each with its glyph**, and three states that
  used to be prose became rows: an error is `✗`, a notice `·`, a cancelled
  turn `!`. The last is a distinction the app did not draw before — stopped
  is not failed — and `Outcome` draws the same line for a refused call.
- **Two frames the app had are now different screens.** First run lost its
  wordmark and its 3-row footer for the standard 5-row band (`cells.css`:
  "blank, prompt or keys, blank, status, blank — every frame"); the empty
  state lost its wordmark and its `in` row (the top bar already carries the
  directory) and is **top**-anchored, where it was bottom-anchored. Only the
  conversation hangs off the composer now, which is `2a` against `1d`.

Three things the reference draws that are deliberately **not** drawn, each
because it would state something untrue rather than because it was hard:
`config → …/config.toml` (config is YAML, and two files), `esc stop` and
`esc clear` (Esc is unbound in the composer; `^c` cancels), and `/access`
(no such command). Recorded as
`frame-names-files-a-command-and-keys-the-product-does-not-have`.

**Progress (2026-09-19, ADR 0003 — the permission option row is a sentence):**
The decision panel's option list was `5c`'s name + detail pair on `5a`'s
screen. It is now `5a`'s sentence: one column, the grant pattern quoted inside
it one step quieter, no detail column.

What went with the shape, because all three were mechanisms for an axis the
ADR removed: `App::decision_pattern_scope`, `PatternScope`, `GrantSummary`,
`GrantUnit`, `App::decision_grant`, `decision::grant_lines`, `GRANT_RULE_MAX`,
the panel's grant-summary and `Tab` rows, and `Tab` itself as a panel binding.
`broad_pattern`, `directory_glob` and `program_glob` are untouched — the broad
unit is still ADR 0001's, it is just now the pattern of two specific rows
rather than a mode the whole list sits in.

Three things a reader of this crate should know:

* **`OptionRow` deliberately serves two different controls now.** Its doc
  comment used to assert that the permission list and the model picker "cannot
  drift into two different controls". They are two controls, because the
  design draws two (`5a` and `5c`); what the shared type still holds in common
  is the selection convention — mark, band, number, tones.
* **The quoted pattern is elided by the renderer, not by `app.rs`.** A grant
  over a 200-character shell command is ordinary input and `Row::build` wraps,
  so an unelided sentence would silently become a two-row option. `app.rs` has
  no frame width, so the budget is computed in `option_rows` from `LABEL_COL`
  and `MARGIN_X` — which is why `LABEL_COL` is now a module constant.
* **`max_height` still caps the panel at a quarter of the frame.** Its doc
  comment justified that against `5a`'s stated half on the grounds that the
  grant-summary and `Tab` rows made Aldwin's panel need ~20 rows. Those rows
  are gone and the argument is largely spent, but it was deliberately left
  alone in the same pass: two geometry changes at once make the next
  screenshot delta unreadable about which caused what.

Measured over the five permission scenes at three sizes in both themes — 30
frames, clean. *(The conformance catalogue this closed seven entries in was
deleted on 2026-09-20 with the harness that produced it; see
`.claude/spec/aldwin-review.md`'s Progress entry for why. The measurement
stands, the catalogue numbers no longer resolve, and git history has them.)*

**Progress (2026-09-19, the conformance catalogue's unblocked layout items):**
*(That catalogue was deleted on 2026-09-20 — see `aldwin-review.md`. The
work below was done and stands; its item numbers no longer resolve.)*
Six Class A deviations, plus
one the gates found while they were being captured. Each is recorded in full
there; what belongs here is what moved in this crate and why a reader of the
code should care.

* **The caret is drawn.** `chrome::caret_row` paints `▌` in `--tui-mark` at
  the draft's cursor and nothing calls `set_cursor_position` any longer, so
  the terminal's own cursor stays hidden everywhere — which is what `14d`
  specifies (`▶  ▌`, both `--t-mark`) and what first run already did. One
  cell of the composer's text column is held back for it (`CARET_LEN`):
  without it a row filled to its last character had nowhere to put the glyph
  and ratatui clipped it at the rect's edge, which is `justified_line`'s
  lesson in a third place.
* **A turn knows whose it is.** `transcript::Speaker` replaces "is this entry
  a `UserMessage` or an `AssistantText`" as the rule for drawing a turn
  break. A tool group, an approval card and a resolved prompt all belong to
  the agent, so the first of them opens the agent's turn — break band above
  it, `harness` in the label column — and the reply that follows continues
  that turn rather than opening a second one. The cache key gained `opens`
  alongside `first` for the same reason `first` is in it: both depend on what
  came before, so both can change without the entry changing.
* **A turn break is not drawn against the top of the viewport.**
  `Transcript::viewport` skips a leading separator the viewport starts
  inside. It deliberately does not backfill the freed rows or change
  `Transcript::len` — the note in its doc comment is the important half:
  feeding the drop back into the row count would shorten the transcript,
  un-scroll the turn it hid, and bring the band back next frame.
* **A `@@` line is a hunk header.** `diff::Kind::Hunk`, rendered in
  `--tui-hunk-header` with an empty gutter, anchoring the line numbers from
  its own offsets. This only ever shows in a ```diff fence in assistant
  prose; `aldwin_tools::diff::unified` emits no header, so the approval
  card's own numbering is unchanged.
* **First run's option list is sized by its content.**
  `first_run::option_rows` measures the list once and gives every row the
  same width, so the selection band is a rectangle over the *list* rather
  than a fill to the frame's right margin — 167 cells of accent at 200
  columns before. Note what this is *not* sized by: `5c`'s 48-cell command
  list, which is the nearest stated number and is too narrow for this list's
  own copy by four cells.
* **The permission panel leaves the conversation a quarter of the frame.**
  `decision::max_height` reserved one row for the log and now reserves
  `max(5, height / 4)`. The panel elides sooner as a result, which is the
  trade `5a` asks for.
* **The elision markers lost their decoration.** `⋯` (U+22EF) and `—` were
  in every "N more lines not shown" row, and neither is in the closed glyph
  table or the baseline's typographic exemption. The reference writes this
  row as plain prose (`81 more lines`), so it does too now.

Three more followed from scoring those changes, and two of them were
regressions from the list above — see the conformance spec for the
measurements:

* **The panel protects its head, not just its tail.** `clamp_panel` took a
  two-row head on faith; the permission panel's *target* row sat just past
  it, so the tighter budget cut the one row naming the file. The head is now
  passed in — the whole prompt card — and the grant summary, the `Tab` row
  and the options separator are the clampable middle. At 52×20 this also
  recovered a top bar and a footer the panel had been pushing off the frame.
* **A hunk header takes neither the gutter nor the sign column.** It had been
  rendered through the row builder with both empty, which put it on the code
  column; `5b` draws it at the field's left edge.
* **The elision markers lost their decoration.** `⋯` (U+22EF) and `—` were in
  every "N more lines not shown" row, and neither is in the closed glyph
  table or the baseline's typographic exemption. The reference writes this
  row as plain prose (`81 more lines`), so it does too now. The `breakages`
  gate caught this the first time a frame was short enough to elide.

One rule came out of the pass and is now in two places: **a frame too small
for the design's 120×36 gives up spacing before structure.** A turn break
drops its two blank rows and keeps its band; the panel's options separator
does the same. Both alternatives were measured and both are worse — dropping
the separator whole puts a panel's facts against its option list, and
dropping content instead is what the head-protection fix was for.

`crates/tui` is clippy-clean, 292 tests pass and `render.snap` is reblessed;
the snapshot diff was read line by line before each blessing, which is where
the approval panel's new elision behaviour was checked rather than assumed.

**Progress (2026-08-29):** All 12 Steps implemented and tested — `972d150`,
audit-fixed in `bd09172`. Two Pitfall-level gaps, deliberate and disclosed
rather than silently missing: (1) tool-activity groups don't literally
"collapse after a short delay" — each call renders as one bounded summary
line instead, which bounds log flooding without needing a redraw timer,
but isn't the spec's literal mechanism. (2) Shift+Enter's terminal-
dependence (the Pitfall naming kitty/iTerm2/xterm specifically) couldn't
be verified against real terminal sessions in the sandbox this was built
in — there was no terminal to attach to, only a piped/non-interactive
shell. Ctrl+J is wired as a fallback on reasoning about the failure mode,
not empirical cross-terminal testing. A live run (planned separately)
should confirm or correct that reasoning — pay attention to both of these
during it.

**Progress (2026-08-29, live run):** First real-terminal session surfaced
two bugs, both fixed in `a2a28ba`: (1) `handle_key` routed straight to
`handle_approval_key`/`handle_prompt_key` while a card or prompt was
pending, and neither recognized Ctrl+C — a developer who didn't already
know the exact letter keybinding had no responsive key at all, which read
as the harness hanging/crashing. Ctrl+C now always resolves the pending
gate as a deny/decline. (2) `UserMessage` and `AssistantText` both
rendered `Style::default().fg(BRIGHT)`, so the two speakers were
visually identical in the log despite the Design section's stated
bright/normal split — assistant text is now bold-bright, user text is
terminal-default. Cards and the status bar now also spell out the Ctrl+C
keybinding inline, since it wasn't discoverable before. The two gaps
above (tool-activity collapse mechanism, Shift+Enter cross-terminal
verification) are still open — this session's terminal wasn't used to
re-verify Shift+Enter specifically.

**Progress (2026-08-29, follow-up):** A `/`-prefixed `UserMessage` (a
slash command) now renders dim rather than sharing plain user messages'
normal style, per developer request once `/help`/`/exit` landed in
aldwin-cli — otherwise a command looks identical to a chat message in
the log, undermining the point of having named commands at all. See
`is_command` in `ui.rs`; duplicates cli's own `/`-prefix check since tui
can't depend on cli (wrong direction) to reuse it.

**Progress (2026-08-29, second follow-up):** Two more requests from the
same live session: (1) user/assistant separation via color alone (plus
the earlier bright/normal split) still read as too subtle — assistant
text now gets a `●` marker on its first line, user text moved off
terminal-default onto a dedicated green (`USER` in `ui.rs`), and
`draw_log` inserts a blank line between every log entry, not just at the
user/assistant boundary. (2) fenced code blocks in assistant text were
rendering as raw text, backticks included — `highlight.rs` now parses
` ``` ` fences (`ui::split_code_fences`) and syntax-highlights the body
via `syntect` (bundled syntax/theme dumps, `default-fancy` feature — pure
Rust regex backend, no C toolchain dependency), inside a dim
`┌─ lang` / `│ ` / `└─` border; an unrecognized language tag falls back
to `syntect`'s plain-text syntax rather than refusing to render. Explicit
user choice: full syntax highlighting over a lighter bordered-only
treatment, accepting the added dependency, ~4MB larger release binary,
and `base16-ocean.dark` as the highlighting theme (no way to detect the
terminal's actual background — same open question as the deferred accent
color above; revisit together).

**Progress (2026-08-29, scrolling fix):** The developer reported the TUI
"has no scrolling capability" at all. Root cause: `ui::draw` called
`app.scroll.set_viewport_height(log_inner_height, app.log.len())` —
`log_inner_height` is real rendered rows, `app.log.len()` is *entry*
count, and `ScrollState::max_offset` computed `total_len.saturating_sub
(viewport_height)` from those two mismatched units. A handful of entries
routinely renders to far more rows than the viewport, so `max_offset`
stayed 0 long after there was real content below the fold — auto-follow
never advanced past the top, and manual scroll keys had nothing to move
into, since `draw_log`'s `.skip(app.scroll.offset)` was already being
applied to the correct (flattened-line) vector; only the offset itself
was wrong. Fixed by adding `App::total_lines()` (`app.rs`) — an exact
(not approximate) rendered-row count via a new pure `log::line_count`,
proven line-count-equal to `ui::render_entry`'s output for every
`LogEntry` variant except one narrow, self-correcting streaming edge
case — and using it everywhere `self.log.len()`/`app.log.len()` was
previously passed to a `ScrollState` method. `scroll.rs`'s doc comment
updated accordingly (it previously and incorrectly described the unit as
"whole entries"). Confirmed by reverting the `ui::draw` change alone and
watching the new regression test fail exactly as the user described,
then pass again once restored.

**Progress (2026-08-29, markdown support):** Assistant prose was rendering
raw markdown source (`**bold**`, `` `code` ``, `# heading`, `- item`,
literal asterisks and backticks included) — reported directly by the
developer ("LLM output is in markdown, but aldwin doesn't support it").
`ui::render_markdown_line` now parses each prose line (fenced code was
already handled separately, see the 2026-08-29 second-follow-up entry
below) for bold/italic/inline-code/strikethrough/links, and per-line block
prefixes for headings, bullet/ordered lists, blockquotes, and thematic
breaks. Hand-rolled rather than a CommonMark crate: a real block parser
normalizes blank lines and reflows paragraphs across source lines, which
would break `log::line_count`'s exact one-`Line`-per-source-line invariant
that `ScrollState`'s bookkeeping depends on (see the scrolling-fix entry
below); per-line prefix detection plus a small recursive inline pass
covers what LLMs actually emit without touching line count. All markdown
styling uses modifiers only (bold/italic/underline/reversed/crossed-out) —
no new colors — since the Palette section below reserves the one accent
color for the approval card and focused input. Plain assistant prose
dropped its blanket bold modifier as part of this (now BRIGHT only, bold
earned via `**...**` or a heading) so real emphasis has contrast against
the surrounding text.

**Progress (2026-08-29, muted user color + welcome banner):** Two more
developer requests. (1) The dedicated LightGreen for user input (from the
2026-08-29 "second follow-up" entry above) read as too loud against the
developer's actual terminal color scheme — `USER_FG`/`USER_BG` in `ui.rs`
replace it with a muted gray text color plus a subtle background tint
(fixed RGB, not a named ANSI color, so it isn't reinterpreted by whatever
the terminal theme maps that slot to), applied only to plain chat
messages — a `/`-prefixed slash command keeps its plain dim style with no
background, preserving the harness-directed-vs-conversation distinction.
(2) A welcome banner now renders above the conversation log on every draw
(`ui::intro_lines`, always exactly `log::INTRO_LINE_COUNT` rows): an ASCII
rendering of the little owl from aldwin.md's Mascot section (boxy
outline, `◉` camera-iris eyes as the one expressive feature, perched on a
rail rather than ambulatory, talons gripping rather than acting), the
`ALDWIN` wordmark and tagline, and a version/model line
(`v{CARGO_PKG_VERSION} · {model_name}`). It isn't a `LogEntry` — it isn't a
core event, so it doesn't belong in the append-only event log semantics
that `log.rs`'s doc comments describe — instead `ui::draw_log` prepends it
directly and `App::total_lines` accounts for its fixed row count (plus the
one separator before the first real entry) the same way it already
accounts for the transient thinking indicator. The owl uses ACCENT
(cyan) for its outline/eyes and the wordmark — a deliberate, scoped
expansion of accent beyond "card border and focused input only" (see the
Palette bullet below), not a resolution of the still-open mascot color
palette question in aldwin.md's Mascot section.

**Progress (2026-08-29, git commit in the banner):** The banner's version
line originally showed only `CARGO_PKG_VERSION` — reported back by the
developer as unhelpful, since the whole workspace shares one version
(`0.1.0`) via `version.workspace = true` that doesn't move commit to
commit; on an actively-developed harness that's not enough to tell a
developer which build they're actually running. `crates/tui/build.rs`
now shells out to `git rev-parse --short=8 HEAD` (falling back to
`"unknown"` if git isn't available, e.g. a source tarball with no `.git`)
and `git status --porcelain` for a `-dirty` suffix, exposing the result as
`ALDWIN_GIT_HASH` via `cargo:rustc-env`; `ui::intro_lines` reads it with
`env!(...)` alongside `CARGO_PKG_VERSION`. Also explains the "why is my
build binary not showing the new intro at all" report immediately prior
to this entry — the real cause there was a stale prebuilt binary, not a
code defect, but it's the reason the version line needed to earn its keep
enough to answer "which commit is this binary actually built from" going
forward.

**Progress (2026-08-29, mascot pivot to Mjolnir):** The owl mascot from the
welcome-banner entry above was replaced at the developer's explicit
direction toward something more "aggressive/directive" — several
procedurally-generated candidates were tried in between (a cobra shaded
with `░▒▓█`, a hand-coded "tribal" infinity mark, two hand-coded Mjolnir
attempts using a diagonal crosshatch-weave texture) and rejected, most
pointedly for not being "a true representation" of the reference images
supplied. The version that landed, `ui::MJOLNIR_ART`, is not
hand-drawn or procedurally generated at all: it's a literal trace of a
real reference photo (thresholded to pure black/white, trimmed, resized
preserving aspect ratio, then read back pixel-for-pixel with one source
pixel mapped to one Braille dot — 2×4 real sub-character dots per cell,
not the `░▒▓█` shading-level approximation every earlier attempt used).
This settles aldwin.md's Mascot section in a new direction; that
section's "little owl" rationale is superseded, not merely
supplemented — update it to describe Mjolnir if/when that file gets its
own pass. `INTRO_LINE_COUNT` grew to 28 (21 art rows + blank + wordmark +
tagline + blank + version, plus the 2 border rows) to fit the traced
art's true proportions — noticeably taller than the owl or any of the
procedural Mjolnir attempts, since a faithful trace doesn't compress to
fit a target row count the way hand-authored art can.

**Progress (2026-08-29, mascot trace fixes: quality, size, layout):** Three
issues reported directly against the traced Mjolnir mark above. (1) "The
ascii art looks malformed/incorrect" — the original trace hard-thresholded
the resized image straight off the resize, which fragmented the fine
knotwork linework into disconnected speckle; the pipeline now applies a
Gaussian blur between resize and threshold (`-blur 0x0.6` before
`-threshold 52%`) so thin strokes survive as continuous lines instead of
broken dots. (2) "A little too large" — re-traced at a smaller target
(54×64px source → 27×16 Braille cells, down from 35×21) using a
height-constrained resize (`-resize x64`) rather than width-constrained,
which also happens to read more cleanly at the smaller size since there's
less linework crammed into the same dot budget. (3) "It should be left
aligned with the name of MJOLNIR alongside it on the right" — the banner
was centering one stacked column (art, then wordmark/tagline/version below
it); `ui::intro_lines` now builds the info block as a second column placed
beside the art on the same rows (vertically centered against the art's
height), and `ui::bordered` switched from centering to a small fixed left
margin. This only works because `MJOLNIR_ART`'s rows are fixed-width (not
trimmed of trailing blank Braille cells) — trimmed rows would put the info
column at a different screen column on every row. `INTRO_LINE_COUNT`
dropped to 18 (16 art rows + 2 border — the info block no longer adds
rows of its own, since it now shares the art's rows instead of following
them).

**Progress (2026-08-29, big wordmark, and a hammer-technique detour
reverted):** The plain "M J O L N I R" text line wasn't enough presence
next to the hammer art — the developer asked for the wordmark itself
rendered as block-letter art. Landed on FIGlet's "Whimsy" font (`-k`
kerning), found by rendering "MJOLNIR" through the ~370-font
xero/figlet-fonts collection and grepping for a fragment the developer
pasted as their preferred reference, after two earlier wordmark attempts
(hand-drawn angular block letters via a small stroke-rasterizer, then
FIGlet's "Colossal") — the developer wanted a real existing font, not
another from-scratch design. `ui::WORDMARK_ART` (10 rows) now renders
above the hammer, with a blank separator, then `ui::MJOLNIR_ART` below it
with the tagline/version info still beside the hammer (unchanged from the
entry above). `INTRO_LINE_COUNT` grew to 29 (10 wordmark + 1 blank + 16
hammer + 2 border).

Also worth recording since it nearly shipped: mid-turn, in the same
request that asked for the wordmark, the developer separately reported
the hammer "malformed again" after an earlier scaling fix. Read that as
license to re-derive the hammer from scratch using `░▒▓█` block-shading
instead of Braille (reasoning: Braille glyph coverage is more
font-dependent, and the developer's own best-received *reference* images
used block shading) — box-averaged a fresh trace at 27×46 and wired it
in alongside the wordmark work. The developer caught this immediately
("did you change the hammer itself? I only wanted a text addition") and
clarified the malformation was a scaling issue from an earlier commit,
not a font-rendering problem, and that the Braille hammer itself was
fine. Reverted `MJOLNIR_ART` back to the 16×27 Braille array verbatim
(pulled from the prior commit rather than retyped) before it was ever
pushed. Lesson for next time: a "let's give it a try" on a bundled
proposal (wordmark + hammer swap presented together) approves the
bundle as understood, not license to expand scope further within it —
confirm the swap specifically when a request only asked for an addition
elsewhere. The actual "malformed" cause (a genuine scaling bug from an
earlier commit, per the developer) is still open — not yet diagnosed.

**Progress (2026-08-29, wordmark lands on ANSI Shadow, beside the hammer
again):** Fast-follow correction and font search. (1) The wordmark entry
above put `WORDMARK_ART` *above* the hammer with a blank separator — a
second unrequested deviation from the standing layout rule (art
left-aligned, text alongside it on the right, from the same-day "mascot
trace fixes" Progress entry) that the developer caught immediately
("you broke the rule where the text is supposed to be alongside the
hammer"). `ui::intro_lines` now builds one combined right-hand column —
`WORDMARK_ART` stacked above blank/tagline/blank/version — placed beside
`MJOLNIR_ART` on the same rows and vertically centered against its
height, the same technique the tagline-only info block used before the
wordmark existed. `INTRO_LINE_COUNT` is back to 18 (16 hammer rows + 2
border) since the right column no longer adds rows of its own. (2) Font
search: "Whimsy" was superseded twice more — two further developer
reference pastes turned out not to be standard FIGlet fonts at all (most
likely gradient-shaded text-art-generator output, not matched against
the ~370-font collection or the highest `░▒▓█`-density candidates in it)
— until the developer pasted a code snippet naming a `LOGO_ART` constant
already in FIGlet's "ANSI Shadow" rendering a different product's name,
asking for the same treatment on "MJOLNIR"; that font had already been
fetched earlier in the session while chasing an unrelated lead, so it
needed re-rendering, not rediscovery. Separately flagged and resolved:
ANSI Shadow is the exact font the referenced `LOGO_ART` snippet's
project (Hermes Agent) uses for its own banner — offered Whimsy (already
built, not from Hermes) and two other distinct bold/3D fonts as
alternatives; developer chose to keep ANSI Shadow anyway, since it's a
public FIGlet font, not something proprietary to Hermes. (3) The
hammer's real width (27 cols) plus a 3-space gap plus ANSI Shadow's
"ALDWIN" (59 cols) pushes total banner content past 80 columns for the
first time — this broke over a dozen `ui.rs` tests that had assumed an
80-col `TestBackend`/`rendered()` call, not because their own assertions
were wrong but because the intro banner's own content wrapped inside an
80-col `Paragraph`, corrupting `ScrollState`'s row math (which counts
logical lines, not wrapped screen rows) for the whole log, not just the
banner — every test using the shared `app()` fixture now needs at least
~100 columns. Widened every affected `TestBackend::new`/`rendered(...)`
call; two sites used a different local variable name than the sed
pass's exact-match pattern (`assistant_app`, `user_app`) and needed a
manual follow-up fix.

**Progress (2026-08-29, symmetry, padding, gradient, tagline, stacked
stats):** Five more requests against the settled banner layout above. (1)
"The hammer itself is still not rendering symmetrically" — the trace
looked asymmetric (uneven top end-caps, a rightward lean) even though the
source photo read as symmetric on direct inspection; root cause was in
the resize/blur pipeline, not the source, so this pipeline now forces
bilateral symmetry at the source image itself (`-flop` plus
`-evaluate-sequence mean` averaging the image with its own horizontal
mirror) before the same resize/blur/threshold/Braille steps — guarantees
symmetry regardless of downstream filter quirks, rather than trying to
debug the asymmetry's exact origin. (2) "The hammer needs padding all the
way around it" — `ui::intro_lines` now emits one blank `Line` before and
after the art block, and `ui::bordered`'s `LEFT_MARGIN` grew from 2 to 3.
(3) "Different colors/gradient ... to make it more fancy" — new
`ui::aldwin_row_color(row, total)`, a top-to-bottom RGB lerp (near-white
cyan at the top fading to deep blue at the base) applied per-row to the
hammer art's style; every other element (wordmark, tagline, stats) stays
on the existing flat palette. (4) The tagline changed from "a tool for
thought" to "every strike is yours to call. nothing moves without you." —
three rounds of developer feedback (first: "about the fact that this
harness is a tool that gives you the authority/decisiveness to have
control over the LLM, generate something for me to review"; second: "it
needs to be longer and better"; third: the developer supplied the final
text directly) before landing on wording that names the developer's
authority over the LLM explicitly rather than the harness's own
qualities. (5) "The stats/statuses should not be inline but stacked
vertically" — the combined `model · version · commit` line split into
three separate labeled rows (`model`, `version`, `commit`), changing
`info`'s type from `Vec<(String, Style)>` to `Vec<Vec<Span<'static>>>` so
each row can carry its own label/value span pair.
`log::INTRO_LINE_COUNT` grew from 18 to 20 (the two new padding rows);
the hammer art itself (`MJOLNIR_ART`) kept its existing 16×27 shape — only
the source symmetry changed, not the row/column dimensions.

**Progress (2026-08-29, wrapped-row scroll math):** The developer reported
text getting obscured in real sessions — root cause was the exact gap the
"wordmark lands on ANSI Shadow" Progress entry above disclosed in passing
but didn't fix: `App::total_lines`/`log::line_count` counted one screen row
per *logical* source line, while `draw_log` rendered through
`Paragraph::wrap(Wrap { trim: false })`, which can spread any single
logical line across multiple screen rows once it's wider than the render
width (a long tool-result summary, a long retry message, a long assistant
line). `ScrollState`'s offset math — built on the logical count — drifted
out of sync with what was actually on screen the moment anything wrapped,
clipping content at the bottom of the log area, usually right above the
status bar. That earlier entry's fix (widening test `TestBackend`s to
~100 columns) only masked the symptom in tests; the underlying mismatch
was still live for any real terminal width or content length that
actually wraps.

Fixed by dropping the hand-kept logical count entirely in favor of
ratatui's own wrap-aware `Paragraph::line_count(width)` (behind the new
`unstable-rendered-line-info` cargo feature — same `WordWrapper` ratatui's
render path uses internally, so the count is exact by construction, not
re-derived by hand). `ui::build_log_lines` now builds the log's `Vec<Line>`
once, shared by `draw_log` (renders it) and the new `ui::log_row_count`
(counts its wrapped rows — `App::total_lines` delegates to this). `draw_log`
also switched from `.skip(offset)` on the unwrapped line list to
`Paragraph::scroll((offset, 0))`, since `offset` is now in wrapped-row
units and skipping pre-wrap `Line`s would drift out of sync with that unit
the same way the old count did. `App` gained a `render_width: u16` field
(set by `ui::draw` each frame) so scroll navigation between draws
(`handle_key`, `push`) has a width to count against. `log::line_count` (now
unused) and its tests were deleted; `log::INTRO_LINE_COUNT` survives as a
`#[cfg(test)]`-only constant — it's still an accurate fixed banner-row
count, just no longer part of live scroll math, only test row-offset
math. Regression test:
`ui::tests::auto_follow_accounts_for_wrapped_rows_not_just_logical_lines`
— verified it fails against the pre-fix code (a long wrapping line's tail
gets clipped) before confirming it passes against the fix.

**Progress (2026-08-29, live-feedback batch: cursor, multiline nav, spinner, diff coloring, /clear):** A batch of standing complaints from actual use, fixed together:

1. **Shift+G/j/k silently swallowed the first keystroke of a draft.** The vim-style scroll bindings fired whenever `input.is_empty()`, regardless of modifiers — so typing a message starting with `G`, `j`, or `k` scrolled the log instead of inserting the letter (this is what the developer actually hit typing a capital G). Removed outright rather than patched: PageUp/PageDown/Home/End already cover keyboard scrolling without the ambiguity, and Up/Down took over the vacated job (next point).
2. **No cursor visible in the input box.** `draw_input` never called `Frame::set_cursor_position` — ratatui doesn't place one on its own. Now derives `(line, col)` from `App::cursor` via the new `app::cursor_line_col` helper and positions the real terminal cursor there, clamped to the input box's inner area; skipped while a card/prompt is pending (input is blocked then anyway).
3. **Input didn't support multi-line navigation.** Up/Down now move the cursor between the draft's lines first (`App::move_cursor_vertical`, preserving column where possible, by source line not wrapped screen row), falling through to log-scroll only when there's no such line to move to (single-line draft, or already at its first/last line) — same fallback shape the old empty-input-only vim bindings tried for, but keyed off cursor position instead of buffer emptiness so it can't eat a keystroke.
4. **No loading feedback between turns.** `App` gained `turn_active` (true from `TurnStarted` to `TurnEnded`) and a free-running `tick: u64` counter, advanced every 120ms by a `tokio::time::interval` in `run.rs`'s `select!`. The log's trailing indicator is now an animated Braille-dot spinner (`ui::SPINNER_FRAMES` — the same glyph family `MJOLNIR_ART` traces the hammer in): "thinking…" while an extended-thinking block is open, else "working…" for the rest of an active turn, so the stretch between tool calls and before the first token streams back is no longer silent.
5. **Approval-card diffs were hard to read.** Every diff line rendered in the same plain `BRIGHT` — no color, no signal. `ui::render_approval_card`/`render_diff_line` now give added/removed lines a full-width background tint (same "pad to render width" technique as the user-message chat-bubble background), and collapse unmodified context beyond `DIFF_CONTEXT_RADIUS` (2) lines from the nearest change into a single "N unchanged lines" marker instead of listing every line.
6. **No way to clear context mid-session.** New `Command::ClearHistory` / `Event::HistoryCleared` round trip (aldwin-core) and a `/clear` slash command (aldwin-cli, forwarded rather than handled locally like `/help`, since core has to act on it) — wipes `ConversationLog` and, via `HistoryCleared`, the TUI's own rendered `log` and turn state in step, so the welcome banner reappears the same way it does for a genuinely fresh session.
7. Two smaller polish items: inline `` `code` `` in assistant prose used `Modifier::REVERSED` (bright-white block), which read as jarring against real terminal themes — swapped for a plain distinguishing color (`CODE_FG`), a scoped exception to the "modifiers only, never a new color" rule below (that rule predates this ask). And the ordinary end-of-turn line read as flat/mechanical ("— turn ended —") — reworded to "— answered —"; the cancelled/error variants keep their own wording since those already name a different outcome.
8. **Slash commands only read as dim after Enter, not while being typed.** `is_command`'s dim styling (item covered in the 2026-08-29 second-follow-up entry above) only ever touched the already-submitted `LogEntry::UserMessage`; `draw_input` rendered the draft as a single unstyled `Paragraph::new(&str)`, so a command looked identical to a plain message until it was already sent. `draw_input` now builds its `Paragraph` from a `Text` of per-line `Line`s instead of the raw `&str`, so per-word styling can ride along. First cut only checked whether the input's very first character was `/`, mirroring `is_command`'s whole-message rule — developer follow-up caught that a command word typed anywhere past position 0 (e.g. `hi /exit there`) went unstyled even though it's the identical word. Reworked per explicit developer direction into `ui::highlight_command_tokens` (`KNOWN_COMMAND_WORDS`, duplicated from `cli::slash::intercept`'s match arms for the same reason `is_command` is duplicated): scans every whitespace-delimited word on every line and dims an exact match wherever it falls, deliberately *not* mirroring `is_command`'s "must be the whole message's leading token" rule — this is a cosmetic hint that a recognized command word was typed, independent of whether it would actually be intercepted (only a real leading `/`, per `is_command`, ever is).

**Progress (2026-08-30, spacing + panic-safety/wide-char audit-fix):** Two
unrelated changes. First, a developer-reported spacing complaint: the log's
last rendered line butted directly against the status bar with no
breathing room — `ui::draw`'s vertical `Layout` gained a 1-row blank
spacer between them. Second, a rust-skills audit (coding-guidelines,
m15-anti-pattern, domain-cli, m01-ownership) plus a follow-up 3-pass
verification: (1) `run.rs` never restored the terminal (raw mode,
alternate screen, cursor) on a panic unwinding through the event loop,
only on a normal return — verified as a real gap (no panic hook, no
`Drop` anywhere in the chain existed) though no currently-reachable panic
site was found, so this is cheap insurance rather than a live bug; fixed
with a `TerminalGuard` whose `Drop` is the panic-path fallback and whose
explicit `restore()` is the normal-return path, and which (unlike the
`?`-chain it replaces) always attempts all three restore steps even if an
earlier one fails. (2) `render_entry`'s `UserMessage` padding and
`render_diff_line`'s diff-line padding sized their full-width background
tint by `chars().count()`, undercounting double-width glyphs (CJK, most
emoji) in ordinary chat/diff content — not just the banner, which already
disclosed this same assumption scoped to its own narrow-glyph-only art.
Confirmed with a concrete repro (a 2-character CJK message breaking the
single-row chat-bubble assumption at a narrow render width). Fixed by
switching both to `unicode_width::UnicodeWidthStr::width`, a new direct
dependency (already present transitively via ratatui).

**Progress (2026-08-31, banner rescale + permission model):** Two developer
requests on the welcome banner: (1) `MJOLNIR_ART` read as "quite large" —
downscaled from 21×35 to 13×21 per explicit "must be proportionate"
direction, by decoding the existing trace's Braille dots back into an
84×70 bitmap, box-filtering it down by a uniform 0.6 in both dimensions
(not a naive per-cell crop, which would have distorted rather than
shrunk the shape), and re-encoding the result — same aspect ratio,
same trace, smaller. `MJOLNIR_ART_WIDTH` and `log::INTRO_LINE_COUNT`
(17, down from 25) both derive from/track the new shape per the existing
convention. (2) The banner gave no indication of the current permission
model until a tool actually triggered a prompt — `ui::intro_lines` now
takes `&StatusInfo` instead of a bare model-name string and renders a
fourth stat line, `access`, showing the same merged read/shell/edit
allow/deny view the status bar already computes (`App::refresh_permissions`
/ `perm_state`) — the effective grant for *this* directory (project scope
merged under global, session on top), not a config dump. Colored with the
existing diff-tint colors (`DIFF_ADD_FG`/`DIFF_DEL_FG`) rather than new
ones, matching their established "state at a glance" job.

**Progress (2026-08-31, visual redesign — polished layout, hero-only-when-empty
banner, panels, sidebar):** A full visual pass, prompted by the developer
describing the TUI as "quite rough and barebones," not "immersive," and not
using the terminal's full area — confirmed concretely by screenshotting the
running binary (a small ANSI-capture → HTML → headless-chromium pipeline
built for this pass) before touching any code: the welcome banner rendered
on *every* draw, not just an empty session, still eating ~40% of a typical
terminal's height mid-conversation, with a large dead void below whatever
content fit, and no panel/border around the log at all — text just floated
on the terminal background. This is a visual-only change: every interaction
(input-blocking while a card/prompt is pending, scroll auto-follow/
disengage/re-engage, Ctrl+C cancel-vs-quit, approval y/n/Ctrl+C-denies,
multiline input navigation, markdown/diff rendering) is byte-for-byte
unchanged — confirmed by diffing `handle_approval_key`/`handle_prompt_key`/
`resolve_prompt` against git, which show no changes at all.

Four bands now, not three: a persistent 1-row header (identity/status,
replacing the always-on banner's job once real content exists), the body
(conversation log, optionally beside a sidebar), a 1-row footer
(context-sensitive keybinding legend), and the input box. The welcome
banner (`ui::intro_lines`, now split into `intro_content` + `hero_lines`)
only renders when `app.log.is_empty()` — mutually exclusive with real
entries, not stacked above them — and is vertically centered within
whatever pane height it has, using a new `App::render_height` field
threaded the same way `render_width` already was. The log panel is now a
real bordered ratatui `Block` (`BorderType::Rounded`), with a `Scrollbar`
shown when content overflows the viewport. This is the one place the
redesign risked reintroducing the exact class of bug this file's history
already documents twice (scroll math desyncing from what's actually
rendered): `draw`'s `Block::inner()` call is now the single place an
"inner width/height" is ever computed, feeding both `App::render_width`/
`render_height` and the same-frame render call — never two independently-
derived values — guarded by a new regression test
(`log_row_count_uses_the_bordered_panels_inner_width_not_the_outer_width`,
verified red against a deliberately reintroduced bug before confirming
green against the fix, same discipline as the incidents it guards against).

A secondary, optional sidebar (`draw_sidebar`, fixed 24 cols) shows
permission detail, active tools (now with a name, not just an opaque
call_id — `StatusInfo.running_tools` changed from `Vec<String>` to
`Vec<RunningTool>`), turn/step, and a message count — state the single flat
status line had no room for. Toggled by Ctrl+T (not Ctrl+B — that's tmux's
own prefix key). Auto-collapses below 104 total body columns regardless of
the developer's toggle preference — `App` only ever stores the preference;
`ui::draw` is the only place that combines it with the width check, so a
narrow terminal always wins and the conversation log never drops below 80
columns when the sidebar shows at all. This narrows, rather than reopens,
the "split-pane layout with persistent sidebar — deferred" Out of Scope
item below: it's secondary/ambient and width-gated, not a primary layout
element competing with the conversation.

Card/footer key labels (`approval_key_hint`/`prompt_key_hint`) are now
single functions called by both the inline card and the footer, so the two
can't drift apart (guarded by
`footer_and_approval_card_show_identical_key_labels`). Tool-activity/retry/
error entries gained leading glyphs (▸/✓/✗/⟳/ℹ) instead of bracketed text
tags; retry gained the one genuinely new color (`WARNING_FG`, amber) —
extracted, along with every existing color constant, into a new
`palette.rs` module now that `ui.rs` covers header/footer/sidebar rendering
too. Hand-drawn card corners switched from `┌┐└┘` to `╭╮╰╯` to match the new
ratatui panels. Input box switched to a rounded border and gained dim
placeholder text when empty.

Verified via a design-iteration harness built for this pass and kept in the
repo (`crates/tui/examples/preview.rs` — seeds an `App` with one of six
named scenes and draws one frame to a real alternate-screen terminal,
`#[doc(hidden)]` re-exports in `lib.rs` expose just enough of `ui::draw`/
`app::{PendingApproval,PendingPrompt,RunningTool}` for it to do so without
becoming part of the supported public API) plus a driver script
(tmux capture-pane → ANSI-to-HTML → headless chromium screenshot) — not
committed, but documented here since it's the reason this pass could be
visually validated step by step rather than shipped on faith. 89
`aldwin-tui` tests pass (up from 82; new coverage: the inner-width
regression above, sidebar width-gate-overrides-preference, Ctrl+T toggle,
footer/card key-label parity, sidebar shows tool name not call_id), full
workspace `cargo test`/`cargo clippy -- -D warnings` both clean. Many
existing `ui.rs` tests keyed to exact buffer coordinates relative to the
old always-on banner needed rework once the banner and real log entries
became mutually exclusive — most were converted from hand-derived row
arithmetic to a `find_row` substring search instead, which is more robust
to future layout changes than the coordinate math it replaced.

**Progress (2026-08-31, posting-inspired polish: tinted borders, status
badge, chip styling):** A follow-up UX pass, at the developer's explicit
request, studying darrenburns/posting (a Textual-based terminal HTTP
client the developer named directly, citing its design as something to
learn from) and applying its *structural* design discipline — not its
literal pink/magenta palette, which would clash with Aldwin's own
established cyan/hammer identity. Read posting's `themes.py` (its `Theme`
model: primary/secondary/background/surface/panel/warning/error/success/
accent, plus per-HTTP-method colors and a `border-title-status` pattern)
and `posting.scss` (`.section { border: round $accent 40%;
&:focus-within { border: round $accent 100%; } }`, `border-title-align:
right`) directly from source, plus the project's own README screenshot, to
ground this in what's actually there rather than a general impression.

Four concrete, scoped changes, each screenshotted before/after via the same
tmux-capture → ANSI-to-HTML → headless-chromium pipeline the prior redesign
built: (1) `PANEL_BORDER` changed from a `DIM`-gray alias to a muted,
desaturated tint of `ACCENT`'s own hue (`Rgb(45, 82, 87)`) — every panel
now carries a whisper of the one accent color instead of unrelated neutral
gray, mirroring posting's `$accent 40%`-vs-`100%` discipline (a hand-picked
fixed RGB, since ratatui has no runtime alpha-blend-over-background
primitive) — a refinement of, not an exception to, the standing "one
accent, not scattered" rule: still one hue family everywhere, `ACCENT`
itself still reserved for what should visually outrank ordinary chrome.
(2) `access_spans` (read/shell/edit allow/deny, used by the header, hero,
and now unified into `draw_header` too — previously a third, slightly
different hand-rolled rendering of the same three states) now renders the
state word as a small padded chip (colored background, not just colored
text) — modeled on posting's `border-title-status`/method-color badges;
right-padded only, not both sides, so the flattened `"read:deny"`-style
substring several existing tests already keyed on survives unchanged. (3)
The log panel's own border now carries a right-aligned status badge — "•
live" (accent) or "⏸ scrolled" (amber) — mirroring posting's Response panel
showing its HTTP status directly in the border title. This surfaces
`ScrollState::following`, previously invisible on screen entirely (only
shown when the log is non-empty; the hero has nothing to follow/scroll).
(4) The footer's default key hints became small chips (key on a
`PANEL_BORDER`-tinted badge, description dim) instead of one flat string,
via new `DEFAULT_KEY_HINTS`/`key_hint_line` — and picked up `^T sidebar`,
which the Ctrl+T toggle had never actually been advertised anywhere despite
existing since the prior redesign. Sidebar running-tools also gained
per-tool-name color coding (`tool_color`, a stable hash into a new 6-color
`TOOL_PALETTE`), echoing posting's per-HTTP-method colors.

Caught and fixed during self-review before landing: the "•" live-badge
glyph was first tried as "●" — the same marker `render_assistant_text` uses
for the assistant speaker — which broke
`assistant_text_gets_a_marker_that_user_text_does_not` (a whole-buffer scan
for "is there an assistant marker anywhere" is no longer a safe test once
an unrelated widget can also render that exact glyph); switched to "•". The
chip-badge footer hints, once `^T sidebar` was added, no longer fit an
80-column terminal (the row isn't wrapped) — tightened `key_hint_line`'s
padding from two-sided to leading-only, confirmed against an 80-col
screenshot before and after. `TOOL_PALETTE`'s third entry was originally an
exact RGB duplicate of `CODE_FG` (both `Rgb(224, 175, 104)`, picked
independently) — caught by grepping every `Rgb(...)` literal in
`palette.rs` for duplicates as part of the audit, not by visual inspection.

Explicitly not adopted from posting, and why: jump-mode (single-key focus
jumping between named widgets) and the command palette are real, well-
executed posting features, but they exist to navigate *many* simultaneous
focusable panes (collection tree, seven request tabs, five response tabs) —
Aldwin has exactly one focusable widget (the input box) outside of a
modal card, so there is nothing for either feature to navigate between yet;
building either now would be speculative complexity with no current use,
not a UX gap this session actually has. Per-pane tabs (Headers/Body/Query/
...) don't apply either — Aldwin's "content" is one linear conversation
log, not several independent structured sections. These are noted here as
considered-and-deferred, not silently dropped, in case the interaction
model ever grows enough panes to make them worth revisiting.

92 `aldwin-tui` tests pass (up from 89; new coverage: `tool_color`
determinism, the live/scrolled badge in both states, its absence during the
hero), full workspace `cargo test`/`cargo clippy -p aldwin-tui -- -D
warnings` both clean.

**Progress (2026-08-31, chat padding, code-block redesign, diff line
numbers):** A live-review batch against the status-line-polish pass
(`ab8c237`), all per explicit developer feedback: (1) chat messages had
inconsistent/no padding — user bubbles filled to the render width with no
inset and no top/bottom breathing room, assistant text had none of either.
New shared primitive `filled_line`/`BOX_PAD_H` (1 column) gives every filled
box in the log — chat bubbles, code blocks, diff/approval-card rows — the
same left/right inset by construction; `card_line` now delegates to it, so
every existing card gained the same inset for free. User messages gained a
`card_padding_line` blank row above/below (matching the approval card's own
top/bottom padding); assistant prose gained a matching blank row above/below
plus a 1-column left indent (`indent_prose_line`) for the same visual
alignment, staying unfilled (no bg) per the standing "assistant has no
background" design. (2) An active turn showed "working…"/"thinking…" twice —
once appended to the log itself (`build_log_lines`), once in the status line
above the input, which already showed the identical thing. The log's copy is
removed outright (`spinner_line` deleted with it); the status line is now
the only place live turn activity shows — see
`active_turn_activity_shows_once_not_duplicated_between_log_and_status_line`.
(3) The status line had padding below it (a spacer row plus the input box's
own internal top `Padding`) but none above, read as an oversized gap under
the line and none above it. `draw`'s vertical `Layout` moved the one spacer
row from between the status line and the input box to between the log and
the status line — same total row budget, so the input box's position and
height are unaffected (confirmed no `the_terminal_cursor_...`/`command_token_
is_dimmed_...` test needed touching), the status line now has one row of
padding on both sides instead of two below and none above. (4) The
assistant-speaker "●" marker is removed outright (per explicit "that needs
to go") — user/assistant are still visually distinct by color alone (bright
vs. muted+tinted; see `user_and_assistant_messages_are_visually_distinct`),
same as every intermediate design already relied on for everything except
this one glyph. (5) Fenced code blocks dropped the hand-drawn `╭─ lang` /
`│ ` / `╰─` ASCII border entirely in favor of a real filled box: a new
`palette::CODE_BG` (its own darker tier, distinct from `BG_ELEMENT` so it
still reads as its own surface nested inside the assistant bubble), a
language-label header row, and a `card_padding_line` spacer under the label
plus one at the bottom — "a real code block in a document," per the
developer's own phrasing, not ASCII art. (6) Diff lines (both the Edit
approval card and a ```diff fence in assistant prose) gained an old-file/
new-file line-number gutter (`number_diff_lines`, `diff_gutter`) — numbered
relative to the shown diff since `aldwin_tools::diff::unified` emits no
`@@ -a,b +c,d @@` hunk header to anchor an absolute file offset on (it diffs
a single already-replaced hunk, not a whole file; see that function's own
doc comment). A context line shows the same number on both sides, a removed
line only its old number, an added line only its new one — the same
two-column convention GitHub's own diff view uses. Diff backgrounds
(`DIFF_ADD_BG`/`DIFF_DEL_BG`) were already full-width as of `ab8c237`; the
developer's report that they showed "no background" was against a build
older than that commit, not a real gap — confirmed by inspecting the
already-current source before touching it, rather than re-doing work that
was already done.

Verified two ways: 93 `aldwin-tui` tests pass (up from 91; two coordinate-
pinned tests — `user_and_assistant_messages_are_visually_distinct`,
`a_slash_command_renders_differently_from_a_plain_user_message`, and three
more — were converted from hand-derived row offsets to `find_row` since the
padding changes shifted them, the same migration this file's history already
describes doing once before for the same reason), `cargo clippy -p
aldwin-tui --all-targets -- -D warnings` clean; and a throwaway scratch unit
test (written, run once with `--nocapture` to dump the rendered buffer as a
text+background-color-tag grid, then deleted — not committed, same spirit as
the design-iteration harness in `examples/preview.rs`) confirmed the actual
column alignment of the padding inset, the code block's box boundaries, and
the diff gutter's exact spacing before considering this done.

**Progress (2026-08-31, assistant padding correction + triple-diff fix):**
Two more developer-reported live-use complaints, against the chat-padding
pass directly above. (1) Assistant messages read with noticeably more top/
bottom padding than the input box or a user chat bubble. Root cause: that
same pass gave `render_assistant_text` its own leading/trailing blank
`Line::default()` row *in addition to* the blank separator row
`build_log_lines` already inserts between every pair of rendered entries — a
filled bubble's own `card_padding_line` padding is visually distinct from
that separator (colored fill vs. plain gap), so the two don't read as
doubled the way two indistinguishable blank rows do. `render_assistant_text`
no longer pushes its own leading/trailing blank rows; the separator alone
now gives assistant messages the same single-row gap every other unfilled
entry (tool activity, notices, retries) already got. (2) The developer
described a single proposed edit showing its diff three times: once as the
LLM's own prose before the tool call, once in the real approval gate, once
again after the edit landed. The approval gate itself (`ToolApprovalRequested`
→ `ApprovalCard`) only ever fires once per edit call — confirmed by tracing
every emission site (`core::dispatcher`) and the `edit_class: true` bypass of
the generic four-tier check (`tools::dispatcher`, `permissions::Engine::
check_tool`) — so this was never a duplicate-render bug in the log. The
actual cause lives in `aldwin-core`'s base system prompt
(`crates/core/src/prompt.rs`): it told the model to "always propose a diff
and wait for approval," which is instructions to do by hand, in chat text,
exactly what the `edit` tool's own structural gate already does
automatically and unconditionally (see CLAUDE.md's "Edit is never
allowlistable" constraint) — so a compliant model narrates the diff itself,
then the tool call triggers the real card, then it narrates a change summary
afterward. Reworded to say the tool call itself is the proposal and the
model should not also narrate the diff before or after calling it — one
call, one approval, one diff shown. No test pinned the old wording (the
comment above `BASE` already calls the prose "its own deliverable," open to
revision without touching the structural-ordering guarantees the rest of
that file's tests do cover).

Verified: `aldwin-tui`'s existing 93 tests still pass unmodified (none
asserted an exact blank-row count around assistant text, only that *a*
blank row exists between entries — `a_blank_line_separates_consecutive_
log_entries`); `aldwin-core`'s 11 tests pass unmodified; `cargo clippy -p
aldwin-tui -p aldwin-core --all-targets` clean on both touched files.

**Progress (2026-08-31, duplicated-input turn + wrapped-prose padding):**
Two more developer-reported live-use bugs, one in each of a
still-active spec (`aldwin-tui`) and an already-archived one
(`aldwin-core`) — noted here since this file is where a developer would
look first for a TUI-surfaced complaint, even though the root cause landed
outside this crate. (1) The developer reported that submitted input
sometimes reached the model duplicated — visible by asking the model to
echo back what was sent. Not a TUI input-handling bug (`handle_key` already
filters to `KeyEventKind::Press`, and crossterm reports paste as ordinary
key events with bracketed paste unhandled/off, so pasted text was never
actually duplicated at the input layer) — the real bug was in `aldwin-
core::Agent::run_turn` (`crates/core/src/agent.rs`): the `Command::Submit`
handler appended the new turn's `UserMessage` to `self.log` *before*
calling `run_turn`, which then built its first step's request as
`messages_from_log()` (already including that just-appended record) *plus*
an explicit `messages.push(Message::user(user_text))` of the same text —
so every turn sent the developer's message to the LLM twice, though the
conversation log itself only ever recorded it once and stayed clean (which
is why no existing test caught it — none asserted on the actual request
`ScriptedClient` received, only on the log). Fixed by dropping the
redundant push and the now-unused `user_text` parameter `run_turn` took
solely to make it; `messages_from_log()` alone is authoritative. New
regression test `submitted_text_reaches_the_llm_exactly_once` asserts the
submitted text appears exactly once in the first request `ScriptedClient`
observes (confirmed to fail against the pre-fix code, reproducing the
report, before confirming it passes against the fix). (2) The developer
also reported that a long assistant reply's first screen row had the
correct left padding but every wrapped continuation row after it did not.
Root cause: `render_assistant_text`'s `Prose` arm built one full logical
`Line` per source line of markdown (via `render_markdown_line`) and
inserted the `BOX_PAD_H` left-inset span once, at that `Line`'s start
(`indent_prose_line`) — but the actual row-splitting for anything longer
than the panel width happens later, inside `draw_log`'s `Paragraph::
wrap(Wrap { trim: false })`, and ratatui's word-wrapper has no concept of
repeating a caller's padding span on the continuation rows it produces; it
just carries on the same styled-grapheme stream from wherever the previous
row left off. New helper `wrap_prose_line` (`crates/tui/src/ui.rs`) does
the word-wrap itself — greedy fill at whitespace boundaries, hard-breaking
a single word wider than the row, preserving per-span styling across a
break, keeping a line's own genuine leading whitespace on its first row but
dropping whitespace a wrap decision introduces on later rows — so every row
`render_assistant_text` now emits is already ≤ the available width *before*
`indent_prose_line` runs on it individually; `Wrap` never has to further
split anything this crate builds for the log (true already of every
`filled_line`/`card_line` row elsewhere — this brings prose to the same
invariant, closing the one place it didn't hold). New regression test
`wrapped_assistant_prose_keeps_the_left_inset_on_every_row` renders a
single unbroken 300-character run in a narrow viewport and asserts every
wrapped row shares the same left inset (confirmed to fail — `[1, 0, 0, 0,
0, 0, 0, 0]` — against the pre-fix code before confirming it passes).

Verified: `aldwin-tui`'s 94 tests pass (93 + the one new one) and
`aldwin-core`'s 12 tests pass (11 + the one new one), `cargo build
--workspace` and `cargo test --workspace` clean, `cargo clippy -p
aldwin-tui --all-targets` clean on the touched file (`ui.rs`); `agent.rs`'s
new test reuses the same `loop { match ev_rx.recv()... { Event::X => break,
_ => {} } }` idiom every other test in that file already uses, including
clippy's pre-existing `single_match` note on that idiom, which this file
already carries elsewhere and doesn't gate on.

**Progress (2026-09-01, queued approvals/prompts):** Developer report: "when
the LLM requests multiple diffs or permissions at once, it breaks the
approval process and the user can only approve one thing." Root cause: both
gates (`App::pending_approval`/`pending_prompt`, `crates/tui/src/app.rs`)
were a single `Option<T>`, but the underlying round trip was never
single-outstanding — `aldwin-core`'s `Agent::dispatch_tools` drives every
tool call in a step concurrently via `future::join_all`, and each
Edit/permission-gated call independently calls `DispatchContext::
request_approval`/`request_prompt`, keyed by its own `call_id` in a shared
`PendingMap` (`crates/core/src/dispatcher.rs`) built for exactly this —
core's side of the round trip was already correct per-call_id; only the
TUI's single-slot tracking of "which one is currently interactive" wasn't.
Parallel tool use (Anthropic's
default) routinely produces more than one Edit or permission-gated call in
one step, so a second `ToolApprovalRequested`/`PromptRequested` arriving
while the first was still unresolved was a real, reachable case, not a
hypothetical one — and the `Option` silently overwrote it. The first call's
approval channel then hung forever with no key able to reach it: its
`oneshot::Receiver` never received a decision, so its dispatch future never
returned, `join_all` never completed, and the whole step (turn) stalled
until Ctrl+C cancelled it — the developer only ever saw whichever card
happened to win the overwrite, matching "can only approve one thing."

Fixed by making both gates queues (`VecDeque<PendingApproval>`/
`VecDeque<PendingPrompt>`, renamed `pending_approvals`/`pending_prompts`
for the plural): `ToolApprovalRequested`/`PromptRequested` now push onto
the back instead of overwriting; `handle_approval_key`/`handle_prompt_key`
only ever act on the front (`.front()`/`.pop_front()`) — resolving it pops
it and the next queued one becomes interactive automatically on the very
next keystroke, no separate "advance" step needed. Input stays blocked
(`draw_input`'s placeholder/cursor gating) exactly as before, just against
"either queue non-empty" instead of "either `Option` is `Some`." Also added
a `queue_hint` helper in `ui.rs`: the status line now appends "(+N more
pending)" to the front card's own key hint when the queue holds more than
one, so resolving the visible card doesn't silently surprise the developer
with another one demanding input right after — kept separate from
`approval_key_hint`/`prompt_key_hint` themselves since those are also what
each card's own key row in the log renders with, where a queue-depth
suffix would be wrong (a card only ever represents itself).

New regression tests: `two_pending_approvals_are_queued_not_overwritten_
and_resolve_in_order` and `two_pending_prompts_are_queued_not_overwritten_
and_resolve_in_order` (`app.rs`) each dispatch two requests with different
`call_id`s and assert both stay independently resolvable in FIFO order,
with each card's own log resolution recorded correctly; confirmed to fail
against a simulated pre-fix overwrite (`pending_approvals.clear()` before
each push) before confirming they pass against the real fix.
`footer_shows_a_count_of_additional_pending_approvals` and
`footer_shows_no_queue_count_for_a_single_pending_approval` (`ui.rs`) cover
the new hint.

Verified: `aldwin-tui` 98 tests pass (94 + 4 new), full workspace build/
test (306 tests) and `cargo clippy -p aldwin-tui --all-targets` clean on
every touched file (`app.rs`, `ui.rs`, `examples/preview.rs` — the latter's
design-iteration harness also constructed the old `Option` fields directly
and needed the same field-name/queue update to keep compiling).

**Progress (2026-09-02, decision panel replaces the inline approval/prompt
card):** Direct developer feedback on the approval-card UX: "when an LLM
needs to ask for permission, or approval a new temporary row shows up in the
chat, it's very ugly, not clear and disjointed" — expected instead "some
kind of universal panel that shows up above the text field input box,
clearly stating what the approval is for and then buttons for
approvals/rejections." Root complaint was structural, not cosmetic: a
pending `ApprovalCard`/`PermissionPrompt` was a `LogEntry`, rendered inline
by `build_log_lines` like any other chat entry — it scrolled with the rest
of the conversation (so scrolling up could carry it out of view entirely,
with no fixed place to look for "what does the harness want from me right
now"), and the only other trace of it was a one-line key hint in the status
line.

Fixed by adding a fifth, fixed-height layout band directly above the input
box (`ui::draw`'s `panel_area`, between the status line and the input),
driven by `App::pending_approvals`/`pending_prompts` rather than `App::log`:
`decision_panel_lines` builds whichever request is at the front of the
queue using the exact same `render_approval_card`/`render_prompt_card` the
old inline card used (now taking a `keys: &str` param instead of always
reaching for `approval_key_hint()`, so the panel and a resolved card's
historical log record can each pass what's right for their own case), and
`render_entry`'s `LogEntry::ApprovalCard`/`PermissionPrompt` arms now render
nothing at all while `resolution: None` — the panel is the only pending-state
UI. Once resolved, the exact same full card (diff included) still renders
inline in the log as a permanent record, completely unchanged from before —
only the *live* interaction moved, not the history a developer might
scroll back to later. `PendingApproval` gained its own `diff: String` field
(mirroring `PendingPrompt`'s existing `payload`) so the panel is
self-sufficient from the queue alone, rather than reaching back into `App::log`
by `call_id` and depending on an invariant ("there's always exactly one
matching unresolved entry") the type system can't enforce.

Two correctness details worth recording. (1) `decision_panel_lines` checks
`pending_approvals` before `pending_prompts` — mirroring `App::handle_key`'s
real priority. The *old* status-line hint checked prompts first, which was
already latently backwards on the rare step where both queues held an entry
at once; invisible before since it only cost a one-line hint mismatch, but
would have shown an entirely wrong request front-and-center in a full panel,
so this fixes it rather than carrying it forward. (2) The panel's own
`Paragraph` needed the same wrap-and-recount discipline `log_row_count`/
`draw_log` already established for the conversation log
(`unstable-rendered-line-info`'s `Paragraph::line_count`) — caught by
visually inspecting a real render (a scratch `#[test]` dumping a `TestBackend`
buffer row-by-row via `eprintln!`/`--nocapture`, same technique as the
2026-08-31 chat-padding pass's throwaway verification, not committed) before
any automated test existed: a permission prompt's title/keys are built from
arbitrary tool-call data (e.g. `PromptPayload::Tool`'s `target`, an
unbounded shell-command string), and the panel's first `Paragraph` had no
`Wrap` at all — ratatui truncates rather than wraps an un-wrapped
`Paragraph`, so a long target would have silently lost content past the
frame's right edge. Fixed by adding `ui::panel_row_count` (identical
technique to `log_row_count`) for the layout's height computation and
`Wrap { trim: false }` on the actual render call, so the two can never
desync the way `log_inner`'s own doc comment describes two earlier
incidents doing. New regression test:
`a_long_permission_prompt_wraps_in_the_panel_instead_of_being_clipped`.

Also new: `ui::clamp_panel`/`panel_max_height`, since the panel is a fixed
`Constraint::Length` band (unlike the old inline card, which relied on the
log's own scrolling to cope with unbounded content) — an Edit call adding
one large new block (all "added" diff lines, none of which
`DIFF_CONTEXT_RADIUS`'s collapsing helps with, since that only elides
unchanged *context*) could otherwise produce a panel taller than the
terminal itself. `panel_max_height` reserves room for at least one row of
the log plus the spacer/status-line/input bands below it; `clamp_panel`
keeps the panel's leading (title/path) and trailing (keys/padding) rows
intact and collapses whatever body doesn't fit into one "⋯ N more lines ⋯"
marker — approving/denying never actually requires scrolling through every
line. Regression test:
`a_very_large_diff_is_truncated_in_the_panel_but_the_buttons_stay_visible`
(asserts the keys are still present after truncation, not just that
truncation happened at all).

The status line (`draw_status_line`) lost its own pending-hint branches
entirely — it now always shows normal turn/activity content, even while a
decision is pending, since the panel is the one place that job belongs now.
This is not misleading: `join_all`-driven parallel tool dispatch means other
non-gated calls can genuinely still be running while one call sits blocked
on approval, so "working…" stays accurate throughout.

Test suite: `aldwin-tui` 104 tests pass (98 + 6 new — the two clipping/
truncation regressions above, plus four asserting the core behavior directly:
pending content is absent from the log, a resolved card's record is
unchanged, the panel stays visible when the log is scrolled away from the
bottom, and the panel's own key hint now appears exactly once instead of
twice). Several existing tests (`approval_card_colors_added_and_removed_
lines_distinctly`, `approval_card_collapses_unchanged_context_beyond_the_
radius`, `diff_lines_show_old_and_new_line_numbers`, and the renamed
`footer_*` key-label tests) were updated to populate `App::pending_approvals`/
`pending_prompts` instead of pushing an unresolved `LogEntry` directly, since
that's no longer where this content renders. Full workspace `cargo test`
(311 tests, 1 ignored, pre-existing) and `cargo clippy -p aldwin-tui
--all-targets -- -D warnings` both clean; a pre-existing, unrelated
`single_match` clippy failure in `aldwin-core::agent.rs`'s own test module
(the same idiom aldwin-tui.md's 2026-08-31 wrapped-row-scroll-math entry
already disclosed for a different file) was confirmed present on the
pre-change tree too, via a stash-based comparison, before ruling it out as
unrelated to this change.

**Progress (2026-09-02, follow-up: wrapped card/diff rows now keep their own
padding, not left as a disclosed gap):** The entry above shipped with a
disclosed cosmetic gap — a card/prompt row wide enough to wrap lost its
1-column left inset and full-width background fill on the wrapped
continuation row, since `filled_line` only ever built one `Line` and left
any further splitting to the caller's own `Paragraph::wrap`, which has no
idea `filled_line` had already inset/filled it. Called out as "known
limitation, not fixed" in the first cut — developer pushback was immediate
and correct: "if it doesn't work properly then it needs to be fixed," not
documented around. Fixed properly rather than patched around: `filled_line`
now does its own wrapping via `wrap_prose_line` (the same word-wrapper
`render_assistant_text`'s `Prose` arm already established for exactly this
class of problem — greedy fill at whitespace, hard-break a single token
wider than the row, preserve per-span styling across a break) *before*
adding any padding, then applies the `BOX_PAD_H` inset and `bg` fill to
*every* resulting row itself. `card_line`/`render_diff_line` (its two
callers) now return `Vec<Line<'static>>` instead of a single `Line`, and
every call site across `render_entry`'s `UserMessage` arm,
`render_assistant_text`'s diff-fence and code-block paths,
`render_approval_card`, `render_card`, and `clamp_panel`'s truncation marker
switched from `.push(...)` to `.extend(...)`/`.flat_map(...)` accordingly —
this was the right layer to fix it at since every filled row in the log
*and* the decision panel goes through this one primitive, not just the
permission-prompt case the same-day entry above already covered.

Verified with the same before/after discipline as every other fix in this
file: a new regression test,
`a_wrapped_card_row_keeps_its_full_width_background_fill`, was confirmed to
fail (right edge showed `BG_BASE`, the frame background, instead of
`BG_ELEMENT`) against a deliberately reintroduced single-`Line` version of
`filled_line` before confirming it passes against the real fix.
`a_long_permission_prompt_wraps_in_the_panel_instead_of_being_clipped`
(the earlier entry's own regression test) needed a small correction once
this landed: it originally asserted the 200-character target appeared as
one contiguous substring, which broke once wrapped rows correctly gained
their own fresh leading inset (a single space now interrupts the run at
each wrap point, which is the fix working, not a regression) — switched to
counting characters (`out.matches('x').count() == 200`) instead, which
verifies the same "nothing was dropped" property without depending on
exact spacing. `aldwin-tui` 105 tests pass (104 + 1 new); full workspace
`cargo test` (312 tests) and `cargo clippy -p aldwin-tui --all-targets --
-D warnings` both clean.

**Progress (2026-09-02, decision panel becomes a numbered, arrow/digit-
navigable list):** Direct developer follow-up on the decision panel: "make
sure the approval options appear as a list and not some weird keyboard
shortcuts, like so: `Approval / 1. Yes / 2. Yes session / 3. No` (key
bindings for 1-3 or selecting with arrow keys and pressing enter are valid
inputs here)." The panel's content already lived in one place (the same-day
entries above); this replaces *how it's chosen*, not where it lives —
raw per-payload letter shortcuts (`y`/`n`; `o`/`s`/`p`/`a` + Shift for
deny-at-tier; `s`/`p`/`n` for context files) are gone outright, replaced by
one generic numbered list every pending gate now shares.

`App` gained `DecisionOutcome` (`Approve(bool)` | `Prompt(PromptResponse)`)
and `DecisionOption { label, outcome }`, plus `App::decision_options()` — a
pure function from whichever request is at the front of the queue (same
approvals-before-prompts priority as everywhere else) to its numbered list,
called by both key handling and rendering so the two can never disagree
about what option N means. `App::decision_selected: usize` tracks the list
cursor, reset to 0 whenever the front of either queue actually changes (a
fresh push into an empty queue, or a pop revealing the next item) — not on
every push, so a second/third item queuing up behind an already-interactive
one doesn't disturb the visible cursor. The old three functions
(`handle_approval_key`, `handle_prompt_key`, `resolve_prompt`) collapsed
into two: `handle_decision_key` (Up/Down move the cursor, clamped rather
than wrapping; Enter confirms whichever option is selected; a digit `1`-`9`
jumps to and confirms that option directly, skipping Enter; anything else is
dropped, no typing ahead) and `resolve_decision` (the shared pop/record/send
tail, keyed on which `DecisionOutcome` variant it got rather than needing to
re-inspect the payload). Ctrl+C is deliberately *not* wired to "whatever the
list's last option is" — a Tool prompt's last option is "always deny," a far
more consequential, harder-to-reverse action than the one-time decline
Ctrl+C has always meant (aldwin-tui.md's 2026-08-29 live-run fix and the
Pitfall below) — `decline_outcome()` maps it explicitly to the same low-stakes
outcome as before, independent of list order.

This is a deliberate, explicit *supersession* of this file's own Pitfall
("Approval card dismissed by an accidental keypress — require an
unambiguous labeled key ... not Enter"), not an oversight: the developer
explicitly asked for Enter as a valid confirm action alongside arrow keys and
digits. What made bare Enter unsafe before was that it looked identical to
every other "just press Enter" action in the app with no visible indication
of what it would do; a numbered list with a visible `▸` cursor showing
exactly which option Enter will confirm removes that ambiguity — the
underlying concern (no accidental, invisible resolution) is satisfied
differently, not dropped. See the Pitfalls section below, updated to record
this explicitly rather than leaving the old wording to read as still-current
guidance it no longer is.

`ui.rs`: new `render_decision_options` renders `"{n}. {label}"` per option,
the selected row prefixed with `▸` and shown in `ACCENT`+bold, everything
else `BRIGHT`. `render_approval_card`/`render_prompt_card`/`render_card`
dropped their `keys: &str` parameter for `pending_tail: Vec<Line<'static>>` —
the *entire* trailing block (numbered options, an optional "(+N more
pending)" queue-count note, and the card's own closing padding), built once
by `decision_panel_lines` and spliced in verbatim when `resolution: None`;
a resolved historical entry still builds its own "resolved: …" line plus
padding directly, ignoring `pending_tail` (callers pass `Vec::new()`).
`clamp_panel` (the large-diff truncation guard from the same-day entry
above) changed its `TAIL` from a hardcoded `2` to a `tail: usize` parameter
sized to the *actual* rendered options-block length: a Tool prompt's full
8-option tier list is a real, common case now (previously it was 1 hint
line), and a fixed guess would either truncate real, selectable options away
or over-protect rows that aren't the list at all.

Verified: `aldwin-tui` 109 tests pass (105 + 4 new — a numbered
Approve/Deny list renders correctly, a Tool prompt's full 8-option list
renders with correct numbering, the `▸` cursor marker moves when
`decision_selected` changes, and the large-diff truncation guard still
keeps the (now multi-row) options list visible) plus the five `app.rs` tests
exercising the old letter-shortcut paths rewritten for digit/arrow/Enter
input instead (`approval_card_enter_confirms_the_default_first_option`,
`approval_card_digit_2_denies_directly_without_enter`,
`approval_card_arrow_down_then_enter_denies`,
`approval_card_arrow_navigation_clamps_at_the_list_ends`,
`permission_prompt_resolves_on_a_numbered_selection_and_records_resolution`,
plus the two queued-request regression tests updated to select by digit
instead of by letter). Full workspace `cargo test` (316 tests) and `cargo
clippy -p aldwin-tui --all-targets -- -D warnings` both clean. Visually
verified via the same disposable `TestBackend`-dump-to-`eprintln!` technique
as the same-day entries above (not committed): both an Approve/Deny list and
a full 8-option Tool-prompt list render with correct numbering and cursor
placement before this was considered done.

**Progress (2026-09-02, rust-skills audit finds and fixes a real
tail-truncation bug in `clamp_panel`):** A 3-pass audit of this session's own
diff, run explicitly against the m01-ownership/m03-mutability,
m15-anti-pattern/coding-guidelines, and m09-domain/m10-performance skills,
surfaced one confirmed, high-severity defect in the numbered-list work above:
`clamp_panel`'s budget math (`keep = max - head - tail - 1`) assumed its own
truncation marker always cost exactly one row. It doesn't — `card_line`
wraps the marker exactly like any other card row once its ~70-column text is
wider than the panel, which is common (any panel narrower than ~70-75
columns), not exotic. The undercounted budget let the *tail* — the options
list, the one thing `clamp_panel`'s own doc comment says must never be cut —
get silently pushed past the panel's real row budget and clipped by the
outer layout. Confirmed via real renders, not just arithmetic: an ordinary
8-option Tool prompt (short title, nothing unusual) on a 50×14 terminal lost
options 5-8 entirely with a nonsensical "0 more lines not shown" marker in
their place (a companion bug — the degenerate case where head+tail alone
already account for the whole panel, so nothing was actually hidden, still
emitted a "hidden" marker it didn't need and couldn't afford); a
long-permission-target Tool prompt at 30×20 lost options 7-8 with no
indication anything was missing at all.

Fixed by replacing the closed-form budget calculation with an iterative
refit: shrink `keep` (how much of the body survives) one row at a time,
re-measuring the marker's *actual* rendered row count (via the same
`card_line` it's built with) on every attempt, until head + kept body +
marker + tail genuinely fit within `max` — or `keep` reaches 0, at which
point no marker is added at all if there was nothing left to hide. This
mirrors the same discipline `log_row_count`/`panel_row_count` already
established elsewhere in this file: measure the real wrapped cost, never
assume it. A remaining, disclosed, much lower-severity gap: `clamp_panel`'s
`HEAD` (the padding+title rows it protects from truncation) is still a fixed
guess of `2`, so an extremely long single-value title (as in the 30×20 case
above) gets abbreviated behind the generic "more lines" marker rather than
receiving the same measured protection now given to the tail — a real
imprecision, but one where the marker still accurately reports what
happened and the options list survives intact either way, unlike the fixed
bug above.

New regression tests, each confirmed to fail against the pre-fix
`clamp_panel` before confirming they pass against the iterative-refit fix:
`a_long_prompt_title_can_be_abbreviated_but_the_full_options_list_must_survive`
and `no_truncation_marker_appears_when_nothing_was_actually_hidden`. Also
fixed in the same pass, lower severity: `card_padding_line`'s
`.unwrap_or_default()` silently masked what its own doc comment already
claims is a guaranteed invariant (empty content never wraps) — switched to
`.expect(...)`, matching this file's own established convention for
guaranteed-invariant unwraps (e.g. `PromptResponse always serialises` in
`app.rs`), so a future regression in that invariant panics loudly with a
clear cause instead of silently rendering an unfilled padding row (the same
class of subtle visual defect this session already spent real effort
tracking down once).

Noted but not changed in this same pass, pending developer confirmation: the
"approvals before prompts" priority predicate appeared independently in four
places (`App::decision_options`, `App::decline_outcome`,
`ui::decision_panel_lines` twice) rather than one shared accessor — a
maintainability risk (the exact bug class already fixed once this session,
in the other direction, at the status-line/`handle_key` boundary), not a
live bug at the time since all four sites agreed.

**Progress (2026-09-02, follow-up: priority check consolidated behind
`App::pending_front`):** Developer confirmed the consolidation above should
happen. New `app::PendingFront<'a>` enum (`Approval(&'a PendingApproval)` |
`Prompt(&'a PendingPrompt)` | `None`) and `App::pending_front(&self) ->
PendingFront<'_>` — the one place "front of `pending_approvals` if
non-empty, else front of `pending_prompts`, else neither" is decided.
`decision_options`/`decline_outcome` now match on it directly instead of
each re-checking `pending_approvals.front().is_some()`;
`ui::decision_panel_lines` does the same (imports `PendingFront` from
`app`), keeping only what genuinely still differs per arm — which
`VecDeque`'s `.len()` feeds the "(+N more pending)" note — inline in each
match arm rather than factored out further, since that part isn't the
duplicated invariant.

New regression test `a_pending_approval_takes_priority_over_an_already_pending_prompt`
(`app.rs`) queues a prompt first, then an approval, and asserts the
approval resolves first through `handle_key` alone (public behavior — the
resolved `Command` and which queue empties — not `pending_front()`'s own
plumbing), so this stays a guarantee about what the developer actually
experiences, not a test of the accessor's internals. `aldwin-tui` 112 tests
pass (111 + 1 new); full workspace `cargo test` (319 tests) and `cargo
clippy -p aldwin-tui --all-targets -- -D warnings` both clean.

`aldwin-tui` 111 tests pass (109 + 2 new); full workspace `cargo test` (318
tests) and `cargo clippy -p aldwin-tui --all-targets -- -D warnings` both
clean.

**Progress (2026-09-02, directory-scope prompt option + humanized prompt
title):** Developer report: permission prompts were "aggressive" for
ordinary reading (each new file under an already-trusted directory
re-prompted individually) and didn't make clear what was actually being
asked ("a human readable explanation... and then underneath in small/
greyed out text what the raw tool call actually is"). Two changes, both
scoped to a pending `PromptPayload::Tool` in the decision panel — no
change to the Approve/Deny binary shape a pending `ToolApprovalRequested`
(Edit) uses, per aldwin's non-negotiable "Edit is never allowlistable."

(1) `render_prompt_card` now shows a per-kind humanized sentence
("Claude wants to read a file") as the accent/bold title, with the
literal `kind: target` demoted to a dim subtitle underneath
(`humanize_prompt`/`raw_prompt_call`, `ui.rs`) — same information as the
old title, just split by primary/secondary instead of concatenated into
one string a developer had to parse.

(2) A new Tab-toggleable scope for the tier options' persisted pattern
(`App::decision_pattern_scope`, `PatternScope::{Exact,Directory}`) —
available only when the payload says `path_like: true` (threaded from
aldwin-tools' new `Tool::permission_target_is_path`, through
aldwin-permissions' `check_tool`) and the target has an enclosing
directory to broaden to (`App::directory_glob`: `"./crates/tui/src/
ui.rs"` → `"./crates/tui/src/**"`; a bare filename with no `/` offers
nothing). Deliberately *not* a 9th option or a doubled allow/deny×scope
list — per this spec's own Pitfall-adjacent discipline (see aldwin-
permissions.md's "four-tier prompt growing a fifth option... each tier
doubles cognitive load") the 8 tier labels stay exactly as they were;
Tab flips which pattern they'd all persist, shown via a new dim hint
line above the list (`ui::scope_hint_line`, `App::decision_scope_hint`)
— "scope: this file (...) · Tab for this directory (...)" and its
reverse once toggled. The chosen pattern rides in a new `pattern` field
on `PromptResponse::Tool` (aldwin-permissions), replacing the
dispatcher's old behavior of always persisting the exact target
verbatim — see aldwin-permissions.md/aldwin-tools.md's matching
Progress notes for the wire-type and dispatcher side. Resets to `Exact`
at the same three points `decision_selected` already resets (a fresh
request becoming the new front, or `resolve_decision` popping to the
next one) — a broadened scope must never leak from one prompt onto an
unrelated one.

New coverage: 11 `app.rs` tests (`directory_glob` unit tests, scope-hint
availability/absence, Tab toggling and its no-op case, the actual
persisted-pattern round trip with and without toggling, and the reset-
on-next-prompt guarantee) and 5 `ui.rs` tests (humanized title + dim raw
call, their relative styling, the hint line's presence/wording in both
scope states, and its absence for a non-path-like target). `aldwin-tui`
128 tests pass (112 + 16 new); full workspace `cargo test` and `cargo
clippy -p aldwin-permissions -p aldwin-tools -p aldwin-tui
--all-targets -- -D warnings` (this pass's actually-touched crates) both
clean. `examples/preview.rs` gained a `prompt_path` scene exercising the
new hint, alongside the existing `prompt` scene (a `shell` target, which
correctly shows no hint at all).

**Progress (2026-09-02, live-feedback batch: descriptive activity, mouse
wheel scroll, light-mode input contrast):** Another round of direct
developer feedback, three items fixed together:

1. **Status line gave no sense of what the model was actually doing.** "The
   status line shows working and thinking, but I wonder if we can be more
   descriptive... perhaps something like Working | Analyzing the project
   structure." A bare "working…" covered the entire stretch of an active,
   non-thinking turn regardless of what was actually happening underneath.
   New `ui::activity_label` distinguishes what `App` already tracks but the
   status line wasn't yet surfacing as the *leading* word: a named tool in
   flight (`tool_gerund`, "reading a file…"/"running a shell command…"/
   "inspecting code…" — present-progressive phrasing of the same handful of
   kinds `humanize_tool_kind` already humanizes for the permission prompt,
   kept as a separate small table rather than shared since the two need
   different grammar), "running N tools…" when parallel dispatch has more
   than one in flight at once, "responding…" once assistant text is
   streaming for the current step (the log's tail entry is a
   `LogEntry::AssistantText` for exactly that stretch — no new state needed),
   or, with neither yet, the honest "working…" for "waiting on the model's
   first token or tool call of this step" — there's no more specific true
   thing to say there. `thinking…` is unchanged (still its own, more
   specific branch, checked first). The trailing `tools:` list (raw tool
   names, colored per name) still shows alongside this, unchanged — the
   headline word is now descriptive, the detailed record is still there too.

2. **"Cannot scroll when selecting text."** Root cause: `run.rs` never
   enabled crossterm's mouse capture at all — every mouse event, wheel
   included, was handled entirely by the terminal emulator, and most
   terminals suppress or reinterpret wheel-scroll while a native selection
   drag is in progress, so the app never even saw the notch to act on.
   Fixed by enabling `EnableMouseCapture` (paired with `DisableMouseCapture`
   in `TerminalGuard`/`restore_terminal`, alongside the existing raw-mode/
   alt-screen restore steps — mouse capture must never leak past the
   process exiting) and a new `App::handle_mouse`, wired into `run_loop`'s
   `tokio::select!` alongside `handle_key`: `MouseEventKind::ScrollUp`/
   `ScrollDown` call the same `ScrollState::line_up`/`line_down` the Up/Down
   keys already use (a wheel notch is a nudge, not a page). Deliberately
   *not* gated on a pending decision the way the keyboard scroll bindings
   are (the decision list's own Up/Down repurposes those keys while
   pending) — the wheel and the numbered decision list are independent
   input channels with nothing to conflict over, so scrolling back through
   history to re-read context while a decision is pending is just useful.
   Every other mouse event kind (click/drag/move) reaches `handle_mouse`
   too, once capture is on, but is a deliberate no-op.

3. **"Selecting and copying text also copies UI elements like the
   scrollbar/input field."** Same root cause and same fix as #2, not a
   separate change: with mouse capture off, *every* click-drag was the
   terminal's own native text selection, which is purely grid-based and has
   no way to know a border/scrollbar/padding column isn't "real" content —
   so even an accidental, casual drag (meant only to scroll or highlight
   for reading) swept up whatever cells it crossed. With capture on, a
   plain click/drag is now an app-level `MouseEvent` `handle_mouse` ignores,
   not terminal selection — in effectively every mouse-capturing terminal
   app (vim's `mouse=a`, htop, tmux panes, ...) the terminal's native
   selection is still reachable, just behind its usual bypass modifier
   (Shift-drag on most terminals, Option-drag on iTerm2), which stops
   *accidental* chrome-capture without removing deliberate copying. This
   is a real behavioral trade-off, not a full fix of "terminal selection
   over a bordered TUI can grab a border character" in general (still true
   of a deliberate Shift-drag, same as any other bordered terminal app,
   and out of this crate's control) — documented here rather than silently
   assumed away. See `aldwin-tui.md`'s Out of Scope bullet below, narrowed
   accordingly (mirrors how the 2026-08-31 sidebar entry narrowed the
   split-pane rejection rather than reopening it outright).

4. **"The colors ... do not work on light mode setups (text is dark on
   light mode and it clashes with the dark background)."** Found by
   auditing every `Style::default()` in `ui.rs` for a content-bearing span
   with no explicit `.fg(...)` — every one of them already set one *except
   exactly one*: `highlight_command_tokens`'s plain-word branch (an
   ordinary, non-command word typed into the input box) fell through to
   bare `Style::default()`, which resolves to the terminal's own default
   foreground. `draw_input`'s box always fills `BG_INPUT`, a fixed dark
   navy, regardless of the developer's terminal theme (see `palette::
   BG_BASE`'s doc comment on why the app paints its own opaque background
   everywhere) — on a dark-themed terminal, "default foreground" happens to
   be light, so this read fine by coincidence; on a light-themed one it's
   typically dark (tuned to sit on a light background), which is exactly
   dark-text-on-the-app's-own-dark-navy: unreadable, for literally every
   plain word the developer typed. Fixed with an explicit `.fg(BRIGHT)` on
   that branch — the same discipline every other span in this file already
   followed. This is a real, narrow, confirmed bug fix, not a light theme:
   the app still paints one hardcoded dark palette regardless of the
   developer's own terminal background (a standing, deliberate choice — see
   the Palette Decisions entry, "Minimal monochrome palette... blocked on
   the mascot palette question"), it just no longer depends on the
   terminal's ambient default color anywhere, so it can't clash with it
   either. `TuiConfig.theme: Option<String>` (aldwin-config) already exists
   in the schema for a future real second (light-tuned) palette, but every
   past palette change in this file was screenshot-verified against a real
   render before shipping (see the `examples/preview.rs` harness and its
   tmux-capture pipeline, used throughout this file's history) — hand-
   picking a second full set of RGB values with no way to visually verify
   them in this session would risk trading one bad-contrast report for
   another, so that's flagged here as real, separable follow-up work rather
   than attempted blind.

New tests: `status_line_describes_the_running_tool_instead_of_a_generic_
working_label`, `status_line_says_running_n_tools_when_more_than_one_is_in_
flight`, `status_line_shows_responding_once_assistant_text_is_streaming`
(item 1); `mouse_wheel_scrolls_the_log_by_one_line`, `non_scroll_mouse_
events_are_ignored`, `mouse_wheel_scrolls_the_log_even_while_a_decision_is_
pending` (item 2, `app.rs`); `highlight_command_tokens_gives_plain_words_an_
explicit_bright_fg` (item 4, replacing the old assertion that plain words
were unstyled — that was the bug, not the spec). `status_line_shows_
activity_running_tools_and_message_count` (pre-existing) updated: it
asserted the old generic "working" text was still present once a tool
started running, which is no longer true by design once item 1 landed.
`aldwin-tui` 134 tests pass (128 + 6 new: 3 in `app.rs` for item 2, 3 in
`ui.rs` for item 1 — item 4's test replaced an existing one rather than
adding a new one, since the old assertion was pinning down the bug); full
workspace `cargo test` (347 tests) and `cargo clippy -p aldwin-tui
--all-targets -- -D warnings` both clean.

**Progress (2026-09-02, self-review of the batch above finds a real gap in
item 1):** A manual audit of this session's own diff (correctness pass over
`activity_label`/`run.rs`/`handle_mouse`, plus the aldwin-config side of
the same batch) found one confirmed, low-severity defect: `activity_label`'s
single-running-tool arm read `one.name` directly, not through the same
empty-name→`call_id` fallback (`App::apply_event`'s doc comment on
`pending_tool_names` notes the lookup this falls back from can in principle
miss) the trailing `tools:` list already applied — so a `RunningTool` with
an empty name would have shown the leading activity word as a bare "using …"
with nothing after "using ", while the list right next to it correctly fell
back to the call id. New `ui::running_tool_name`, shared by both sites, so
the two can't drift apart on this again. Confirmed via a deliberate revert
of just the fix: `status_line_falls_back_to_the_call_id_for_a_running_tool_
with_no_name` fails (asserting on the literal "using …" the pre-fix code
produces) before confirming it passes against the shared-helper fix.
`aldwin-tui` 135 tests pass (134 + 1); full workspace `cargo test` (348
tests) and `cargo clippy -p aldwin-tui --all-targets -- -D warnings` both
clean.

**Progress (2026-09-02, DIM/BRIGHT were still terminal-remappable — the
light-mode item above was only half fixed):** Direct developer pushback,
correctly: "you're not testing with a light mode color scheme, so you cannot
reproduce the issue." True — the earlier fix in this same batch (item 4,
`highlight_command_tokens`'s missing `.fg(BRIGHT)`) was verified with an
`xterm -bg white -fg black` session, which only overrides the terminal's
*default* fg/bg (what `Color::Reset` resolves to). It never touched the
ANSI 16-color palette, so it couldn't have exercised — or caught — the
actual remaining bug: `palette::DIM`/`BRIGHT` were `Color::DarkGray`/
`Color::White`, named ANSI indices (8/15), not fixed RGB — the one exception
to this file's own stated rule (see `USER_FG`'s doc comment: "Fixed RGB
rather than a named ANSI color so the tint doesn't get reinterpreted by
whatever the terminal theme maps that ANSI slot to"). A real light-mode
terminal theme remaps those slots for *its own* readability against a light
background, independent of whatever this app tries to paint. Reproduced
directly this time: an xterm session configured with Solarized Light's
actual, published 16-color ANSI table (a widely-used real scheme, not a
synthetic worst case) renders index 8 as `#002b36` — near-black navy. Every
`DIM`-styled span (status-line metadata, timestamps, dim labels) came out
dark-navy-on-this-app's-own-dark-navy-background, essentially unreadable.
`BRIGHT` (index 15 → Solarized's `#fdf6e3`, a light cream) happened to
survive in that specific palette, but was exposed to the identical failure
mode by construction — not proof it was safe, just that this one example
palette didn't happen to trip it.

Fixed by giving `DIM`/`BRIGHT` fixed RGB values (`palette.rs`), same
treatment as every other constant in that file, chosen to read clearly
against the `BG_BASE`/`BG_ELEMENT`/`BG_INPUT` dark-navy family regardless of
any terminal palette. Verified against the same reproducing Solarized Light
xterm session (real 16-color `-xrm` overrides, not just `-bg`/`-fg`) before
and after: the before capture shows the status-line metadata essentially
invisible; the after capture, same palette, same scene, shows it clearly
legible. `aldwin-tui` 135 tests pass unmodified (no test hardcoded the old
`Color::White`/`Color::DarkGray` values directly — all reference the `DIM`/
`BRIGHT` constants, which is exactly why none needed touching); full
workspace `cargo test` (348 tests) and `cargo clippy -p aldwin-tui
--all-targets -- -D warnings` both clean.

Lesson recorded plainly since it's a real process gap, not just a code one:
"tested in a light-mode terminal" needs a real remapped ANSI palette, not
just a flipped default fg/bg — the two are different mechanisms, and this
file's own established verification technique (a real terminal session,
captured, not just `TestBackend`'s text-only dump) is what actually caught
this the second time.

**Progress (2026-09-02, an actual light theme, not just a dark theme that
survives a light terminal):** Direct developer follow-up once the item
above shipped: "can't we provide a basic light color scheme? for users who
went a light color scheme." A fair distinction from what the two entries
above actually did — those made the one *hardcoded dark* palette immune to
the terminal's own theme; neither gave a developer who wants the *app
itself* to look light (matching a light terminal, not fighting it) any way
to get that.

`palette.rs` is now a `Palette` struct (every color as a field) with two
fixed instances, `DARK` (`DARK`'s values are `aldwin-tui`'s pre-existing
palette verbatim — zero visual change for anyone not opting in) and `LIGHT`
(new: a light lavender-white background family mirroring `DARK`'s indigo-
slate hue rather than flat neutral white, and every foreground/accent/diff/
tool color independently re-tuned for contrast against *that* scale, not
just inverted). One deliberate exception: fenced code blocks (`code_bg`)
stay the same dark value in *both* themes — a code block reading as its own
distinct dark box is a common, expected pattern independent of the app's
own theme (many light-themed editors keep code blocks dark), and it means
`highlight.rs`'s syntect theme (`base16-ocean.dark`) never has to change
per app theme either.

Selected once at startup via a new `Theme` enum (`Dark`/`Light`) and
`Theme::from_config`, which finally wires up `TuiConfig.theme: Option<
String>` — present in aldwin-config's schema since V0 but never read by
anything until now. `App` gained a `theme: Theme` field (`App::with_theme`,
a builder method rather than a new `App::new` parameter, so the many
existing `App::new(model, permissions)` call sites — tests, `examples/
preview.rs`, aldwin-cli's bootstrap — didn't all need to thread a theme
through just to keep their existing `Dark` default); `aldwin-cli`'s
bootstrap resolves `config.global_tui().theme` into a `Theme` and passes it
into `aldwin_tui::run`, which threads it into `App::new(...).with_theme
(theme)` before the first draw. `tui.yaml`'s annotated header documents the
field's two values.

Deliberately *not* a global/`OnceLock` "current theme" — `ui.rs`'s ~20
render functions that touch color either already take `app: &App` (five of
them: `draw`, `draw_log`, `decision_panel_lines`, `draw_status_line`,
`draw_input` — each does `let pal = app.theme.palette();` once and reads
`pal.accent`/`pal.dim`/etc. from there) or now take an explicit `pal:
&Palette` parameter threaded through the whole pure call graph beneath them
(`render_entry` → `render_assistant_text` → `render_markdown_line` →
`parse_inline`, `card_line`/`card_padding_line`/`filled_line`,
`render_approval_card`/`render_prompt_card`/`render_card`,
`render_decision_options`, `render_diff_line`/`diff_gutter`, `tool_color`,
`clamp_panel`, `access_spans`/`intro_content`/`hero_lines`,
`highlight_command_tokens`). A shared mutable "current theme" would have
made one test's theme choice leak into another's — this crate's `cargo
test` runs many tests in parallel inside one process — so explicit
threading keeps every test (and every real render) self-contained
regardless of scheduling; see `palette.rs`'s own module doc comment for the
full reasoning. *Superseded same-day, below:* "runtime theme-switching
mid-session was never asked for and isn't supported" turned out to be true
for about as long as it took to ask — `App::theme` living as a plain field
(not the global this paragraph deliberately avoided) is exactly what made
switching it live a small addition rather than a redesign once the
developer did ask; see the `/theme` Progress entry below.

Verified the same way every dark-theme change in this file's history was —
not from the RGB values alone: a real xterm session (Xvfb, no tmux — this
session's own transcript explains why tmux specifically was avoided)
running the actual built preview harness (`examples/preview.rs`, extended
with an optional theme argv so a scene can be screenshotted in either
theme), screenshotted across the `conversation` (prose + a fenced code
block, confirming `code_bg` correctly stays dark), `approval` (the diff
card — the highest-risk element for light-mode contrast), `tools` (parallel
running-tool name colors), and `long` (retry/error rows, plus a same-scene
dark-theme capture confirming zero visual regression there) scenes, in a
plain default xterm with no color overrides at all — proving the palette is
now fully self-contained and doesn't depend on the terminal's theme in
either direction. New tests in `palette.rs`: `from_config` case-
insensitivity and its default-to-`Dark` fallback (`None`, `"dark"`, garbage,
empty string), `DARK != LIGHT`, and a cheap real-invariant check (`LIGHT`'s
background is lighter and its primary text darker than `DARK`'s, by luma).
`aldwin-tui` 140 tests pass (135 + 5 new — no existing test needed touching
beyond passing `&DARK`/`&palette::DARK` explicitly at pure-function call
sites, since `DARK`'s values are unchanged from what those tests already
asserted against); full workspace `cargo test` (353 tests) and `cargo
clippy -p aldwin-tui -p aldwin-cli --all-targets -- -D warnings` both
clean.

**Progress (2026-09-02, `/theme` — switching from inside the harness, live,
no restart):** Direct developer follow-up to the light-theme entry above:
"shouldn't we add a slash command for users to select the theme from
inside the harness itself?" aldwin-cli's interceptor gained `/theme
light|dark` (see its own archived spec's post-archive addition for the
command/config side) and aldwin-core gained `Event::ThemeChanged { theme:
String }` (see its own matching post-archive addition) as the vehicle to
reach a *running* TUI — the interceptor has no other way in, since it and
the TUI only share the one `Event` channel.

`App::apply_event`'s new arm is one line: `self.theme = Theme::from_config
(Some(&theme))` — reparsing the raw string through the same fallback
`Theme::from_config` already applies at startup (anything but `"light"`
means dark), rather than trusting the interceptor's own validation, so the
"unrecognized means dark" rule lives in exactly one place. Nothing else
needed changing: `App::theme` was already a plain field, not the global
this same file's own Progress entry (immediately above) deliberately
avoided — `ui::draw` reads it fresh via `app.theme.palette()` on *every*
frame, no caching to invalidate — so live-switching, which the entry above
called out of scope, turned out to already be supported by construction
once the developer actually asked for a way to trigger it.

Verified two ways, not just by reading the diff: `theme_changed_switches_
the_active_theme` and `theme_changed_with_an_unrecognized_value_falls_
back_to_dark` (`app.rs`) exercise `App::apply_event` directly; a scratch
example (Xvfb + xterm, not committed, same technique as every other real-
render check in this session) constructed one `App`, drew it once in Dark,
called `app.apply_event(Event::ThemeChanged { theme: "light".into() })` on
that *same* instance — exactly what the real event loop does, no
reconstruction — and drew it again: the second frame showed the identical
log content in Light with no restart, confirming the live-switch claim
this entry makes rather than assuming it from the code alone. A third gap
caught only by remembering to check, not by the compiler:
`KNOWN_COMMAND_WORDS` (`ui.rs`) — the hand-kept duplicate of `cli::slash::
intercept`'s dispatch table that drives the input box's live "dim a
recognized command word as you type" hint — hadn't grown a `/theme` entry,
so the new command would have typed as plain text with no visual cue it
was headed for the harness rather than the model, unlike every other real
command. Fixed, with a new regression test
(`theme_command_word_is_dimmed_live_like_every_other_known_command`)
guarding that one constant specifically, the same way
`command_word_is_dimmed_live_even_mid_message` already guards `/exit`.
`aldwin-tui` 143 tests pass (140 + 3 new); `aldwin-cli` 27 tests pass
(22 + 5 new, covering the no-argument report, persist-and-emit for both
directions, case-insensitivity, and invalid-value rejection neither
persisting nor emitting `ThemeChanged`); full workspace `cargo test`
(361 tests) and
`cargo clippy -p aldwin-tui -p aldwin-cli --all-targets -- -D warnings`
both clean (`cargo clippy -p aldwin-core` — the lib itself, not its
pre-existing test-module `single_match` finding disclosed in the
2026-09-02 self-review Progress entry above and confirmed unrelated via a
stash comparison — also clean).

- **Layout (rewritten 2026-09-02 onto the Mjolnir Design System — see that Progress entry for the full account):** Seven bands top to bottom: a persistent 3-row top bar (`draw_top_bar` — `aldwin` identity left, model/version right) and its 1-row rule; the body (full-width scrollable conversation log — no sidebar; the 2026-08-31/09-02 sidebar was already fully removed from the code before this pass, this corrects prose that had drifted stale); a 1-row spacer; a 1-row status line (live activity — `draw_status_line`); the decision panel (zero-height and invisible whenever nothing is pending); and the multi-line input area. `panel_max_height` reserves the top bar's and the decision panel's own chrome (title band + footer, applied outside `clamp_panel`'s budget) explicitly, so the two never silently exceed the frame between them.
- **Conversation Log:** Rendered inside a borderless, opaque `ground`-filled ratatui panel with a `Scrollbar` shown when content overflows the viewport. Append-only rendered view of core events; the welcome hero (see the 2026-09-02 Progress entry) only shows when the log is empty. Each `UserMessage`/`AssistantText` entry leads with a one-line `you`/`harness` speaker label (`speaker_you`/`speaker_agent`) — no filled chat-bubble background any more (2026-09-02; the design system's own `Prose`/`Turn` components carry none). Tool-activity entries lead with `●` (done, `glyph_done` or `del` on error) / `◐` (running, `glyph_running`) — no bracketed text tag, no `▸`/`✓`/`✗`. `RetryAttempt`/`Error`/`Notice` lead with a colored lowercase label word instead of a glyph (the design system's fixed glyph table has no roles for any of the three). Tool activity (ToolDispatched → ToolCompleted) renders inline as grouped entries per step. ThinkingStart/an active turn with no thinking block show an animated spinner (`◐◓◑◒`, 2026-09-02, replacing the earlier Braille cycle) in the status line only. Scroll: auto-follows new content when the view is at the bottom; disengages when the user scrolls up; re-engages on End. Line scroll via arrow keys or the mouse wheel; page scroll via PgUp/PgDn.
- **Decision Panel (chrome rewritten 2026-09-02 onto the design system's Permission screen — behavior unchanged):** ToolApprovalRequested/PromptRequested render in a fixed full-width panel above the input (`ui::decision_panel_lines`), never inline while pending. Chrome: an accent-700 top rule, a title band (`permission`, no glyph, the payload's own kind badge right-aligned in `gauge_fill`), the body (humanized title, diff or raw call), the numbered options list (`▌` mark colored `mark`/`mark_idle` by selection, paired with a `band` background — no separate cursor glyph, no per-row shortcut letters), and a footer (`↑↓ to move   1-N to pick   ⏎ to confirm` left, `saved to .aldwin/permissions.yaml` right). Enter confirms the selected option, a digit `1`-`9` jumps to and confirms one directly, Ctrl+C always resolves the safe one-time decline regardless of cursor position. Input is blocked while pending. Once resolved, the full card (diff included) still renders inline in the log as a permanent historical record.
- **Input Area:** Multi-line textarea with a visible terminal cursor, an accent `▶` prompt glyph on its first line only (2026-09-02, `Composer.jsx`), dim placeholder text when empty, and Up/Down line navigation within the draft. Any word matching a known slash command dims live, anywhere it's typed on any line. Enter submits; Shift+Enter inserts a newline. Ctrl+C cancels the active turn or exits when idle. Input is blocked while a decision is pending.
- **Status Line / Top Bar (2026-09-02, restructured onto the design system — see that Progress entry):** `draw_top_bar` is the static identity row (harness name, model, version) at the very top of the frame; `draw_status_line`, one row directly above the decision panel/input, is the live-activity row: a spinner plus activity label, model name, turn/step counter, any tools currently in flight (in `value`, uniformly — not hashed per name any more, since the design system's own `ToolLine` doesn't color by tool identity), a running message count, and a right-aligned Ctrl+C hint. There is no sidebar. Neither band participates in `ScrollState` — only the log panel scrolls.
- **Palette (rebuilt 2026-09-02 onto the Mjolnir Design System — see that Progress entry for the full field-by-field account):** `palette.rs`'s `Palette` struct now mirrors the design system's own `--tui-*` semantic tokens (31 fields — `ground`/`bar`/`bar_bottom`/`line`/`rule`/`text`/`body`/`code`/`context`/`value`/`label`/`dim`/`quiet`/`mark`/`mark_idle`/`band`/`accent_text`/`speaker_you`/`speaker_agent`/`gauge_fill`/`gauge_track`/`glyph_done`/`glyph_running`/`glyph_pending`/`hunk_header`/`modal_line`/`diff_box`/`add`+`add_bg`+`add_code`/`del`+`del_bg`+`del_code`), not the old flat `dim`/`bright`/`user_fg`/`bg_*`/`code_*`/`diff_*_fg`/`diff_*_bg`/`warning_fg`/`panel_border`/`tool_palette` set — every value is the design system's own resolved hex (dark ground `#161826`, accent dusty azure `#84aed9`), not hand-picked. `DARK`/`LIGHT` selected once at startup via `Theme::from_config`/`TuiConfig.theme`, switchable live via `/theme`, threaded explicitly through `ui.rs`'s render functions — unchanged mechanism, only the field set changed. Text hierarchy is now `text` (primary, the "you" turn's content) / `body` (agent prose) / `code` / `context` (stdout) / `value` (right-flush facts) / `label` (muted labels) / `dim` (dimmest metadata) / `quiet` (quietest tier) — replacing the old two-tier `bright`/`dim` split. Rules are flat, single-color (`rule`, one step more muted than `line`) — not Nocturne's fading-gradient signature the design system's own token layer still ships but whose revision log marks unused by the actual reference screens. No per-tool-name color hashing any more (`tool_color`/`TOOL_PALETTE` removed outright — the reference `ToolLine` doesn't do this). Permission allow/deny states (`access_spans`) render as plain colored text (`add`/`del`), not a filled chip — the design system's own rule is that the accent is "a mark or a line, never a filled field," and none of its components use a background-filled badge for a state word. Fenced code blocks sit on `diff_box`, the system's one nested-quote surface, in both themes. *(Corrected 2026-09-03 — see that Progress entry: this was a fixed theme-invariant `CODE_SYNTAX_BG` while `highlight.rs` was pinned to a dark `syntect` theme; the highlighter now picks the matching half of the `base16-ocean` pair from `Palette::theme`, so the surface no longer has to stay dark in a light session.)*

**Progress (2026-09-02, full visual redesign onto the Mjolnir Design System):**
The developer imported a new design system (`claude.ai/design`, project "Aldwin
Design System", plus a second scratch project "Design system tokens
discussion" holding the actual handoff bundle and its revision log — the
source of record for every concrete decision below) and asked for it applied
to aldwin's existing functionality: same interactions, new look, small
layout refactors acceptable, no new features. This is a visual/layout pass
only — every keybinding, gate, queueing, and scroll behavior this file
already documents is unchanged; only how it's drawn changed.

**Tokens.** `palette.rs`'s `Palette` struct was rebuilt field-for-field to
mirror the design system's `--tui-*` semantic tokens (`ground`/`bar`/
`bar_bottom`/`line`/`rule`/`text`/`body`/`code`/`context`/`value`/`label`/
`dim`/`quiet`/`mark`/`mark_idle`/`band`/`accent_text`/`speaker_you`/
`speaker_agent`/`gauge_fill`/`gauge_track`/`glyph_done`/`glyph_running`/
`glyph_pending`/`hunk_header`/`modal_line`/`diff_box`/`add`+`add_bg`+
`add_code`/`del`+`del_bg`+`del_code`, 31 fields total) rather than the old
flat `dim`/`bright`/`user_fg`/`bg_base`/`bg_element`/`bg_input`/`code_fg`/
`code_bg`/`diff_add_fg`/`diff_add_bg`/`diff_del_fg`/`diff_del_bg`/
`warning_fg`/`panel_border`/`tool_palette` set — every value is the design
system's own resolved hex, not re-derived (dark ground `#161826`, accent
dusty azure `#84aed9`, diff added hue 145 `#70cf75`/removed hue 24 `#e86c68`,
etc.); the two `_bg` diff tints are the one exception, hand-blended over
`diff_box` since ratatui has no alpha-blend primitive (documented inline
with the source rgba + blend base, so the arithmetic is checkable). `LIGHT`
mirrors the design system's `.tui-light` scope, including the rule that the
selection band must be *darker* than the page there, not lighter.
`SPINNER_FRAMES` changed from a 10-frame Braille cycle to the token layer's
own `◐◓◑◒` (`tokens/motion.css`). `WARNING_FG` has no equivalent in the new
token set (the system's vocabulary genuinely has no "warning" role) —
`RetryAttempt` now reads as a bold `label`-colored "retry" tag plus `dim`
detail instead.

**Glyph vocabulary.** The design system's Iconography table is small and
fixed (`▌ ● ◐ ○ ✔ ▶ █ + -`) with an explicit rule: "if a mark is needed and
it is not in that table, do not draw one." `render_entry`'s tool-activity
arm now reuses `●`/`◐` (done/running) rather than the old `▸`/`✓`/`✗`, with
a failed call staying `●` but recolored to `del` — color carries the
distinction, not a new glyph. `RetryAttempt`/`Error`/`Notice` dropped their
`⟳`/`✗`/`ℹ` glyphs entirely in favor of a colored lowercase label word
(`error:` in `del`, `notice:` in `quiet`), for the same reason.

**Top bar (new — see the Layout Decision bullet below for why this is a
structural addition, not a bare reskin).** Every one of the design system's
five reference screens opens with a persistent 3-row identity bar plus a
1-row rule (`TopBar.jsx`, `tokens/cells.css`'s `--bar-top-h`); the
2026-08-31 redesign had folded identity into the single status line instead.
`draw_top_bar` restores it: `aldwin` in `text`, no glyph (the design
system's own revision log: "the top bar carries no accent mark: the name is
the brand, and a pip there indicated nothing") on the left; model name and
build version on the right (the reference's `model · gauge · cost` group
doesn't port literally — aldwin tracks neither a context-window gauge nor a
per-session cost anywhere in `StatusInfo`, so neither is fabricated).
`draw_status_line` keeps the live-activity job it already had (spinner,
activity label, turn/step, running tools, message count) and gains a
right-aligned Ctrl+C hint (`^c to cancel`/`^c to exit`, matching
`StatusLine.jsx`'s own `right` prop, adapted to Aldwin's real binding).

**Hero.** The hand-traced Braille hammer (`MJOLNIR_ART`) and FIGlet
wordmark (`WORDMARK_ART`) are gone outright — not trimmed, removed — per the
design system's explicit, repeated rule: "No logo... every mark is a
Unicode box-drawing or block character," and its Assets section: "None. No
images, no icons." `intro_content` is now the tagline plus `model`/
`version`/`commit`/`access` fact rows on the transcript's label-column
convention; `log::INTRO_LINE_COUNT` dropped from 18 to 6 to match.

**Transcript.** `UserMessage`/`AssistantText` dropped the filled
chat-bubble background entirely (`Prose.jsx` is plain colored text on the
panel ground, no fill in the reference) in favor of a one-line `you`/
`harness` speaker label (`speaker_you`/`speaker_agent`) with content
indented under it (`SPEAKER_INDENT`, a compact stand-in for `Turn.jsx`'s
literal 12-cell label column plus 2-cell gutter, which doesn't port to a
variable-width ratatui panel). Fenced code blocks kept their filled-box
treatment (the design system has no generic "code block" component to
match, only `InlineDiff`) but moved off the old `CODE_BG` onto a new fixed
`CODE_SYNTAX_BG` — still theme-invariant, for the same reason `CODE_BG` was
(`syntect`'s `base16-ocean.dark` theme has no light counterpart) — picked
noticeably darker than `DARK.ground` after a first pass nearly matched it
and the block all but disappeared in a real dark-mode render. *(Superseded
2026-09-03: the premise was wrong, not the reasoning — syntect bundles a
light `base16-ocean` too, and `Palette::theme` says which is wanted, so the
constant is gone and the block sits on the themed `diff_box` in both.)* Diff
rendering (`render_diff_line`) now splits the `+`/`-` sign color from the
code-text color (`add`/`del` vs. `add_code`/`del_code`), matching
`InlineDiff.jsx` exactly instead of one merged color.

**Decision panel.** The source's own revision log settled this screen
*back* onto a bottom-anchored full-width panel after trying a centered
modal-with-scrim — i.e. the exact shape aldwin already had from the
2026-09-02 decision-panel entry above — so no layout reversal was needed
here, only chrome: a `panel_band` (accent-700 top rule + an accent-900
title band reading `permission`, no glyph, with the payload's own kind
right-aligned in `gauge_fill` — "bash"/"read"/"edit"/etc., mirroring
`Modal.jsx`'s `badge` prop) and a footer (`↑↓ to move   1-N to pick   ⏎ to
confirm` left, `saved to .aldwin/permissions.yaml` right, no `esc to
close` — a permission has to be answered, so there's no escape hatch key to
advertise). Both are assembled *outside* `clamp_panel`'s truncation budget
now (`decision_panel_lines` applies them after clamping, not before) —
they used to share the same protected-but-still-counted tail the options
list did, which on a real terminal (not just a torture-test one) left too
little budget for the tail itself once the new chrome's fixed overhead was
counted too; `panel_max_height` reserves the top bar's 4 rows and the new
chrome's fixed 5 (band 2 + footer 3) explicitly so the total never drifts
out of budget the way that first cut did. `render_decision_options` swapped
the old `▸`-vs-blank cursor marker for `OptionRow.jsx`'s own convention:
the `▌` mark is always drawn, colored `mark` (selected) or `mark_idle`
(not) — selection is color-only, paired with the `band` background, never a
distinct glyph.

**Composer.** `draw_input` gained `Composer.jsx`'s accent `▶` prompt glyph
on the textarea's first line (continuation lines don't carry it — multi-line
drafts are aldwin's own extension beyond the reference's single-line
composer). This is the one place the redesign touched the cursor-placement
code this file's history treats carefully: the glyph's 2-column width has
to be added back into `draw_input`'s `set_cursor_position` math for line 0
specifically (not every line), guarded by a new regression test.

**Sidebar.** Already fully removed from the actual code before this pass
started (only stray doc-comment mentions remained, cleaned up here) — the
2026-08-31/09-02 Design-section prose describing a Ctrl+T sidebar was stale
relative to the code even before the design-system import; this pass is
what finally corrects that prose (see the Design section below).

**Verified:** 139 `aldwin-tui` tests pass (138 existing, reworked in place
where they pinned old field names/glyphs/wording, plus one new regression
test for the composer cursor fix — none deleted for coverage, only for
features that no longer exist: the mascot-art shape/gradient tests and the
per-tool-name color test). Several tests needed taller `TestBackend`
heights, the same category of change this file's history already
describes doing repeatedly (widening for the ANSI Shadow wordmark, again
for the chat-padding pass) — the new top bar plus decision-panel chrome
raises the realistic minimum terminal size a full 8-tier permission prompt
needs room for. Full workspace `cargo test` (356 tests) and `cargo clippy -p
aldwin-tui --all-targets -- -D warnings` both clean; a pre-existing,
unrelated `single_match` clippy failure in `aldwin-core::agent.rs`'s own
test module (already disclosed in this file's 2026-09-02 self-review
Progress entry above) reconfirmed present with this pass's changes stashed
out, via the same stash-comparison discipline that entry established.
Screenshotted via `examples/preview.rs` (all six scenes, dark and light)
through a Xvfb + xterm + ImageMagick `import` pipeline — not tmux, per
standing developer instruction — the same "real terminal render, not just
`TestBackend`'s text-only dump" discipline this file's palette-work
Progress entries have used throughout; the `CODE_SYNTAX_BG`/ground
near-collision above was caught this way, not by any automated check.

**Progress (2026-09-03, fidelity correction — comparing against the actual
handoff HTML, not just its component source):** Direct developer pushback on
the pass above: "layout and general UX is not correct... colors appear to be
completely wrong... I expected a very careful and very detailed overhaul."
Root cause of the gap, found on review: the first pass was built from reading
the design system's `.jsx` component source and `readme.md` prose and
translating by hand — the actual pixel-exact reference (`Agent TUI v2.dc.html`
in the second, "tokens discussion" project, plus its own revision log) was
fetched but never *rendered and looked at* as an image; a base64 blob sitting
in tool-call context isn't the same as seeing it. Corrected this session by
saving the handoff HTML plus its real token CSS to disk and rendering it with
headless Chromium (`--headless --screenshot`, run from `/root` — the snap's
own confinement silently no-ops a write anywhere else, including `/tmp`,
which cost real time to diagnose) — true ground truth, not a re-derived
approximation. Pixel-sampled the render against this crate's own screenshots
first (`convert ... txt:-`): the palette hex values themselves matched
exactly (`#232532` top bar, `#161826` ground, bit-for-bit) — the color
*tokens* were never wrong; what was wrong was layout structure and which
token got applied where. Confirmed fixes, all against the rendered reference
directly:

- **Turn layout was fundamentally wrong.** The reference lays a turn out as
  two columns *on the same rows* — a label gutter (`you`/`harness`) beside
  its content, from the very first row — not a label on its own line with
  content indented underneath, which is what the first pass shipped.
  `render_entry`/`render_assistant_text` rebuilt around a new
  `with_label_column` helper (`CONTENT_INDENT` = a fixed gutter width): the
  label sits on row 0 only, every other row — including tool-activity/retry/
  error/notice entries, which continue the current turn rather than
  starting one — gets a matching blank prefix instead. `ToolActivityStatus`
  summaries are now right-flushed (`justified_line`) instead of just
  trailing after two spaces, matching `ToolLine.jsx`.
- **No rule between turns.** `build_log_lines` only ever inserted a blank
  `Line::default()` between entries; the reference draws a real flat `rule`
  row between a fresh `UserMessage`/`AssistantText` and whatever came
  before (not between a turn and its own tool-activity continuation, which
  stays blank-only, matching the reference's own internal spacing).
- **No visible border on a quoted diff.** `InlineDiff.jsx` draws a real
  one-cell box (`┌─…─┐`/`│`/`└─…─┘`); the first pass used only a flat rule
  above/below, invisible enough in a screenshot to read as "no diff
  treatment at all." New `diff_box_border`/`boxed_line` draw the real
  border, used by both the approval card's diff and a fenced ` ```diff `
  block in assistant prose (`boxed_diff_lines`).
- **Permission body sentence was bold+accent; the reference is plain
  `body`.** The title *band* above it already carries the accent weight;
  the sentence itself ("The agent wants to run a shell command.") is
  unstyled prose. Fixed in `render_approval_card`/`render_prompt_card`.
  Also: numbered options had a stray period ("1. Allow once") the reference
  doesn't — `render_decision_options` dropped it.
- **A shell command had no `CommandBlock.jsx` treatment.** It read as a dim
  `shell: {command}` line, the same shape as any other prompt kind. New
  `command_block_lines`: a `ground`-colored field with an accent `$`
  prompt, used only when `kind == "shell"` — other kinds keep the plain
  line, since a `$` prompt doesn't mean anything for a file path.
- **Wrong voice: "Claude wants to..." instead of "The agent wants to..."**
  — `readme.md`'s Content Fundamentals says third person for the model
  when the harness is speaking about it; fixed across every
  `humanize_*`/`ApprovalCard` title.
- **Top bar showed no working directory.** The reference's identity group
  is always `aldwin   ~/src/gateway` (`· branch*` too, but aldwin tracks
  no git state anywhere and a runtime git shell-out is a real new
  capability, not a display fix — left out, disclosed, not silently
  faked). `current_dir_display` adds the cwd, `~`-shortened, using only
  already-available process state (`std::env::current_dir`).
- **Decision-panel footer's own separator used the wrong token** (`rule`,
  the muted one, instead of `line`, which is what the reference's real
  `border-top: 1px solid var(--tui-line)` resolves to) — and the rule the
  reference draws *above the options list itself* (between the meta content
  and the numbered list) didn't exist in the first pass at all; both fixed
  in `decision_panel_lines`/`card_rule` (which now takes an explicit `fg`
  instead of a two-state guess).

Verified two ways per fix, not just by reading the diff: `cargo test`
(140 `aldwin-tui`, 358 workspace, all passing — several existing tests
needed taller `TestBackend`s, since the corrected chrome has a real,
larger minimum size than the first pass's under-built version did, the
same category of change this file's history already describes doing
repeatedly) and a second round of `examples/preview.rs` screenshots,
compared frame-by-frame against the same Chromium-rendered reference used
to find the bugs — not just against the first pass's own (wrong) output.
`cargo clippy -p aldwin-tui --all-targets -- -D warnings` clean.

Lesson recorded plainly since it's a real process gap: reading a design
system's component source and prose is not the same as looking at its
actual rendered output, even when both are "available" in the same
fetch — a base64 image blob in tool-call context has to be saved to a file
and viewed to count as having been seen at all.

**Progress (2026-09-03, grid + bottom-bar audit — third round of developer
feedback):** the developer reported that "the chat rows themselves appear
misaligned and do not follow the cell/grid system," that "the status line is
above the text field input, but... it is below in the designs," and asked for
a deep dive into the TUI design files themselves rather than the token layer
alone. Both reports were correct. Measured against `Agent TUI v2.dc.html`'s
own markup (its `padding`/`flex-basis` values divided by the 9x20px cell
from `tokens/cells.css`) and fixed:

1. **The grid.** `MARGIN_X` = 3 cells (`--margin-x: 27px`), `LABEL_COL_WIDTH`
   = 12 (`--label-col: 108px`), `LABEL_GUTTER` = 2 (`--label-gutter: 18px`),
   so `CONTENT_INDENT` = cell 17 (`--body-col: 153px`). Every speaker row now
   puts its label in cell 3 and its prose in cell 17. Option rows are the one
   deliberate exception the reference makes — flush to the frame edge, `▌` in
   cell 0, number in cell 3, label in cell 6.
2. **Bottom bar order.** `BottomBar.jsx` is blank / composer / blank /
   **status** / blank — the status line is *below* the composer, and the
   design system's own prose says so ("the composer is a three-row field with
   one quiet status line under it"). It had been above. While a decision is
   pending the panel *replaces* those rows rather than stacking above them
   ("the panel takes the composer's place"), which also relieved the
   long-running `clamp_panel` height pressure.
3. **Margins are two-sided.** `padding: 0 27px` is a margin on *both* sides;
   `filled_line` was only applying the left one, so panel prose wrapped three
   cells late, and `body_column_width` now holds a margin back on the right so
   code blocks and diff boxes stop short of the frame instead of running into
   it.
4. **Spacing measured, not guessed.** Six cells part *unrelated* groups (top
   bar name/cwd, footer hint groups); a single-spaced ` · ` separates facts
   *within* one group. The composer prompt is `▶` plus two spaces, putting the
   draft's first character in cell 6. A diff sign carries its own trailing
   space (`+ ` / `- `), coloured with the sign token, not the code.
5. **The diff box is inset.** Inside the permission card it rides the card's
   own `MARGIN_X`; inside a turn's body column it sits flush, since
   `CONTENT_INDENT` already positions it. The new `Inset` type makes which one
   applies explicit at each call site instead of hardcoding a margin into the
   box primitives.
6. **The transcript recedes at 35%** while a decision panel is open — the
   reference puts the whole conversation column at `opacity:.35` in both of
   its panel scenes. A terminal cell has no alpha, so `palette::fade`
   composites it and `fade_area` applies it as a post-pass over the drawn
   cells, each fading toward *its own* background so cards and diff bands
   recede against their own surfaces.
7. **One panel border, not two.** `draw` was drawing the bottom-bar edge and
   `panel_band` its own rule, stacking two lines where the reference has one
   `border-top: 1px solid var(--tui-modal-line)`. The shared row now carries
   that single rule, in `modal_line` when a panel is open.
8. **Code blocks follow the theme.** `CODE_SYNTAX_BG` was a fixed-dark
   constant because `highlight_lines` was pinned to syntect's
   `base16-ocean.dark` in both app themes — a black box in a light session,
   against the light palette. The highlighter now picks the matching half of
   the `base16-ocean` pair from `Palette::theme` (a new field, so anything
   holding a palette can ask), which frees the block to sit on `diff_box`,
   the design system's one nested-quote surface, in either theme. This closes
   the "no way to detect the terminal's background" question that comment
   deferred: the developer states it outright in `tui.yaml`.

Also hardened along the way: `card_footer_line` drops its right-hand
provenance token when both halves don't fit rather than wrapping a fragment
onto a row outside the panel (found by probing at 60 columns — the design is
drawn at 120, so nothing narrower had been checked).

Three regression tests pin the corrections that came from developer reports
directly: `the_status_line_sits_below_the_composer_not_above_it` (order, not
coordinates, so a height change can't silently flip it back),
`speaker_rows_sit_on_the_grids_label_and_body_columns`, and
`the_transcript_dims_while_a_decision_panel_is_open`. Whole workspace green
(143 in `aldwin-tui`), `cargo clippy -p aldwin-tui --all-targets` clean,
both themes re-screenshotted against the Chromium-rendered reference.

Lesson, again a process one: the token layer and the component prose were
both consumed correctly and still produced a wrong layout, because neither
states cell positions — those only exist as pixel values in the handoff
HTML's inline styles, and had to be divided by the cell size to be read at
all. Measuring the reference markup is a distinct step from reading it.

**Progress (2026-09-03, live-feedback batch — Ctrl+C, permission clarity,
mouse):** four items from a developer round of using the harness, three of
them defects and one a design question answered by the developer directly.

- **"Ctrl+C after /theme appears to be broken."** It was, and not only after
  `/theme`. `cancel_or_quit` decided whether a turn was running by scanning
  the log backwards for the most recent `UserMessage` ("running") or
  `TurnEnded` ("finished"). A slash command is submitted like any other
  message, so `submit` logs a `UserMessage` for it — but aldwin-cli's
  interceptor answers `/theme`, `/help`, `/reload-config` and any unknown
  command itself: the core never sees them, no turn starts, and no
  `TurnEnded` is ever appended. From the first such command onward the scan
  answered "a turn is running" to every Ctrl+C forever, so the key sent
  `Command::Cancel` into a session with nothing to cancel and the developer
  could never exit with it again. Replaced with the flags the events already
  maintain (`turn_active`, plus a new `awaiting_turn` covering the gap
  between submitting and `TurnStarted` landing, cleared by whatever comes
  back — a turn, or the `Notice`/`HistoryCleared`/`ThemeChanged` a locally
  handled command answers with). Added on top as a general escape hatch,
  since the failure class here is "trapped in the session": a second Ctrl+C
  within `DOUBLE_CTRL_C_TICKS` always exits regardless of what the state
  believes, with the first press saying so in the log.
- **"Permissions are not clear — are we approving the tool? the directory?
  what are we concretely doing?"** The panel named tiers ("Allow for this
  project") and nothing else: not what the rule would cover, not how long it
  lasts, not where it lands. Two additions answer it in the developer's own
  terms. `App::decision_grant` states the literal `kind:pattern` rule a
  saved answer adds — the same string that shows up in `permissions.yaml`,
  so "allow" on a shell prompt visibly means *this command*, not the shell
  tool — with the Tab scope toggle demoted to a second line under it (it
  used to be the only such line, and rendered for path-like targets only,
  which is exactly why a shell prompt explained nothing). `DecisionOption`
  gained a `detail` column saying what each answer does: "this call only;
  nothing is saved", "saved to .aldwin/permissions.yaml", "saved to
  ~/.aldwin/permissions.yaml". The panel footer's standing "saved to
  .aldwin/permissions.yaml" note is gone — it was true of exactly one tier
  on offer, an unconditional falsehood under every prompt.
- **"Do we need all of the deny options?"** No, and the developer chose the
  narrower list: four allow tiers and one `Deny` (tier `Once`), down from
  eight. `ToolTier` is untouched — the engine still supports deny at every
  tier — but a standing "never do this" rule belongs in `permissions.yaml`
  as a deliberate edit, not as options 6-8 of a prompt answered under time
  pressure. The list is also short enough now to read at a glance, which is
  most of what the previous item was about.
- **Text selection** — see the Out of Scope entry; mouse capture reverted.

Tests: the Ctrl+C fix is pinned by
`ctrl_c_still_quits_after_a_locally_handled_slash_command` (written against
`/theme`'s exact event sequence) plus the double-press pair; the panel by
`a_tool_prompt_states_the_rule_it_would_save_and_what_each_option_does`,
`the_panel_footer_makes_no_blanket_claim_about_where_answers_are_saved`, and
`the_option_detail_column_is_dropped_rather_than_wrapped_on_a_narrow_frame`
(the detail column is dropped wholesale below its fit width, never wrapped
per-row into a ladder). 150 in `aldwin-tui`, whole workspace green, clippy
clean on the touched crates. The panel was read back as a real render before
being called done, per this file's standing discipline — that pass is what
caught the rule line sitting flush against the near-identical raw-call line
above it, now parted by a padding row.


## Decisions

- **Conversation-first layout — full-width log, status bar, input bar at bottom.** — Keeps the conversation as the primary surface; state lives in the status bar rather than consuming persistent screen space. Split-pane rejected for V0 — adds complexity without payoff until the conversation log is proven sufficient. *Refined, not reversed, 2026-08-31:* an optional, secondary, width-gated sidebar was added for ambient state (permissions/tools/turn/messages) the header/footer couldn't fit — it is not the primary split-pane this decision rejected: it auto-collapses on narrow terminals and never takes the log panel below 80 columns when shown, so the conversation stays the primary surface either way.

- **Approval card is inline in the conversation log, not a full-screen overlay.** — Inline preserves conversational context during review. Visually distinguished via border + accent color so it cannot be mistaken for assistant output. Input blocked while pending — the developer cannot accidentally bypass the gate by typing ahead. *Refined, 2026-09-02:* per direct developer feedback that a pending card "in the chat" read as "ugly, not clear, disjointed," the *live* card moved out of the scrolling log into a fixed decision panel directly above the input — still not a full-screen overlay, and still visually distinct via the accent color, but no longer part of scrollback while pending (so it can no longer be scrolled out of view, which was the concrete complaint). A resolved decision still leaves the exact same full card inline in the log as a permanent record, unchanged from before — this refines where the *live* interaction happens, it doesn't reverse the "conversational context is preserved" rationale above, since the history is still right there in the log afterward.

- **Multi-line input; Enter submits, Shift+Enter inserts newline.** — Discussion-first posture benefits from longer prompts. Standard convention for multi-line TUI inputs. Single-line-only rejected as too restrictive for the intended interaction mode.

- **Minimal monochrome palette with one accent color in V0.** — Avoids colour decisions blocked on the open mascot palette. One accent is sufficient to make the approval card unmistakable. Rich theming deferred until the mascot palette is settled. *Extended, not reopened, across several 2026-08-29/08-31 Progress entries:* a small, cohesive set of semantic colors (diff add/remove, code, user tint, warning) was added incrementally, each scoped to one clear role — this is still a fixed, hardcoded palette, not the configurable/user-selectable "rich theming" this decision deferred; that remains blocked on the mascot palette question.

- **Thinking indicator shown; thinking content not shown.** — Content is dropped at source in aldwin-core per LlmClient contract. The indicator (ThinkingStart → dim spinner, ThinkingEnd → removed) gives awareness without log clutter.

- **Input blocked while an approval card is pending.** — Structural friction — the developer cannot queue submissions while an edit awaits approval. Consistent with "Edit is never allowlistable in any configuration" from the parent spec.

## Steps

1. Create crates/tui — Cargo.toml with ratatui, crossterm, tokio; depends on aldwin-core.

2. Define App struct: core event receiver, command sender, log snapshot, approval-pending flag, input buffer.
   - Why: Approval-pending flag drives input-blocking; keeping it on App avoids threading it through every handler.

3. Implement the event loop — multiplex crossterm keyboard events and core events onto a single tokio select.

4. Implement the three-band layout: conversation pane, status bar, input area.

5. Implement conversation log rendering — map each core event type to an entry shape; scrollable.

6. Implement tool-activity entries — ToolDispatched opens a dim entry; ToolCompleted closes it with result summary.

7. Implement the thinking indicator — ThinkingStart inserts a dim spinner entry; ThinkingEnd removes it.

8. Implement RetryAttempt rendering — visible inline entry with provider name, status code, message.

9. Implement the approval card — bordered accent-color inline entry, diff block, labeled approve/reject keys; block input while pending.
   - Verify: Typing ahead while a card is pending is silently dropped; card requires an unambiguous explicit keypress to resolve.

10. Implement the status bar — model name, T/S counter, read/shell/edit permission summary, active tool count.

11. Implement multi-line input textarea — Enter submits, Shift+Enter newlines, Ctrl+C cancels or exits.

12. Apply minimal palette — bright/normal/dim text hierarchy; accent on approval card border and focused-input highlight only.

## Pitfalls

- Blocking the ratatui draw loop on channel reads — use non-blocking poll or a short select timeout.
- Approval card dismissed by an accidental keypress — require an unambiguous labeled key ('y'/'n' or similar), not Enter. *Superseded, 2026-09-02, by explicit developer request:* the decision panel is now a numbered list with a visible `▸` cursor (see the same-day "numbered, arrow/digit-navigable list" Progress entry) — Enter is a valid confirm action again, since the visible cursor removes the original ambiguity ("what would Enter even do here") this Pitfall was guarding against. The underlying concern (no silent, invisible resolution) still holds; the mechanism satisfying it changed.
- Tool-activity entries flooding the log during parallel runs — group by step; collapse completed groups after a short delay.
- Shift+Enter behavior is terminal-dependent — test under kitty, iTerm2, and plain xterm; have a fallback binding.

## Out of Scope

- Mouse support — keyboard-only, and as of 2026-09-03 that includes the wheel. *Narrowed 2026-09-02, then reverted 2026-09-03:* mouse capture was briefly enabled so the wheel could scroll the log (`App::handle_mouse`), fixing a report that the wheel couldn't scroll at all. The developer's next round of feedback was the cost of that trade — "text selection has been disabled (or is simply not working)" — which is inherent, not a bug in the wiring: a terminal routes mouse events either to the application or to its own selection, never to both, so capture buys a wheel binding at the price of click-drag selection. For a harness whose premise is that the developer reads and reasons about the transcript, selecting and copying out of it wins; PageUp/PageDown/arrow scrolling already covers the wheel's job. Capture is off, `handle_mouse` is gone, and this line is back to "keyboard-only" without the carve-out.
- Rich/configurable color theming — a two-way `Theme::{Dark,Light}` choice exists now (2026-09-02, `tui.yaml`'s `theme` field — see the Palette bullet and its same-day Progress entry), each a small fixed semantic palette, but that's a binary switch between two hand-tuned sets, not a user-configurable/custom theming system (arbitrary colors, N themes, per-element overrides); that remains out of scope, still blocked on the mascot palette decision for anything beyond these two fixed options.
- Syntax highlighting in diff blocks — plain text diff in V0.
- Conversation log search or filtering.
- Split-pane layout as the *primary* layout element — still deferred. What exists as of 2026-08-31 is a secondary, optional, width-gated sidebar for ambient state, not a split-pane the developer's attention is meant to divide between — see the Conversation-first layout Decision entry above.
- Session persistence, conversation save/restore — out of V0 per parent spec.
- Web client rendering — V1.

**Progress (2026-09-03, ui module audit — structure, not appearance):** the
developer asked for an audit of the TUI implementation on the grounds that it
had become over-abstracted and hard to extend. It had, in a specific way:
`ui.rs` was 3,691 lines doing six unrelated jobs (frame layout, a text
wrapper, a markdown parser, a diff renderer, a filled-row primitive library,
and the decision panel's domain logic), and inside it eight near-identical
row builders — `filled_line`, `flush_line`, `boxed_line`, `card_line`,
`card_padding_line`, `card_rule`, `card_footer_line`, `diff_box_border` —
each re-derived the same wrap → measure → pad-to-width loop, each computing
its own available content width from the same formula. `diff_box_border` and
`boxed_line` had to independently agree on `width - 2*inset - 2` for a box's
top edge to line up with its own sides.

Changes:

- **`ui.rs` → `ui/`**, eight modules with one job each: `grid` (the cell
  constants and the `Ctx` render context), `wrap`, `row`, `markdown`, `diff`,
  `transcript`, `decision`, `chrome`. `ui/mod.rs` is 189 lines and is now the
  only file that touches band layout.
- **One row primitive.** `row::Row` is four numbers — margin, border, pad,
  fill — and every card row, diff row, option row, box edge and blank spacer
  is a constructor over it. The available content width (`Row::avail`) is
  computed once, so a box's edges and its sides can no longer disagree.
- **`Ctx` replaces the `(pal, width)` pair** threaded by hand through 25-odd
  signatures in inconsistent positions. Narrowing into a column is explicit
  (`ctx.body()`), which is where the two historical width-divergence bugs
  recorded above would have been visible at the call site.
- **`PromptView::of`** derives the panel's four facts about a `PromptPayload`
  (sentence, literal call, badge, whether the target is a command block) in
  one match instead of four scattered ones, so a new payload variant is one
  arm rather than a hunt.
- **Defect found by the new snapshot, and fixed:** `clamp_panel` trimmed the
  panel to fit by blind row count, so a diff too tall for its band was cut
  mid-box — a `┌───┐` on screen with no `└───┘` under it and nothing saying
  anything had been dropped. The approval card is now sized against the
  panel's budget *before* it is drawn (its head is measured, not assumed,
  since a wrapped path or sentence is more than one row), and overflow
  collapses into an in-box marker; a band too short for a box at all gets an
  honest one-line note instead of half a box. `clamp_panel` survives as a
  last resort for a prompt card's arbitrarily long target, which contains no
  box to cut.
- **Second defect, same source:** the elided-context marker inside a diff box
  set a foreground but no background, so its text sat on the frame ground —
  a visible hole across the middle of the box. `Row`'s "spans carry their own
  fill" contract makes that hard to get wrong now.

Verification was three passes, and the instrument is checked in.
`tests/render_snapshot.rs` renders 11 scenes × 4 frame sizes × 2 themes and
serializes every cell's symbol, fg, bg and modifiers to
`tests/snapshots/render.snap`. A baseline was captured *before* any
refactoring; afterwards the only frames that differed were the approval
scenes, and only in the two ways above — every other scene was byte-identical,
colours included. It also carries `no_frame_leaves_a_bordered_box_unclosed`,
which counts `┌` against `└` in the fixed panel and would have failed against
the pre-fix code. 150 unit tests and clippy stay clean; the visible design is
unchanged apart from the two fixes.


**Progress (2026-09-03, version reporting + snapshot determinism):** the
developer reported that the TUI's top bar showed `v0.1.0` on a v0.1.11
build, and that `aldwin --version` was wrong the same way. One root cause:
releases were tag-only. `Cargo.toml`'s workspace version sat at `0.1.0`
through eleven tagged releases, and both the top bar and clap's `version`
read `CARGO_PKG_VERSION`, so every build in that stretch reported `0.1.0`
truthfully — the manifest really did say that. Nothing could catch it,
because the manifest and the tag never met anywhere.

- `Cargo.toml` is now the source of truth and is bumped before tagging;
  `release.yml` fails the build when a `v*` tag disagrees with it, which is
  what stops the two drifting again.
- `version.rs` holds `VERSION`, `GIT_HASH` and `VERSION_FULL`
  (`0.1.12 (a1b2c3d4)`). `aldwin --version` prints the full form: on this
  harness most builds sit after the last tag, so the release number alone
  cannot tell two of them apart.
- `StatusInfo` gained `version`, `commit` and `cwd`, filled by `App::new`.
  The render layer no longer reaches for `env!` or `std::env::current_dir`
  mid-draw — `ui` is a pure function of `App` again, which is what lets a
  render test pin build identity instead of inheriting it.

A follow-up the v0.1.12 artifact itself exposed: it reported
`0.1.12 (3117724b-dirty)` despite being built from a clean CI checkout of
the tag. `build.rs` decided dirtiness with a bare `git status --porcelain`,
and this repo checks `target/` in (402 tracked files) — cargo has
necessarily written there before a build script runs, so every build looked
dirty, official releases included. The check is now scoped with
`:(top) :(top,exclude)target`, which is repo-root-relative (a build script's
cwd is its own crate) and ignores build output while still catching any real
source change.

That earlier point also fixes a defect in the snapshot harness added earlier
the same day: it inherited the real version, commit and working directory,
so `render.snap` encoded the identity of whoever generated it. It passed
only because committing does not touch `.git/HEAD` and so did not rerun
`build.rs`; `touch .git/HEAD` broke it immediately, and it would have broken
for any other checkout path or a dirty tree. Scenes now pin `0.0.0` /
`0badc0de` / `~/src/gateway`, and the file is stable across commits,
machines and dirty trees. Confirmed by regenerating, then forcing a
`build.rs` rerun and a dirty tree and re-running: no diff. The regenerated
snapshot's only change was the 96 identity rows; every other row was
untouched.

**Progress (2026-09-03, fourth developer round — surfaces, the resolved-
decision record, and a terminal-free screenshot loop):** seven reports, six
of them defects with a single shared root cause and one a palette question
left open at the bottom of this entry.

New tooling first, because it is what found the rest. `examples/snapshot.rs`
draws every scene into a `TestBackend` and writes each frame as a
self-contained HTML page — one styled run per cell run on the design
system's own 9×20px grid — which headless Chromium screenshots directly. No
tmux session, no real terminal, no interactive step, so a frame can be
rendered and *looked at* in one command. `examples/preview.rs` stays for
driving a real terminal; this is the comparison loop. `Color::Reset` renders
magenta on purpose: a cell the app never coloured is a hole in the palette,
and the point is to see holes rather than plausible-looking output.

The shared root cause: **a span with no explicit `bg` does not paint one**.
It keeps whatever the buffer already held, which under the decision panel is
the frame's `ground` — `ui::draw` fills the whole frame with it before
anything else. `Row` filled every span it built, so the contract held
wherever `Row` was in charge; three call sites handed spans in raw and
silently opted out. Measured against a Chromium render of our own frame:
the cell under the panel title was `#161826`, the frame ground, inside a
`#202d39` band. Reported as "the title 'permission' has a dark background."
In the light theme the same holes are white boxes around `permission` and
around every footer key hint. Fixed at the primitive — `Row::on_field`
paints the row's own fill onto any span that didn't ask for one, and a span
that *did* (a diff row's tint) is untouched — and again as a backstop, with
`decision::draw_panel` painting `bar` under the panel so a future leak lands
on the panel rather than through it.

The rest, each verified against the rendered reference frame rather than
against its markup alone:

- **Border rows floated in the frame background.** `edge()` painted its row
  `ground`, so between the top bar and its own `border-bottom` sat a strip
  of frame background, and likewise under the bottom bar's `border-top`: "the
  input field top border is sitting above the input field with some
  margin/gap, the same is for the very top session bar." In CSS the border is
  1px of a 61px band and touches its own bar; a terminal spends a whole row
  on it, so that row now carries the background of the surface it belongs to
  — `bar` above, `bar_bottom` below, `band` when the panel has taken those
  rows.
- **`CommandBlock.jsx` had lost its inset.** The reference wraps the field in
  the card's `padding: 0 27px` and gives the field its own `padding-left:
  18px`, so it reads as a quoted object with `bar` down both sides and the
  `$` on cell 5. Built on `Row::card` alone it ran the panel's full width:
  "not a box like in the design but instead completely fills the entire
  dialog edge-to-edge with no margin." `Row::pad` makes the second inset
  expressible; the block is now `Row::card(ground).inset(MARGIN_X, bar).pad(2)`.
- **The diff gutter was 11 cells against the grid's 5.** A two-column
  `old │ new` gutter pushed every line of code out of its column
  (`--gutter-line-no-inline: 45px`). A unified-diff row exists on exactly one
  side of the change, so there is only ever one number to show; context rows
  take the new-file number. In-box notes now indent to the code column past
  an empty gutter, as the reference's own `81 more lines` row does.
- **The diff box's borders took the row tint.** `with_fill` swapped the
  field the `│` sides were drawn on, so they went green or red — "the borders
  are not aligned with the background at all." `Row` now carries `field`
  (the box) separately from `bg` (the row), and only `bg` is swapped.
- **A resolved decision rendered as the panel's card, inline.** An answered
  prompt or edit left a full-width `bar`-filled block in the middle of the
  conversation, aligned to nothing around it — "misaligned and wonky" — with
  `format!("{response:?}")` underneath, so the log carried a line of Rust:
  `Tool { decision: Allow, tier: Once, pattern: "…" }`. A resolved decision
  is a tool call that happened, and the reference already renders one of
  those: `ToolLine.jsx` on the turn's own body column — glyph, tool name in a
  6-cell column, target, right-flush summary — with the diff box under it for
  an edit. `log::PromptResolution` replaces the debug string with
  `allowed` + a phrase written for a developer reading back over the session,
  built by `app::describe_response` from the `PromptResponse` rather than
  from the `DecisionOption`: Ctrl+C resolves through `decline_outcome`, which
  has no option to copy a label off, and that is the path a developer under
  time pressure is likeliest to take.
- **A code fence and a diff fence were different components.** Both are the
  system's one nested-quote surface (`--tui-diff-box`), but only the diff
  drew `InlineDiff.jsx`'s border, so the two read as unrelated treatments in
  the same reply. The code block is bordered now, from the same `Row::boxed`.
- Smaller, found while measuring: the panel badge was `dim` where the
  reference gives it `hunk_header` (`Row::split` hardcoded one colour for a
  right-hand slot with two different jobs); a diff header's `a/`/`b/` side
  marker leaked into the path shown as a target; `grid::elide` bounds the
  whole result to `max` cells, the `…` included, since every caller is a
  hand-composed row with exactly that many to spend.

Verified by re-rendering all six scenes in both themes and comparing against
a Chromium render of `Agent TUI v2.dc.html`'s own `4a` and `5a` frames, not
against the previous pass's output. `cargo test --workspace` green (155 in
`aldwin-tui`), `cargo clippy -p aldwin-tui --all-targets -D warnings`
clean. `render.snap` regenerated deliberately after eyeballing the frames.

**Progress (2026-09-03, follow-up — the border was still not on the bar's
edge):** the developer looked at the top bar in the screenshots above and
asked why the border was not aligned cleanly with the bottom of the
component. It wasn't, and the entry above had only half-fixed it: painting
the border row in the bar's own background closed the gap *above* the line
but not the one below it.

Two faults, measured by sampling a column of pixels down the rendered edge:

1. **`─` is the wrong glyph for a border.** It draws through the *middle* of
   its cell, so the row left half a cell of bar under the line: the bar band
   ran to y=99 with the line at y=89, ten pixels of bar below it. A CSS
   border is the last pixel of its band and touches its neighbour with
   nothing in between. The one-eighth blocks are the glyphs that do that —
   `▁` fills the bottom ~2.5px of its cell, `▔` the top — which at a 20px
   cell is about as close to the reference's 1px hairline as a terminal
   gets. Both are Block Elements (U+2580–U+259F), the same range as the `█`
   and `▌` the glyph table already mandates, so any terminal that can draw
   those can draw these. Measured after: bar to y=75, line at y=76–78,
   ground from y=80 — one stray pixel of bar at y=79, which is the font's
   own glyph placement rather than the layout.
2. **The border was costing a row.** `--bar-top-h: 60px` is 3 cells and
   `--bar-bottom-h: 101px` is 5; the extra `1px` in each *is* the border, so
   it belongs inside the band. Spending a separate row on it rendered the
   3-cell top bar as four and the 5-cell bottom bar as six. Both edges now
   live on a row the band already owns — the top bar's last row, the bottom
   bar's first (blank) row — which also hands two rows back to the
   transcript.

The decision panel is the one exception and keeps a row: its first row is
the title band, which carries text, so there is no spare cell edge to draw
`border-top: 1px solid var(--tui-modal-line)` against. It takes the row
above instead, drawn on `ground` so the accent hairline sits flush against
the top of the band with transcript above it.

**Resolved as an underline, after three glyph attempts.** The shape a
`border-bottom` actually has is not a character at all — it is a *cell
attribute*. `SGR 4` rules the bottom of the cell box at the font's own
hairline weight, which costs neither a glyph nor a row and lands exactly on
the band's edge. The three rejected attempts, each for a different reason:

| attempt | placement | weight | why not |
|---|---|---|---|
| `─` on its own row | mid-cell | full cell | floats; spends a row the grid doesn't have |
| `▁`/`▔` one-eighth blocks | correct edge | ~2.5px | rarely-exercised glyph range — "too risky to rely on glyphs" |
| `BorderType::QuadrantOutside` | correct edge | 10px | ratatui's own idiom and well-supported, but "the line is thick as hell" |
| **underline** | **correct edge** | **1px** | — |

Measured after: `bar` #232532 to y=78, one pixel of `line` #3f424d at y=79,
`ground` from y=80. Light theme the same, #e4e7f5 → #cfd3e5 → #f3f5fe.

Colour degrades cleanly in both directions, which is what makes this safe
where a glyph wasn't: `underline_color` carries the exact token on
terminals implementing `SGR 58` (kitty, VTE, WezTerm, iTerm2, mintty), and
where it isn't implemented (notably Alacritty) the underline is drawn in
the cell's own foreground — set to the same token here, so the rule is the
right colour either way. `SGR 4` itself is universal. The crate already
enabled ratatui's `underline-color` feature, so nothing new was needed.

The one limit is direction: an underline is always on the bottom of a cell
and ratatui has no overline modifier (`Modifier` stops at `CROSSED_OUT`).
A `border-top` therefore has to be the underline of the row *above* it,
which is why the bottom bar's first row and the panel's edge row are
painted `ground` rather than `bar_bottom` — they read as the transcript's
last row carrying the rule that starts the bar beneath. `bar_bottom` and
`ground` differ by a 1.05 contrast ratio, so nothing is visibly lost.

Inner boxes stay `BorderType::Plain`: the handoff README is explicit that
"the diff panel is a plain `Block::bordered()` with `BorderType::Plain`.
This was an explicit design decision after review."

Pinned by `the_top_bar_carries_its_border_as_an_underline_on_its_last_row`
and `the_bottom_bar_carries_its_border_as_an_underline_on_the_row_above_it`,
which assert the modifier, both colours, the surface, and that the
neighbouring band starts in the very next cell. `examples/snapshot.rs`
renders the underline attribute into its HTML too — without that the thing
under review is invisible in a screenshot.

**Open, and deliberately not decided here — the palette itself.** The
seventh report was "because the colors are wrong, everything is quite hard
to read, we must solidify the color palette for both dark and light modes."
Every hex in `palette.rs` was re-checked against `tokens/semantic.css` and
`tokens/palette.css` this session and matches verbatim, so this is not
drift — it is the imported values themselves. Measured (WCAG contrast, every
text role against every surface it actually renders on):

| | dark | light |
|---|---|---|
| `dim` (timestamps, tool summaries, option details, placeholder) | 2.3–2.7 | 3.5–4.0 |
| `glyph_done` / `glyph_pending` (`●` `○`, the transcript's status marks) | 2.3–2.7 | 2.0–2.7 |
| `label` / `context` | 3.3–4.1 | 3.5–4.0 |
| `mark` (the accent `▌` and composer `▶`) | 6.0–7.6 | 2.9–3.9 |
| `rule` on `bar` (the rule above the options list) | 1.07 | **1.00 — identical** |
| `bar_bottom` vs `diff_box` vs `ground` | 1.01–1.05 | **1.00 — identical** |

AA text is 4.5, AA large and non-text is 3.0. In the light theme
`quiet`/`label`/`context`/`dim` are one value across four roles, and
`bar`/`bar_bottom`/`diff_box` are one value across three, so the metadata
hierarchy and three of the five planes do not exist. Both themes draw a rule
on the panel that is invisible on it.

The obstacle is that the fix cannot come from the documented ramp alone:
four AA-passing metadata tiers on `#161826` would need steps between
`#9397ab` and `#cfd3e5`, and the neutral ramp has one; separating
`ground`/`bar_bottom`/`diff_box` needs values between `#161826` and
`#232532`, and it has none. So closing this means new steps, which is
inventing palette locally — the thing `.claude/CLAUDE.md`'s Design System
section exists to forbid. Left for the developer to direct, with the
measurements above as the case; whatever is chosen belongs in the design
system first and in `palette.rs` second, or the two drift.

**Progress (2026-09-03, colour-transport audit — the pipeline is exact, so
the palette is the finding):** the developer put a design frame and a
screenshot of `main` side by side and reported a stark colour difference,
suspecting "we have done something wrong with consuming the colors/applying
them," noting it was not the first time. It is not a consumption bug. The
whole path was measured end to end, and every layer this crate controls is
exact:

1. **Tokens → `palette.rs`.** All 33 `DARK` fields diffed mechanically
   against `tokens/palette.css` + `tokens/semantic.css`: 0 mismatches. (The
   two pre-blended `_bg` tints were re-derived too — `del_bg` is exact;
   `add_bg` is one unit off in R and B from a hand blend, within the
   browser-sampling note already on it.)
2. **`palette.rs` → render buffer.** `tests/snapshots/render.snap` carries
   every cell of 11 scenes × 4 sizes × 2 themes. Across all **315,040
   cells**: zero `Color::Reset` backgrounds, and zero named/ANSI (therefore
   terminal-remappable) colours anywhere. The surfaces in the buffer are
   `Rgb(22,24,38)` / `Rgb(35,37,50)` / `Rgb(27,29,43)` — the tokens verbatim.
3. **Buffer → the wire.** The real binary was run under a pty and its
   output parsed. Every colour is 24-bit truecolor SGR: `48;2;22;24;38`,
   `48;2;35;37;50`, `48;2;27;29;43`, foregrounds likewise. Nothing is
   downsampled by us, and ratatui/crossterm add no colour-depth logic. The
   only bare `ESC[39m ESC[49m ESC[59m ESC[0m` is ratatui's end-of-frame
   reset, after all cells are painted.
4. **Quantisation ruled out as the cause of what was reported.** If
   something downstream *were* collapsing to the 256-colour cube,
   `ground`/`bar_bottom`/`diff_box` would all land on index 234 `#1c1c1c`
   — one flat grey, indigo gone, the plane hierarchy destroyed. The
   screenshot still shows the indigo cast, so truecolor is arriving. (The
   check is still worth having: `tmux display -p '#{client_termfeatures}'`
   must contain `RGB`, and `COLORTERM` must be `truecolor`.)

**One real defect found and fixed.** The `, ` separating tool names in
`chrome::draw_status_line` was a bare `Span::raw`, i.e. `Style::default()`
— the *terminal's* default foreground, not a token. It was the single cell
in the entire corpus painting a visible glyph outside the palette's
control: near-black on a light-profile terminal, near-white on a dark one.
Now `label`. `chrome.rs`'s own `highlight_command_tokens` doc comment
already stated this rule for composer words; nothing enforced it, so it was
violated 90 lines above the comment.

Enforced now by `every_painted_cell_uses_a_palette_colour_never_the_terminals_own`,
which walks every cell of every scene at every size in both themes and
asserts two rules: no `Reset` background anywhere (a hole in the opaque
canvas), and no `Reset` foreground on a cell carrying a glyph. Whitespace
is exempt — `grid`/`row` build margins and gutters from bare `Span::raw`
and paint no ink, so constraining those would forbid a genuinely colourless
idiom. This is the durable answer to "not the first time": the class of bug
is now a test failure rather than a review catch.

**So the difference the developer is seeing is the palette itself — the
open item in the entry above, restated.** Those measurements were
re-derived independently this session and reproduce exactly:
`ground`/`bar_bottom`/`diff_box` sit at 1.01–1.05 contrast (three planes
that are one plane to the eye), `dim`/`glyph_done`/`glyph_pending` at
2.7 against a 3.0 non-text floor, `label`/`context` at 4.1 against a 4.5
text floor; in light, `bar`/`bar_bottom`/`diff_box` are literally one hex
and `quiet`/`label`/`context`/`dim` are literally one hex.

Two things make that read worse in a terminal than in the mock, and both
argue the fix has to be *more* separation than the browser needs, not the
same:

- A 1.05 plane step survives in a browser as a large flat antialiased
  region. In a terminal the same step is drawn per cell, under text
  antialiasing, at the display's own gamma — the boundary that reads as a
  soft plane edge in the mock reads as nothing.
- The mock is 15px JetBrains Mono at a 20px line box with browser
  rasterisation. A terminal uses the developer's font at their size,
  weight and hinting, so the *amount of ink* per glyph differs — which
  moves apparent lightness of every text tier independently of its hex.

Neither is fixable in `palette.rs`, and neither is the developer's terminal
being misconfigured. `scratchpad/color-probe.sh` (not checked in) prints
the environment, a truecolor round-trip gradient, the six surfaces as
swatches, and an OSC 11 readback, for confirming transport on any specific
terminal before touching palette again.

Still open, and still the developer's call for the same reason as before:
closing this needs neutral steps between `#9397ab` and `#cfd3e5` and ground
steps between `#161826` and `#232532`, which the documented ramp does not
have. That is a design-system change first, `palette.rs` second.

**Progress (2026-09-06, Turn 13 — borderless rebuild, new grid, generated
palette):** Closes the colour-transport gap above, and supersedes the
2026-09-03 grid entry. The design system was re-synced from the bound copy
in the "Design system tokens discussion" project (see `.claude/design/`),
which is the live token layer — the standalone design-system project is
stale and its `updatedAt` does not move when the bound copy is edited, so
neither the file list nor the timestamp there is evidence of currency.

Three decisions landed, all of them structural:

1. **Nothing inside a frame is stroked.** Every rule, pane divider and box
   outline is gone; a band's step on the new seven-rung ground ladder
   (`--color-ground-0…6`) is the boundary. That ended the long argument
   with the medium recorded in the earlier entries — `─` rows floating
   mid-cell, `▁`/`▔` glyph risk, `BorderType::QuadrantOutside` at half a
   cell, and finally `SGR 4` underlines with `underline_color`. All of it
   is deleted. A background colour is exact in a cell grid in a way a
   hairline never was, and the handoff says so directly: separators are "a
   single `Style::bg` on a one-row rect, so nothing here needs
   approximating". `ui/mod.rs`'s `hairline` helper, `Row`'s `bordered`/
   `field` machinery and `Row::border` are all gone; `row::rule_row` became
   `row::band_row`, and `Row::boxed` became `Row::field`.
2. **The grid moved**: label column 12 → 8 cells, body column 17 → 13.
   `--body-col` was deleted upstream on purpose — cell 13 is a consequence
   of margin + label + gutter, and `cells.css` carries a standing
   instruction not to restate it. `grid.rs` already derived `CONTENT_INDENT`
   that way, so only `LABEL_COL_WIDTH` changed.
3. **The palette is generated, not picked** — one hue (300°), lightness in
   even OKLCH steps, chroma falling as lightness rises. This is what closes
   the colour-transport gap: the missing neutral and ground steps the entry
   above was blocked on now exist by construction, so the eleven-field local
   deviation in `palette.rs` is deleted and every value is the token's own
   again. Seven roles were added (`recess`, `break_`, `panel_title`,
   `scrim`, `reverse_bg`/`reverse_ink`, and the `add_row`/`del_row` fills);
   `modal_line` was removed with its token, and `rule` with the concept.

Two bugs fell out of the port, both of which the design system had itself
found and fixed upstream in the same turn:

- The decision panel's title row was painted `band` — the *selection*
  colour, an accent fill. That is exactly the treatment the system rejected
  ("read as a filled accent band and broke the guide's rule"); its own
  bundle had the identical bug. It is `panel_title` now, a lift at the top
  of the ladder, which leaves the selection band the only accent fill in
  the frame besides the gauge. Its right-flush badge moved from the
  gauge-fill step (2.2:1 on that field) to the `you` step (3.8:1).
- A markdown `---` still rendered as a 20-cell run of `─`, a glyph that is
  not in the design system's closed vocabulary at all. It is a `break_`
  band now, the same treatment a turn break gets.

The diff-row alpha arithmetic is also gone: the system ships resolved solid
row fills (`--tui-add-row`/`--tui-del-row`) beside the rgba tints, so
nothing is hand-blended between the source and `palette.rs` any more. And
because a quoted diff has no edges left to lose, the whole "a clamped panel
must not leave a box unclosed" hazard is structurally absent rather than
defended against — `MIN_BOX_ROWS` dropped 4 → 2 and `boxed`'s budget no
longer reserves two rows for edges.

379 workspace tests pass; `render.snap` was regenerated and contains no
box-drawing glyph anywhere.


**Progress (2026-09-06, screen 5d — first run):** Built. `crates/tui/src/
first_run.rs` holds the state and its own terminal loop; `crates/tui/src/ui/
first_run.rs` draws it. It runs *before* the session TUI because the model
answer decides which LLM client `aldwin-cli`'s bootstrap constructs, so it
cannot be a mode inside `App`.

Triggered by either question being unanswered: no provider config resolves
(the model is unknown), or the project has no `.aldwin/permissions.yaml`
(this directory's access posture is undeclared). Only the unanswered steps
are shown, and the `n of m` counter reads off that list — entering a new
directory with a model already configured asks one question, not two.

Faithful to the design system's Brand mark and First run sections: a one-row
reverse-video wordmark (`  A L D W I N  `, accent as ground, desk as ink —
never the multi-row block that was built and cut), the positioning line in
`--tui-dim`, three blank rows between sections, the shared option row
(`▌`, two spaces, a 16-cell name field, then a purpose statement), and
`config → ~/.aldwin/` in the footer.

Three deliberate deviations, each because the design's own wording would have
been false here:

1. **Three access points, not four.** With editing de-scoped (ADR 0001) the
   `write` tier collapses into `read`, and a drafted `run` tier would have
   written identical grants. See ADR 0001 §5.
2. **No `/model` or `/access` in the prose.** The design's copy promises both
   commands; neither exists (`/clear /exit /help /nope /quit /theme`). The
   clause is dropped rather than shipped false.
3. **`1 of 2`, not `step 1 of 2`.** The label column is 8 cells since Turn 13
   and the longer form overflowed into the first option row. The reference
   wrote "step 2 of 3" when that column was 12 cells wide.
4. **`ask` is preselected on the access list.** The design system says
   "nothing is preselected on `access`. Every row shows an idle `▌`, which
   is how the frame says a decision is still open." Built that way first,
   and reported as a defect: with nothing selected `⏎` had to refuse to
   commit, so pressing it did nothing — most visibly on the access-only run,
   where `access` is the first step and the very first key press left the
   screen apparently frozen.

   The preselection is safe *because of which row it is*. `ask` grants
   nothing, so the default answer is the default-deny one and an accidental
   `⏎` widens no permission. A preselected `read` or `all` would be exactly
   the inertia the design system is guarding against and must not be
   introduced; a test pins the preselected tier to the one whose `grants()`
   is empty rather than to index 0, so reordering the scale cannot quietly
   change what enter agrees to.

Also corrected here: `bottom_height` still reserved a row for the panel edge
removed in the borderless pass, which put a stray `bar` row *below* the
footer. The panel now takes exactly its own rows.


**Progress (2026-09-06, grid conformance + scrollbar removed):** The
transcript's `Scrollbar` is gone. The design system lists scrollbars under
"Deliberately absent" beside tabs, breadcrumbs and "any control that needs a
mouse", and this one drew a `║` track down the frame's last column whenever
the log overflowed — the single element in the frame sitting outside the
3-cell right margin. The log still scrolls; only the drawn indicator is
gone, and nothing replaced it: the transcript is bottom-anchored, so the
live end is always on screen. `Palette::line` went with it, since the
scrollbar was its last consumer and `cells.css`'s own rule is "if a token
here is not applied through a `var()` somewhere, delete it rather than
document it".

Four conformance tests now sit beside the snapshot in
`tests/render_snapshot.rs`, run over all 11 scenes in both themes at the
design's 120×36 frame. The snapshot proves a render is *unchanged*; these
prove it is *correct*, which is the gap that let the label column stay at 12
cells for as long as it did — a wrong column is preserved as faithfully as a
right one.

They assert: both 3-cell margins (with the flush option row as the single
spelled-out exception); that a transcript turn puts its speaker on the
margin and its content on cell 13; that the top bar is three rows of one
tone with a *different* tone beneath it, and that no box-drawing glyph or
full row of underlined blanks appears anywhere; and that no scene draws a
scrollbar. All four measure the rendered buffer rather than reading the
constants back out of the code, and the grid values are restated in the test
rather than imported — a test that imports `MARGIN_X` can only ever agree
with it.

Verified load-bearing by mutation: setting `LABEL_COL_WIDTH` back to 12
fails the turn test with the actual rendered row in the message.

Two things this surfaced and did *not* fix, both recorded rather than
changed:

1. The permission panel's option rows still derive their name column from
   the longest label instead of the system's fixed 16-cell field, so the
   panel's list and first run's list are two geometries for what the design
   calls one control. The cause is upstream of the grid — Aldwin's option
   labels are sentences ("Allow for this session", 22 cells) that cannot fit
   16, and Aldwin added a detail column the reference's `5a` has no
   equivalent for. Fixing it means shortening permission copy.
2. Markdown headings render `BOLD | UNDERLINED`, where the design says one
   size and one weight throughout and that "hierarchy is color and
   position". This is LLM-authored prose rather than frame chrome, so it is
   arguably a different domain, but it is a deviation either way.

**Progress (2026-09-06, thinking flag outlives its turn):** Found while
wiring Proton's Lumo in as an OpenAI-compatible provider (see
`.claude/spec/archive/aldwin-llm.md`'s entry of the same date), not by a
TUI change. `App::thinking` was cleared only by `Event::ThinkingEnd` and
`HistoryCleared`, so a stream that died mid-thinking — transport error,
idle timeout, anything that ends a turn without the closing event — left
the spinner reading "thinking" indefinitely, describing work that had
stopped. `Event::TurnEnded` now clears it alongside `turn_active` and
`awaiting_turn`, which is where the invariant belongs: no turn running
means nothing is thinking. Pre-existing and provider-independent, but
`lumo-max` reasons on nearly every turn, which is what made it visible.
Covered by `a_turn_ending_mid_thinking_clears_the_flag` in `app.rs`.


**Progress (2026-09-06, screen 5d asks for a provider, not a model):** The
design's `5d` was re-fetched from the discussion project
(`25845063-…`, `Agent TUI v2.dc.html`) and had changed: the first step is
now `provider`, `access` has moved to `step 2/2`, and the `model` step is
gone from first run entirely. Rebuilt to match.

The move is the right one and worth stating: a model id means nothing until
you know whose catalogue it comes from, and the provider is the answer that
has to be settled before an `LlmClient` can be constructed at all. The model
follows from it — first run writes the chosen provider's default and
`/model` changes it once the session is running.

What the frame now carries, measured off the handoff HTML rather than read
off its prose:

- Each step is its name in the 8-cell label column, `step n/m` beneath it,
  and in the body column one row of prose, a blank row, then the option
  rows. The prose row is new; the two steps sit 3 blank rows apart as
  before (`--section-gap-h`).
- The provider list is 3 curated rows plus a `more` row carrying `the full
  provider list` and a `→` flush to the 3-cell right margin. `⏎` on `more`
  expands the list in place — it neither commits nor advances a step, since
  taking it is asking to see the rest of the question. The row then goes,
  having nothing left to reveal, and the selection lands on the first
  provider it uncovered rather than snapping back to the top.
- A selected row's purpose text moved from `--tui-quiet` to
  `--tui-accent-text`, which is what `5a`, `5c` and `5d` all show and what
  the old code was alone in not doing.
- The footer's key order is the reference's: `⏎ continue` then `↑↓ choose`.

Two earlier deviations are now closed, and one stays:

1. **`step n/m`, not `1 of 2`.** The 2026-09-06 entry above shortened it
   because `step 1 of 2` overflowed the 8-cell label column. The reference
   writes it with a slash — `step 1/2` is *exactly* 8 cells — so the
   reference's own wording fits and is used. A test measures the counter at
   cells 3–10 and asserts the 2-cell gutter behind it stays blank.
2. **`/model` in the prose is now true.** The clause was dropped twice for
   naming a command that did not exist; `/model [provider/]model` was built
   alongside this (see `.claude/spec/archive/aldwin-cli.md`).
3. **`/access` is still dropped.** The access step's design copy promises
   "/access changes it later" and there is no such command, so the sentence
   ends at "Which actions run without asking." A test asserts the string
   `/access` appears nowhere in the rendered frame — a promise the harness
   cannot keep is worse than a shorter sentence.

`ollama` is absent from the provider list, which the design shows as its
fourth curated row. It is the one row whose copy — "local models · no key" —
the harness cannot honour: `api_key_env` is required in `provider.yaml` and
the OpenAI-compatible client refuses to start when the variable it names is
unset, so the row would be an option that cannot open a session. See
`.claude/spec/archive/aldwin-llm.md`'s catalogue note.

Layering: this crate still does not know what an endpoint or a key variable
is. `ProviderChoice { id, purpose }` is the display half of a catalogue row,
handed in by aldwin-cli, and `Answers::provider` is the id handed back —
an `Option`, so the access-only run cannot overwrite a provider it never
asked about. `MODELS` and `ModelChoice` are gone from this crate; the
catalogue that replaced them is `aldwin_llm::PROVIDERS`.

`FirstRun::default()` is now `#[cfg(test)]` and builds a stand-in catalogue
(`alpha`…`foxtrot`, three curated) rather than the real one, so a test does
not fail every time a provider is added upstream. One test measures the
expanded screen against the 36-row frame, which is the tallest it ever
gets. 444 workspace tests pass; clippy clean.


**Progress (2026-09-06, audit of the provider round — the identity bar was
never on the grid):** An audit pass over `4388605` in the shape of the
2026-09-06 `5d` audit: measure the render against the re-fetched frame
rather than re-reading the code. It found one grid defect that had been in
*every* scene since the bar was written, and two logic defects in the
round's own new code.

**The identity bar's working directory sat on cell 16.** Both top bars —
`chrome::draw_top_bar` and first run's — put `--group-gap`'s six cells
between `aldwin` and the cwd. Cell 16 is a position no token in
`cells.css` names. The reference's own `4a`, `5a`, `5c` and `5d` bars all
put the directory *three* cells after the seven-letter name, which is cell
13 — the body column, the same cell a transcript turn's content starts on.
Six cells part two genuinely unrelated groups (`5b`'s `review changes` /
`3 files`, and the footer's key hints), and the identity bar is not that.

`HANDOFF.md`'s prose says the six-cell gap "survives only between the brand
and everything else", which is exactly the trap CLAUDE.md's design notes
warn about: the prose does not state positions, the frame does. The gap is
now `chrome::brand_pad()` — derived from `BRAND`'s own width against
`CONTENT_INDENT`, so cell 13 is never restated — and `BRAND` itself is
written once rather than as a literal in two files.

Pinned by `the_identity_bar_puts_the_working_directory_on_the_body_column`
in `tests/render_snapshot.rs`, over all 11 scenes in both themes, measuring
the rendered buffer. Verified load-bearing by mutation: hard-coding the pad
back to 6 fails it with "the cwd starts on cell 16, not the body column".
`render.snap` regenerated; the diff is row 1 of every scene and nothing
else.

**First run re-asked a question the directory had already answered.**
`FirstRun::new` took only `ask_provider`, and put `Step::Access` in the
list unconditionally — so losing `~/.aldwin/provider.yaml` in a project
that already had a `permissions.yaml` re-asked the access question. That is
worse than noise: `bootstrap` writes the answer with `Config::add_grant`,
which only ever *adds*, so an answer of `all` in that state would silently
widen an allow list the developer had already curated. The one direction a
default-deny harness must never move on its own.

`new` now takes `ask_access` too, and `Answers::access` is an
`Option<AccessTier>` — absent meaning "not asked, leave what is on disk
alone", the same contract `provider` already had. `bootstrap` writes grants
only when it is `Some`. Covered by
`the_provider_only_run_asks_one_question_and_names_no_access_tier`, and by
a test that a screen with nothing to ask still has a step rather than
panicking in `step()`.

Recorded, not fixed: at 52 cells the identity group and the status group
touch with no separating space (`~/src/gatewayclaude-sonnet-5`). Pre-
existing — they touched before this change too, three characters earlier —
and outside the design's own 120×36 frame, which is the only size the
reference specifies. 454 workspace tests pass; clippy clean over the
changed crates.


**Progress (2026-09-06, the model selector):** Reported directly: "The
model selector doesn't work at all, it doesn't appear in the onboarding and
when trying to do it via slash commands it says the model is already
selected when it isn't." Both halves were real, and the first had a cause
outside this crate.

*Onboarding.* `Store::init_global_if_empty` seeded `~/.aldwin/provider.yaml`
with `anthropic` / `claude-sonnet-5`, and it ran **before** the first-run
screen's own test for whether the question was open
(`config.global_provider().is_err()`, `bootstrap.rs`). The seed had always
already answered it, so the provider step had never once been shown to
anyone — every developer silently got the seeded default. Init no longer
writes that file: the other three global files have meaningful empty values
(no grants, no servers, no theme override) and state nothing on anyone's
behalf, while any `provider.yaml` names a host, a model and a key variable
nobody chose. `provider.yaml` is correspondingly dropped from init's
required-file check — a directory without one is an unanswered question,
not a half-deleted config dir.

*The model step.* First run asked for a provider and wrote that provider's
catalogue default, so a model was never chosen at all. There is now a
`model` step between `provider` and `access`, listing the chosen provider's
own models (`ProviderChoice` carries them; the caller still hands over
display halves only). `←` reopens the previous question, since the model
step is the provider step narrowed and a wrong provider must not mean
quitting the screen.

*Three sections in 36 rows.* The frame does not scroll, and three full
lists do not fit it — measured, not guessed: with the catalogue expanded
the body needs 31 of its 30 rows. So a step the developer has *passed*
collapses to the single row that answered it, and the model section is not
drawn at all until a provider is settled (its list is that provider's own).
Every step now fits with rows to spare, pinned by
`the_expanded_screen_still_fits_the_frame`, which asserts it on each step
rather than only the first.

*"Already selected".* The message is `slash.rs`'s `already on …`, reached
when the argument names the provider you are already on — `/model lumo` in
a project whose `provider.yaml` pins `lumo/lumo-max`. It was truthful about
the file and useless as an answer: a developer typing that is reaching for
a list. Bare `/model` now opens one. `App::submit` reads that one
submission rather than forwarding it and opens a two-stage picker
(providers, then that provider's models) in the bottom band, on the
permission panel's own shape — a risen title row, prose, the recessed rule,
numbered flush option rows, key hints — because that panel is the system's
one in-session modal and a second control invented here would read as a
different application. `option_rows` and the key-hint builder are now shared
by both.

The picker **answers by typing the command**: committing submits
`/model <provider>/<model>` exactly as if the developer had typed it, so
aldwin-cli's interceptor stays the only thing that decides which scope the
write lands in and what is reported. Per aldwin-cli.md the CLI owns the
dispatch table; a picker that wrote `provider.yaml` itself would be a second
implementation of `/model` in the frontend, free to disagree with the first.
It opens on the row the session is running on and marks it `· current`, and
a pending decision still outranks it in both key routing and the band.

Verified end to end against the real binary in a sandboxed `HOME`: a fresh
first run wrote `google` / `gemini-2.5-flash` with the right endpoint and
key variable from the three lists, and `/model` → ⏎ → ↓ → ⏎ in a project
pinned to Lumo rewrote that project's `provider.yaml` to `lumo-lite`,
endpoint and key variable intact.

Not fixed, and worth knowing: a *bare model id* is still written onto
whatever endpoint is configured, so `/model claude-opus-5` in a Lumo
project saves `openai-compatible` + Lumo's `base_url` + `claude-opus-5` and
reports success — a config that fails at the host on the next start. The
catalogue is a seed rather than a ceiling (`aldwin-llm`'s own note), so
refusing an unlisted id is not obviously right; the picker sidesteps it,
and the text form still does not.

**Progress (2026-09-06, the model switch actually switches):** Two reports,
one about each half of the same round above. "When switching models, I
notice the top and bottom bars are not reflected with the new model name",
and "when there is no `.aldwin` directory in the current path aldwin is
invoked from, the onboarding screen does not allow for a model to be
selected."

*The bars.* They were honest: `/model` persisted a choice it could not
apply, and both bars read `status.model_name`, a string fixed at startup.
The notice said "this session keeps …, restart to use it," which is a
frontend explaining a limitation of its own wiring rather than a limitation
of the problem. `Agent<C, D>` does own its client for the life of the
process — but it does not have to own the *same* client. It is now handed a
`ClientHandle` (aldwin-cli's `bootstrap`), an `Arc<RwLock<Arc<AnyLlmClient>>>`
whose `LlmClient::stream` resolves the inner client once, when a request
starts, and holds it for that request: a swap landing mid-turn cannot pull
the client out from under a stream already running, and the next turn picks
up the new one. Core is untouched by any of this — still generic over
`C: LlmClient`, still knowing nothing about providers.

`/model` therefore builds the new client *before* it writes anything
(`slash::ModelSwitch`), so a provider whose `api_key_env` is not exported
fails at the command instead of at the developer's next start, and leaves
neither the session nor `provider.yaml` moved. On success it sends
`Event::ModelChanged { provider, model }` — the same "a layer above core has
no other vehicle to reach the TUI" shape as `Notice` and `ThemeChanged` —
and `App` sets `status.model_name` and `current_provider` from it, so both
bars and the picker's `· current` row follow on the next draw. The session
model is now state the interceptor advances (`slash::Session`) rather than
a startup constant; the notice it prints is "now on …", with no restart to
promise.

*The onboarding.* The provider and model steps were gated on
`global_provider().is_err()`, so a developer who had ever configured a
provider anywhere never saw them again — and entering a new directory, the
one moment they are deciding what this project runs on, offered `access`
alone. Both steps are now always on the screen that opens, and they open on
what is already configured (`Configured` → `FirstRun::preselect`, which also
expands the catalogue when the configured row sits behind `more` — a
selection the developer cannot see would be worse than none). Confirming
costs three keystrokes and writes nothing: the answer is compared against
what supplies the setting, and only a *changed* one is written.

Where it is written changed with it. A true first run still writes global —
a project-scope file would leave every other directory unconfigured. But
once a global default exists, an answer given while onboarding a directory
is about that directory, and lands in its own `.aldwin/provider.yaml`
beside the `permissions.yaml` the same screen is already writing. Picking a
model for one project must not silently move the default everywhere.

Verified end to end against the real binary in a sandboxed `HOME`: a true
first run wrote the global file from the three lists; a second directory
showed the lists opened on that global answer, and `↑` on the model step
wrote *that project's* `provider.yaml` while the global one stayed put;
confirming all three rows unchanged left only `permissions.yaml` behind.
Then `/model claude-opus-5` in a running session repainted row 2 (the top
bar's model) and row 35 (the status line) to the new name in the same frame
as the notice — the pty capture only rewrote the cells that changed, which
is exactly the two places the old code could not reach.

**Progress (2026-09-06, audit of the model-switch round):** an audit of the
change above, asked for immediately after it. Four findings, all fixed here,
and the first three were latent damage rather than cosmetics.

*The stream was never proved.* Every turn reaches its client through
`ClientHandle::stream`, and nothing exercised it — the end-to-end check had
only ever run slash commands. The handle now holds `Arc<dyn LlmClient>`
rather than the concrete `AnyLlmClient`, which is both what core sees
through the trait and what lets a test put its own client in there. Three
tests followed: events stream through whatever client is in the handle, the
next request runs on the client that replaced it, and — the guarantee that
makes a live swap safe at all — a stream built before a swap and drained
after it still yields the *old* client's events, so a turn cannot change
model half way through.

*A chosen key variable was being overwritten.* First run rebuilt the whole
`provider.yaml` from the catalogue row, so a developer who exports their key
as `ANTHROPIC_KEY_WORK` and changed only the *model* would have had
`api_key_env` reset to the catalogue's default and their next start broken.
`first_run_provider_config` now carries `api_key_env` over when the answer
names the provider already configured, and takes the catalogue's only when
it names a different one — a different endpoint really does have a different
key. That is also what makes an unchanged answer compare byte-equal, so
confirming still writes nothing.

*An endpoint with no row could be pressed past.* A `provider.yaml` aimed at
a host the catalogue cannot name has no row on this screen: the lists would
open at the top, on a provider the developer is not using, and `⏎⏎⏎` would
move that project onto Anthropic — a decision by inertia, on the screen
built to prevent them. `asks_for_a_provider` holds the two steps back in
exactly that case (`access` alone, as before) and in no other. Confirmed
against the binary: a global `provider.yaml` on `http://localhost:8000/…`
drew `step 1/1` and left the project with `permissions.yaml` only.

*"Already on" could still be false.* The guard compared the argument against
the *file*, and file and session can disagree — `/reload-config` picks up a
hand-edited `provider.yaml` without rebuilding the client. Asking for what
the file already said would then report no change while the session ran
something else, which is the same sentence the developer complained about
in the round above. It now requires both the file and `slash::Session` to
agree before it reports no change; when they disagree it falls through and
swaps, which is what was asked for.

Re-verified against the binary after all four: `/model` on the model the
session is really running still says "already on"; the next `/model` swapped
and repainted both bars; `/model google/…` with `GOOGLE_API_KEY` unset
refused by name, kept the session, and wrote nothing.

**Progress (2026-09-07, Turn 14 — the light theme, the step spine, and
`14d`):** The design moved again, in three ways, and the token layer had
been left behind by all three. Both `.dc.html` frames were fetched and
diffed against each other with their hex values masked: **byte-identical
apart from the `:root{--t-*}` block and two headings**, which is what makes
the light theme a pure re-point of one set of roles rather than a second
design.

*The light palette was regenerated.* Two of the changes are corrections, not
adjustments, and both were defects this project shipped:

- **`--tui-bar` and `--tui-bar-bottom` were inverted.** The light top bar sat
  *lighter* than the composer — the reverse of the dark theme — so the two
  chrome bands read as swapped between themes.
- **The turn break rose above the light ground.** It was the one band that
  did, on the argument that a separator has to stay visible. It does not:
  `#ede9f6` is a full step below `#faf7ff`.

The light ladder is now monotonic, seven ordered rungs, and its narrowest
(1.127:1, ground→break) is *wider* than the dark ladder's own narrowest
(1.011:1, scrim→recess). `palette.rs` grew
`both_ground_ladders_are_strictly_ordered_and_have_no_repeated_rung` and
`the_top_bar_is_further_from_the_ground_than_the_composer_in_both_themes`,
because neither defect is visible to a rendering test — the frame still
draws, it just stops having an edge where it needs one. **The two ladders are
not the same sequence**, and reading the light one as "the dark list
reversed" is what produced both defects; `palette.rs`'s module doc now says
so in place.

New role `--tui-step-done` / `Palette::step_done`. It is `accent-700` in the
dark theme — identical to `glyph_done`, so it looks redundant — and the two
part in light because they recede in *opposite directions*: a finished tool
call goes lighter than the accent (`#a17adf`), a settled first-run step goes
darker (`#6941a1`). Pinned by a test from both sides, since the light pair
reads as a copy-paste slip and the dark pair as a pointless field.
`PANEL_TRANSCRIPT_OPACITY` also moved `.35` → `.45`.

*First run is a spine.* Turn 14 stopped paginating it: all three steps are on
screen from the start, one row each — glyph on the margin, name on cell 13,
content on **`--step-content-col`, cell 29** — in one of three states
(`●` settled / `▌` open / `○` pending). The `step n/m` counter is gone; the
glyphs are the progress. Two new derived grid constants, `STEP_MARK_COL`
(10) and `STEP_CONTENT_COL` (29), live in `grid.rs` beside `CONTENT_INDENT`
and are derived the same way, so moving the option name field moves the
content column with it.

*`14d` is new* — the returning/empty state, replacing the centred welcome
hero. Bottom-anchored, so the empty frame sits exactly where the first turn
will appear rather than jumping when one arrives. Wordmark, then `in` /
`provider` / `access` on the ordinary label column, then one line of prose.
The build's **version and commit came off this screen**; the version is still
on the top bar and still asserted there, and the commit's assertion was
retired with a note rather than silently dropped.

Three of the reference's own values are deliberately **not** rendered,
the same call `chrome::draw_top_bar` already makes about its context gauge
and session cost: no git branch (nothing tracks one), the three real
permission states instead of `14d`'s single tier word (a tier is what first
run *writes*, not what is stored — a hand-edited `permissions.yaml` need not
correspond to one), and no `more` row on the model list (`visible_models`
is already that provider's whole list, and a row that reveals nothing does
nothing). The `←` key stays bound but is no longer hinted, per the
reference's two-hint footer.

*Measured, then rendered, then re-rendered.* Every column was checked against
the frames' own inline styles before any Rust was written (the numbers are in
`.claude/design/HANDOFF.md`'s third first-run supersession note), and one
correction came out of it that a reading alone had got wrong for a whole
turn: **the wordmark is padded by two spaces at each end, not one** — a
17-cell field, not 15. `examples/snapshot.rs` now covers `empty` and the
three first-run steps in both themes, and its page background reads the
theme's own `--tui-scrim` — it was a fixed near-black, which put every
*light* frame on a dark desk, hiding the one surface a reviewer uses to
judge whether the light chrome bands step the right way. Three screenshot
rounds against headless Chromium, the last two byte-identical across 28
frames.

**Pushed upstream**, which earlier rounds did not do: `semantic.css`,
`palette.css` and `SYNC.md` were written back to the bound `_ds/` copy in
`25845063-…`, so the token layer states what the frames render. The frames
still read none of it — each restates its palette inline — so the two can
drift again with no error anywhere, and only a measurement catches it.

**Progress (2026-09-07, transcript padding + the scroll rewrite):** Two
reports from live use, one cosmetic and one not.

*"The chat doesn't have any top and bottom padding and it means the text
touches the top and bottom bars, the designs do not do this."* True: the
body band handed its entire inner rect to the log, so the first turn sat in
the row immediately under the identity bar and the last in the row
immediately above the composer band. `ui::LOG_PAD_ROWS` now holds one row of
the transcript's own ground back at each end — a blank row, per `cells.css`'s
"spacing inside a frame is blank rows, never padding", not an inset with a
tone of its own. The bottom bar already opens with a blank row, so the gap
under the last turn reads as two and the gap under the top bar as one, which
is what the reference frames do. `INTRO_ROWS` dropped 8 → 7 in the same
change: the welcome hero's eighth row was its own hand-rolled trailing gap
to the composer, and with the band spacing *every* transcript it became a
second blank row where `14d` has one.

*"Scrolling is very broken and unnatural. The performance is poor and
scrolling up and down is very difficult."* Three separate causes:

1. **The transcript was rendered three times per frame.** `build_lines` ran
   once for `Paragraph::line_count` (which re-wrapped everything to produce
   the row count) and again for `Paragraph::render` (which re-wrapped
   everything again and threw away every row above `offset`) — on top of the
   build itself, which re-parses every diff and re-highlights every code
   fence through syntect. At the spinner's 120ms redraw cadence, plus once
   per keystroke that moved the offset. Measured on a 2400-row session at
   120×36: **83.8 ms/frame**. The event loop could not keep up with held
   arrow keys, which is what "difficult" meant.

   The fix has two halves. The builders now emit rows that are *already one
   screen row each* — the four arms that were relying on the log
   `Paragraph`'s own wrapper (`RetryAttempt`, `TurnEnded`, `Error`, `Notice`,
   plus a slash-command echo) wrap to the body column themselves via
   `transcript::body_lines`, and the hero elides its `cwd` — so `draw_log`
   slices `rows[offset .. offset + height]` with no `Wrap` at all and the
   count is just `rows.len()`. And `App::transcript_rows` caches that `Vec`
   across frames, keyed on `(render_epoch, log.len(), width, height, theme)`,
   with `App::invalidate_transcript` called from `push`, `apply_event` and
   `resolve_decision`. Same session, same size: **0.47 ms/frame**, ~180×.

   This retires the discipline the 2026-08-29 scrolling-fix and
   wrapped-row-scroll-math entries below installed — "count with ratatui's
   own wrapper so the count and the render can't disagree". It was right
   given a second wrapper existed; deleting the second wrapper is better,
   because the equivalence is now structural rather than maintained. The
   obligation it replaces is on the builders: **an over-wide row is
   truncated now, not wrapped**, so every arm of `render_entry` must fit its
   own column. `a_transcript_row_is_a_screen_row_…` asserts exactly that,
   width by width.

   It also fixed a real defect in passing. A long `Notice` at 52 columns used
   to wrap to a continuation row flush against the frame's *left edge* — the
   `wrap.rs` failure — visible in the checked-in snapshot before this change
   and gone from it after.

2. **A disengaged offset was never clamped.** `line_down`/`page_down` clamp
   against the `max_offset` of the moment, but that maximum *shrinks* when
   the viewport grows — a wider terminal rewraps into fewer rows, a resolved
   permission panel hands its band back to the log — and nothing pulled the
   offset back down with it. The transcript scrolled off the top of its own
   viewport into blank ground, and the only way back was holding Up once per
   row of growth. `ScrollState::set_viewport_height` now clamps on the
   not-following path too. Clamping, not jumping: `following` stays off.

3. **The wheel did nothing.** Mouse capture stays off — the 2026 note below
   is right that capture costs native text selection outright, and for this
   harness that trade is not worth a wheel binding. But it was a false
   dilemma: `run.rs` now sends **DECSET 1007** (alternate scroll mode), and
   the terminal translates wheel notches into cursor-key presses on its own.
   The wheel reaches `App::handle_key` as ordinary `KeyCode::Up`/`Down`, so
   there is no second scroll path to keep in step with the first, and
   selection is untouched. Best-effort in both directions: a terminal that
   doesn't implement the private mode ignores it.

**Progress (2026-09-07, the render budget, measured end to end):** The entry
above was written from an in-process benchmark. Reported next: "CPU usage is
pinned to 100 when scrolling and even pinned around 30 when idle." That is a
different claim from the one that benchmark answered, and it needed a real
process to test, so a measurement rig was built and is worth recording:

- **A fake streaming provider** — a ~50-line OpenAI-compatible SSE endpoint
  that streams a realistic reply (long prose plus a fenced Rust block) token
  by token at a settable rate, with `provider.yaml`'s `base_url` pointed
  straight at it. This is the only way to exercise the streaming path at all
  without a real key, and streaming turned out to be where the cost was.
- **A pty harness** — `script` with its stdin held open by a fifo, so the
  binary sees a terminal and does not exit on EOF, driven by writing
  keystrokes into the fifo and sampling `utime+stime` from
  `/proc/<pid>/stat`.

Four ways this rig lied before it stopped, all worth recording:

- Without the fifo the session sees EOF on stdin and exits, so it measures
  0% no matter what.
- With no `provider.yaml` it measures the *first-run wizard*, not the
  session. First run has no spinner ticker and idles at 0% however badly the
  session behaves.
- `pkill -f <server>.py` matches the shell running it, so the launcher
  killed itself, and the "server" that answered afterwards was a corpse
  returning nothing. A stream that never streamed also reads as ~0%.
- **The sample window has to sit inside the stream.** At 60 tok/s the reply
  finished in ~2.4s but the window was 6s, so two thirds of every streaming
  figure was idle time averaged in. Dropping the rate to 25 tok/s makes the
  reply ~11s, and the window then lands wholly inside it. This
  under-reported every streaming number by about 2.5x — in the same
  direction for every build, so the comparisons held, but the absolute
  figures did not.

Measured on the build before the entry above, 8 turns at 120x40: **82.7%
idle, 89.2% scrolling, 100.8% streaming.** That is the report, reproduced —
and both of the developer's figures are *conservative*, since they grow with
the transcript and this was a short one. The transcript cache from the entry
above had already taken idle and scrolling to 0.7% / 4.3%, but streaming
stayed pinned and rose with the session (58% at four turns, 95% at eight).
Three findings, in the order they were confirmed:

1. **A streamed token invalidated the whole conversation.** `apply_event`
   bumped one epoch for any event, so each `TextDelta` -- one per token --
   threw away every rendered row in the session and rebuilt all of them,
   re-parsing every diff and re-highlighting every fence. Isolating the draw
   from the rebuild in-process: 2.9 / 5.2 / 10.1 ms of rebuild per token at
   93 / 189 / 381 rows, against a flat 0.45 ms of draw. Linear in the
   session, which is what "pinned" meant.

   `ui::Transcript` now keeps rows **per log entry**, beside a copy of the
   entry they were built from, and re-renders only entries whose value
   changed. The key is the entry compared with `==`, not a fingerprint: a
   fingerprint is a second statement of what "changed" means and can
   disagree with the first, and `String`'s comparison short-circuits on
   length, which is exactly the streaming case. Rebuild cost went flat --
   0.76 / 0.98 / 1.02 ms at 189 / 381 / 957 rows.

   It also retires the invalidation flag entirely. Nothing declares "the
   transcript changed" any more; `sync` asks the data. That was the right
   trade twice over -- the flag had to be set from `push`, `apply_event`
   *and* `resolve_decision`, and a fourth mutation site added later would
   have shown a stale conversation with nothing to catch it.

2. **One `terminal.draw` per event.** At 60 tokens/second that is 60 frames
   a second of full-frame work to paint a difference no one can see; on a
   fast model it is several hundred. `run_loop` now coalesces on a 16ms
   floor (`MIN_FRAME`), with a `sleep_until` select arm so a burst that goes
   quiet inside the window still paints promptly, and a keystroke -- which
   never arrives at 60Hz -- still draws immediately. The spinner interval
   also moved to `MissedTickBehavior::Delay`; under `Burst` a slow frame
   made tokio replay every missed tick back to back, so falling behind
   produced a run of catch-up frames with nothing new in them.

3. **Closed code fences were re-highlighted forever.** Once 1 and 2 landed,
   the entry still being streamed into was the only thing re-rendered -- but
   it was re-rendered whole, so every fence that reply had already *closed*
   went back through syntect on every frame: 0.28 ms per fence per frame,
   about half of what a streaming frame still cost. `highlight_lines` is now
   memoised on exactly its three arguments (it is a pure function of them),
   which took a fence from 0.28 ms to 0.017 ms.

**Result.** Both builds on the corrected rig — 6 turns, 25 tok/s, the sample
window wholly inside the stream:

| | before | after |
|---|---|---|
| streaming a reply | 75.2% | **5.5%** |
| idle | 16.4% | **0.6%** |
| scrolling | 86.2% | **2.8%** |

And, which is the point of finding 1, it no longer scales: the same figures
hold at sixteen turns as at eight, where before they rose with every turn.

Both new invariants are pinned by tests that were checked against a
deliberately reintroduced bug before being trusted:
`an_incrementally_synced_transcript_equals_one_built_from_scratch` walks
every mutation shape the session performs (a streamed append, a push, a
resolution back-filled in place, a tool completing, a shortened log) and
compares against a second `App` built from the same log -- it fails the
moment the value is dropped from the cache key; and
`a_streamed_delta_re_renders_one_entry_not_the_whole_transcript` times an
append on a 40-turn transcript against one on a 2-turn transcript, which
goes to an 11x ratio the moment invalidation goes back to whole-log.

**What is deliberately still re-rendered:** the one entry currently being
streamed into, at the frame rate rather than per token. It is the only entry
in the log whose value is still changing, so it is the only one that cannot
be rendered once and kept. Asking "did anything else change?" over a 40-turn
transcript measures 0.001 ms.

**Progress (2026-09-07, the event loop, the frame, and a real composer):**
Two reports. "Scrolling still doesn't feel native — slow/laggy/jittery",
and "the input field is not multi-line: shift+enter does nothing, and a
multi-line paste sends the first sentence as a command."

The first was *not* the renderer this time. Measured in-process against
`TestBackend` at 120x36 — **not** the pty/CPU rig of the entry above, which
answers a different question and is the more expensive instrument; this one
only had to price a frame. At 80 and at 600 log entries, release build:
**0.39 ms/frame idle, 0.47 ms scrolling, 0.67 ms streaming, flat in the size
of the transcript.** The 16 ms frame budget was never close to full.
Everything between the developer's hand and that draw was the problem, in
three places:

1. **Input was starved behind the stream.** The `select!` was `biased` with
   `events.recv()` first, so while a reply streamed — one `TextDelta` per
   token, hundreds a second — that branch was permanently ready and the
   keyboard was never polled until the model stopped talking. Scrolling
   during a reply was not slow; it was queued. Input is polled first now,
   which cannot starve the other way round: no one types fast enough to
   hold the loop.
2. **A burst was replayed one frame at a time.** The loop took a single
   event per iteration before considering a draw. A wheel flick is ~30
   alternate-scroll cursor keys arriving at once, so the transcript walked
   through thirty scroll positions in sequence and kept sliding long after
   the developer stopped. Both queues are now drained into the *same*
   frame — each bounded, so a producer that never goes quiet can't hold the
   loop past a frame — so a flick lands where it was aimed.
3. **A frame was delivered in ~30 pieces.** `CrosstermBackend::new(io::
   stdout())` writes through a `LineWriter` with a ~1KB buffer, and ratatui
   emits no newlines; a full repaint (which every scroll step is) therefore
   left in about thirty separate writes, and a terminal composites whatever
   has arrived when its own refresh comes round. That is the tearing behind
   "jittery". The backend now wraps a 1 MiB `BufWriter`, and each frame is
   additionally bracketed in **DECSET 2026** (synchronized output) with the
   cursor hidden across the paint — ratatui writes the whole frame *before*
   placing the cursor, so without this the caret was dragged visibly across
   every row on the way past.

Two scroll behaviours changed with them: the transcript is bottom-anchored
like `14d`'s body band and the welcome hero already were (short
conversations sat glued under the identity bar, then jumped down to the
composer the moment they outgrew the band), and paging keeps two rows of
overlap instead of swapping the screen for an entirely unfamiliar one.
`App::handle_mouse` handles wheel events too, at the same three rows a
notch a terminal's own alternate-scroll translation sends — capture stays
off (selection is worth more than a wheel binding, per the 2026-09-0x
report) but a session that inherits reporting from a previous program no
longer silently drops the wheel.

The composer is now a real multi-line editor, in `draft.rs`. Three separate
defects were behind one report:

- **No bracketed paste.** The terminal sent a paste as if it had been
  *typed*, so every newline in it was a `KeyCode::Enter` and submitted the
  line above it. `EnableBracketedPaste` plus a `CtEvent::Paste` arm fixes
  it; `draft::sanitize` normalizes CRLF, expands tabs (the one place the
  harness alters what was pasted — a tab advances the *terminal's* cursor
  to a stop no cell-grid layout can predict) and drops every other control
  character, so a pasted escape sequence cannot reach the terminal.
- **Shift+Enter was unreachable, not unbound.** It was bound all along, but
  a terminal only reports it distinctly under the Kitty keyboard protocol;
  without that it is literally the same bytes as Enter. `run.rs` now asks
  for `DISAMBIGUATE_ESCAPE_CODES` where `supports_keyboard_enhancement()`
  says the terminal has it (bounded at 2s on a terminal that answers
  neither that query nor DA1 — in practice immediate), and Alt+Enter joins
  Ctrl+J as a fallback for the rest. This closes the "couldn't be verified
  against real terminals" gap the crate doc carried since 2026-08-29: the
  answer was that reasoning about it was never going to be enough, because
  the key does not exist on the wire unless you ask for it.
- **The draft was measured by newlines and drawn by `Paragraph`'s `Wrap`.**
  Two different ideas of where the rows were: the band was sized from
  newlines (so a pasted paragraph got one row and everything past it was
  clipped) and the caret was placed from the *source* line and column (so
  past the first wrap it drifted a row further away with each one).
  `draft::Layout` is now the single wrapping, shared by the band's height,
  the rows drawn, the caret, and Up/Down — which move by visual row, not by
  source line. The composer caps at 10 rows and scrolls the draft under it
  rather than eating the frame, and `▶  ` became a *gutter* reserved on
  every row rather than a prefix on the first, so a wrapped draft keeps one
  left edge.

Verified under a real pty (`script`, stdin scripted): a three-line
bracketed paste followed by Enter produces exactly one
`Command::Submit { text: "first line\nsecond line\nthird" }`, and the
alternate screen, bracketed paste, alternate scroll and synchronized-update
modes are all set on entry and cleared on exit.

**What this deliberately did not touch:** first run (`first_run::run`) keeps
its own terminal setup, and gets none of the above — no buffered writer, no
synchronized output, no bracketed paste, no keyboard enhancement. It has no
transcript, no scrolling, no streaming and no text field (its keys are
digits and arrows), so every one of those buys nothing there; the only thing
it forgoes is tearing on a repaint that happens at human keypress rate. If
it ever grows a text field, it needs this list.

**Progress (2026-09-07, audit of the round above):** three passes over the
change — correctness, Rust idiom, integration/spec. Probes for the first
pass (tiny frames down to 1x1, an 868k-character paste, wide/emoji
graphemes, every cursor position at three widths, every arrow key at every
cursor position in seven drafts) found no panic and no caret outside its
band. Three real defects came out of it, all now fixed and two of them
mine:

1. **The multi-line composer re-rendered the whole transcript on ordinary
   typing.** `Transcript`'s cache was keyed on the log band's height as
   well as its width. The band is resized by the composer growing — which
   before this round meant an explicit newline, and after it means *any*
   keystroke that pushes the draft across a wrap column. Measured at
   **7.2x the steady-state frame cost on a 40-turn transcript and rising
   with the session**, which is precisely the failure the 2026-09-07 entry
   above was written about, reintroduced through a different door. Height
   is not an input to `block_rows` (no arm of `render_entry` reads it), so
   it simply came out of the key: 0.290s → 0.042s over 100 frames at 40
   turns, now identical to the 2-turn figure. Pinned by
   `a_growing_composer_re_renders_nothing_in_the_transcript`, shaped like
   its streaming neighbour.
2. **The draft was wrapped twice per frame, from two independently-derived
   widths.** `ui::draw` measured it to size the band and `draw_input`
   measured it again to draw it, agreeing only because the bottom band
   happens to span the frame — the exact divergence `log_inner`'s own
   comment says must never be reintroduced. Now one `chrome::Composer`,
   built only when the composer is actually on screen, passed to
   `draw_input`. A 1000-line pasted draft went 1.13 → 0.61 ms/frame
   against a 0.56 ms empty-composer baseline; 20000 lines, 3.97 → 2.43 ms.
3. **A test assertion that could never fail.** The composer-cap test
   checked `!buffer_as_one_string.contains("line-0\n")` — the joined
   buffer has no row breaks in it, so that answered nothing. Rewritten to
   count rows carrying each needle.

The idiom pass took two things from the project's Rust guidelines:
`draft::sanitize` returns `Cow<str>` and borrows an ordinary paste back
untouched rather than copying a whole clipboard to produce the same bytes
(868k chars: 28 → 1.3 ms), with the fast-path predicate and the rewrite
sharing one statement of the rule so they cannot drift; and
`draft::source_line` walks the draft instead of collecting a `Vec<char>` of
it on every Home/End. It also caught `rows as u16` being cast before its
clamp rather than after — harmless today, since the truncated value happens
to land back in range, but wrong on its own terms.

**Progress (2026-09-07, Turn 15 — the light theme rebuilt, shallower):** The
design system moved again and this crate was behind. Only the light half
changed; the dark snapshot is byte-identical before and after.

The upstream change is *depth*, not hue. Turn 14's light theme spanned 19:1
from `#0e0c12` ink to a `#faf7ff` ground with a `#a39fac` desk — nothing in it
failed a contrast floor, it simply read as harsh beside the dark theme, which
separates bands by a step and lets the ends stay soft. It now runs `#241f2b` to
`#f7f5fa` with the seven ground rungs inside 10% of each other. **That is the
decision, not an oversight**: hierarchy is carried by the step between rungs,
so darkening a value here to "add contrast" undoes it. The narrowest light rung
(1.037:1, bar to break) is still wider than the dark one's (1.011:1, scrim to
recess), and `dim` holds 5.14:1 on the recessed field, the darkest band inside
a frame and the binding one there.

Two structural consequences, both of which broke a test that was pinning the
old arrangement, which is what those tests are for:

1. **The light ground rungs are renumbered by lightness**, so the ladder is now
   `ground`, `bar_bottom`, `bar`, `break_`, `recess`, `panel_title`, `scrim`.
   The turn break was the second-lightest band and is now the fourth, below
   both chrome bands. `both_ground_ladders_are_strictly_ordered_and_have_no_repeated_rung`
   was re-ordered to match; the invariant it asserts is unchanged.
2. **`step_done` now sits *below* the mark, not above it.** The design system
   has said since Turn 14 that a settled first-run step recedes by going darker
   than the accent mark while a finished tool call recedes by going lighter —
   but Turn 14's values put both above it, so the prose and the palette
   disagreed. Turn 15's `--color-accent-light-*` ramp fixed the values, and
   `the_two_done_glyph_roles_coincide_in_dark_and_straddle_the_mark_in_light`
   (renamed) now asserts that the two straddle the mark.

Upstream also stopped writing light values as literals: `.tui-light` is
thirty-eight `var()` references into new `--color-{ground,ink,accent,neutral,diff}-light-*`
ramps, so every `LIGHT` field in `palette.rs` names its rung the way the `DARK`
fields already did.

Verified the way this project's `CLAUDE.md` requires rather than by reading
prose: `Agent TUI v2 Light.dc.html` was fetched and its `:root{--t-*}` block
measured against the bound copy's `.tui-light`. All thirty-eight roles agree,
which is the one direction nothing else checks — the frames restate their whole
palette inline and read no token file.

**The syntax ramp, closed in the same pass.** Turn 15 also added five syntax
roles (`--tui-syn-keyword|call|type|string|number` in both themes) with two
rules: no syntax role may outrank the accent mark, and there are exactly five —
everything else in a code block stays `--tui-code` and a comment drops to
`--tui-dim`. `highlight.rs` was loading syntect's bundled `base16-ocean` pair,
which made a fenced block the one region of the frame carrying hues the design
system never chose. Nothing caught it: `every_painted_cell_uses_a_palette_colour_never_the_terminals_own`
only rules out `Color::Reset`, and a syntect `Rgb` passes.

syntect is kept for parsing and the *theme is now built from the palette*
rather than loaded. A syntect `Theme` is only a default style plus a list of
scope-selector → style rules, which is exactly the mapping five roles need, so
the scope matcher, the caching highlighter and the specificity scoring all
still come from syntect while no colour can enter a frame that `palette.rs` did
not put there.

The scope table was written against scopes dumped from the grammars, not
guessed, and three of its rows are only right for that reason:

- **`storage.type` is a keyword, not a type.** Rust's `let` and its `u32` are
  *both* `storage.type.rust` — the grammar does not distinguish them, so no
  selector can. Named types still land on `syn_type`, via `entity.name.*` and
  `support.type`.
- **`keyword.operator` is demoted to `--tui-code`.** An operator is
  punctuation the grammars file under `keyword`, and punctuation is not a
  category. syntect scores the more specific selector higher, so this beats the
  bare `keyword` rule without the table's order mattering.
- **A macro name is a call.** `format!` is `support.macro`, a name being
  invoked.

Two tests, deliberately a pair: `every_highlighted_colour_is_one_of_the_seven_roles`
(five languages × both themes, no eighth colour reachable) and
`each_syntax_role_claims_the_tokens_it_names`. The second exists because the
first would happily accept a selector edit that silently stops matching and
renders the whole block flat in `--tui-code`.

Snapshot effect: code-fence rows only, in both themes; every colour in a
changed row is now a token value, with no base16 left anywhere.

**Progress (2026-09-07, audit of the round above):** three passes —
correctness, Rust idiom, integration/spec. The correctness pass was run as
checks rather than by reading, because the thing being checked is a table of
120 hex values and reading a table of hex values proves nothing:

- **`palette.rs` against the token layer**, resolving `semantic.css` through
  `palette.css`'s `var()` chains and parsing both `Palette` consts out of the
  Rust: all 42 roles × 2 themes agree. The three roles with no Rust field
  (`--tui-line`, `--tui-add-bg`, `--tui-del-bg`) are the documented deliberate
  omissions.
- **`.tui-light` against the light frame's `--t-*` block**: all 35 keys agree.
- **The render snapshot against the palette**: every colour in both halves is
  either a palette value or a 45% `fade()` blend of two — 32 distinct in dark,
  31 in light, none foreign. That is the check that proves no syntect colour
  survived the highlighter rewrite.

One real defect, one that is not ours, and four smaller misses.

**Not ours, and still open: `--tui-reverse-bg` in `.tui-light` disagrees with
the frame.** The light frame paints the wordmark
`background: var(--t-mark)` = `#6b3fb0`; `.tui-light` sets
`--tui-reverse-bg: var(--color-accent-light-800)` = `#4d2a80`. The design
system's own README names `reverse-bg` = `mark` as one of five pairings that
"must hold in every theme, enforced by sharing one palette entry", and records
that the rule exists *because* this token once drifted from the mark and the
wordmark silently kept an old value. Turn 15 drifted it again — through ramps
this time rather than literals. `first_run.rs:288` therefore draws the light
wordmark one rung too dark, on four snapshot cells. `palette.rs` is faithful to
the token layer, which is what makes the disagreement visible at all, so it is
**deliberately not patched locally**: a local deviation would destroy the
one-to-one property the correctness check above depends on. The fix is one line
in the design system's `.tui-light`, in both projects, and the pairing check
belongs in `palette.rs`'s tests once it lands.

Found and fixed:

1. **`examples/snapshot.rs` had the light desk as a hex literal**, `#a39fac` —
   the Turn 14 scrim, two turns stale — under a comment claiming it was "the
   same value `Palette::scrim` carries". It was not, and nothing could catch
   it, because a literal agrees with itself. `scrim` is the one role a real
   terminal has no use for, so the HTML fixtures are its only consumer and it
   was unreachable from an example; there is now a `__preview_scrim_hex` beside
   the other `__preview*` fixture exports, and the fixture reads the palette.
2. **`ui/transcript.rs` still described the code-block surface as taking its
   colours from "the matching half of the `base16-ocean` pair".**
3. `syn_color`'s doc claimed its unreachable fallback was `Palette::code`'s
   value; it was `DARK.code`'s, which would be wrong in a light session. The
   fallback is now a plain grey and the comment says why a palette value would
   be the wrong thing to reach for there.
4. Idiom: the closure inside `fn theme(theme: Theme)` shadowed the parameter it
   was being selected by; `ScopeSelectors::from_str(s)` became `s.parse()` and
   the `FromStr` import went with it.

Also checked and clean: no clippy warning, default or `pedantic`, lands on a
line this round added — the two that remain are pre-existing. Every `base16`
mention left in this spec sits inside a dated Progress entry, which is history
rather than statement; none is in Decisions, Steps, Pitfalls or References.
`syn_type` is never exercised by the render snapshot and `syn_string` only
appears to be, because dark `syn_string` and `add_code` are the same `#9ceaa7`
by design — both roles are covered by `highlight.rs`'s own tests, which is the
right level for them.

**Progress (2026-09-07, markdown tables):** *Design half superseded
2026-09-08 — the table is now drawn; see the next entry. The parsing,
measuring, fitting and streaming behaviour described here all still stand.*
Reported: the chat renders every
other markdown construct but tables come through as literal `|` pipes and a
`---|---|---` delimiter row. Added to `ui/markdown.rs`, and the shape of the
fix matters more than the feature: a table is the first construct here whose
layout is **not** a per-line property — a column is only as wide as the widest
cell anywhere in the block — so the module's entry point moved from
`render_line` to a new block-level `render_prose`, which groups a table's rows
and hands every other line to the per-line path unchanged.
`transcript::render_assistant_text`'s `Prose` arm now passes the whole segment
and no longer wraps, because `render_prose` owns that (the one-wrap discipline
is unchanged; it moved, it did not double).

Two design calls, both made against the Turn 13 rules rather than against what
a terminal table usually looks like:

1. **Nothing is stroked, so there is no grid.** `─`/`│` are not in the closed
   glyph vocabulary (`▌ ● ◐ ○ ✔ ▶ █ + -`) at all, and the rule is explicit
   that a mark outside that table is not to be drawn. A table is therefore a
   header row toned `label` (the system's field-name tier, as in the hero's own
   `in`/`provider`/`access` rows), one `break_` band under it, and the rows on
   the panel ground in `body`. Column *position* carries the whole shape.
2. **That band is the table's width, not the body column's.** `band_row` fills
   its `Ctx`, which is right for a turn break and for a markdown `---` — both
   separate a section from what follows it — and wrong here, where the band
   separates *this table's* header from *this table's* rows. A 40-cell band
   under a 41-cell table reads as part of the table; a full-width one reads as
   a section break sitting inside one.

Widths are measured on the *rendered* cells, after the inline pass, so a
`**bold**` cell is not four cells wider than what it draws. Over-wide tables
shrink widest-column-first (a `yes`/`no` or count column keeps its text while
the prose column gives up cells) and then elide with the system's `…`; the
assembled row is truncated to the column as a backstop, so `Transcript`'s
"a built row is a screen row" invariant holds for a table too — verified at 56
columns as well as 100. Column alignment (`:--`, `:-:`, `--:`) is honoured for
the header row as well as the body, per GFM.

Detection is GFM's: a header row alone is not a table, the delimiter row
underneath it is what commits, and the delimiter's cell count must match the
header's or the block falls back to prose. That is also what makes it correct
mid-stream — a half-arrived table renders as prose until the delimiter lands
and snaps into columns on the next delta, the same posture
`split_code_fences` takes toward an unterminated fence. Seven tests, including
the two rules above stated as assertions (no glyph in the separator, its bg is
`break_`) and the fit-the-column invariant.

**Progress (2026-09-08, the table is drawn — ADR 0002):** The rule-less table
above was rejected on sight, twice: "I want a real table, it's the only thing
that makes sense here." The rules it was built from were quoted back first and
reaffirmed against, so this is a deliberate exception, recorded in
`.claude/adr/0002-markdown-tables-are-drawn.md` and in `CLAUDE.md`'s Design
System and Decision-records sections rather than left as a silent divergence.

A markdown table is now `┌ ┬ ┐ ├ ┼ ┤ └ ┴ ┘ ─ │`, one cell of padding either
side of each cell's content, header above a `├─┼─┤` rule, closed top and
bottom — except below `3n + 1` cells for `n` columns, where a column can shrink
no further and the rows are clipped with `…`, losing the right edge. That is
chosen over drawing an edge where the table does not end and over dropping
columns silently; pinned by
`a_table_with_more_columns_than_cells_clips_rather_than_lying`. Rules in `quiet` — the tier below `dim`, so the grid carries the
structure without competing with the cells; header stays `label`, cells stay
`body`.

**What the exception is, precisely**, because its bounds matter more than the
glyphs: Turn 13's rule governs boundaries between *regions* — a bar from the
transcript, a turn from the one before it, a quoted field from the prose around
it — and a step on the ground ladder expresses those exactly. A table's
boundaries are between *cells*: one per column, repeated down every row, and
they have to agree with each other. A ladder is one-dimensional and cannot
express that, which the rejected version demonstrated in practice — column
position alone held the shape only while every cell was populated and every
column comfortably wide. Nothing else in the crate changes: turn breaks,
markdown `---`, first-run step separators and every band boundary are still
bands, `Block::bordered()` is still out, and the inline diff and code fence are
still recessed fields with no outline (`row.rs:63-75` records the bug class that
fix removed — do not reopen it by citing this entry).

`CELL_GUTTER` (2 cells between columns) became `CELL_PAD` (1 cell either side
of a cell's content): a drawn rule needs far less clearance than a rule-less
layout did, since the rule itself now parts the columns, and two would have a
five-column table spending 15 cells on air. Every column is padded to its full
width including the last, which a rule-less row deliberately did not do — here
the trailing run is what holds the closing `│` on the column the rule rows put
their corner, and one cell short leaves the box visibly unclosed.

`render_snapshot.rs`'s "no box-drawing glyph anywhere" assertion still covers
all eleven chrome scenes and is unchanged; its doc comment now names this
exception, so adding a table scene to `SCENES` fails with the reason rather
than confusingly. Tests: the closed-box shape (corner to corner, every row one
width, the interior rule holding one column down the whole table), the rule
tone against the cell tone, and the alignment/elision/streaming assertions from
the previous entry, updated for the new geometry. Note for anyone extending
them — `str::find` returns a *byte* offset and `│`/`─` are three bytes each, so
column assertions go through the `cell_pos` helper.

**The screenshot harness was lying, and that was a real bug** — found because
the first drawn table "looked misaligned" in the PNGs while the buffer behind
it was exact (rules on columns 13/23/33/82/90 in all eight rows, every row one
width). `examples/snapshot.rs` emitted one absolutely-positioned `<i>` per
*styled run*, so glyphs inside a run advanced at the font's own width —
DejaVu Sans Mono is 0.6015625em, 9.0234px at 15px, against the grid's 9 —
and a rule row, which is one unbroken 78-cell run, finished ~2px right of the
`│` in the content row below it, which re-anchors at every style change. Prose
never showed it; a drawn box could not hide it. Now one `<i>` per *cell*,
walked over the buffer rather than over an accumulated string (a double-width
glyph is one cell whose neighbour holds an empty symbol, so a char index falls
behind from the first wide glyph onward). Costs ~10x the page size, which a
design harness can afford; a wrong screenshot cannot. This predates the table
and would have quietly misinformed any future cell-alignment review — which is
exactly what `IMPORT.md`'s rule 5 ("render it before trusting your reading of
it") exists to prevent, so the renderer itself has to be trustworthy. Note
when reviewing output: at 2x device scale the rules still show hairline seams
from rasterisation alone; at 4x they are continuous, so screenshot the
harness at 4x when the question is whether something lines up.

**Debt this leaves:** the upstream design system still has no table component
and its Iconography table still has no box-drawing glyphs, so this crate now
draws a mark the imported system does not list. ADR 0002's Follow-up carries
it: add the component upstream, re-sync `.claude/design/`, then reconcile
`IMPORT.md`'s glyph-vocabulary section with the ADR.

**Progress (2026-09-08, Shift+Enter — closing the 2026-08-29 gap):** Reported:
"it did work on my Linux machine but on macOS it isn't working", on
Ghostty/Kitty/WezTerm — a terminal family that implements the Kitty keyboard
protocol on both platforms, so this was a real defect and not the
terminal-can't-report-it case the fallbacks exist for.

`run.rs` pushed `DISAMBIGUATE_ESCAPE_CODES` **only** when
`supports_keyboard_enhancement()` answered yes. That query is a write-then-wait
round trip with a 2s timeout whose reply is read off the same input the process
is taking over, so anything that eats or delays the reply — a multiplexer not
forwarding it, a slow answer, a race with another reader — answers "no" for a
terminal that would have honoured the push. It then fails silently, and
Shift+Enter submits. Asking was the fragile part, so it is no longer asked: the
push is unconditional and best-effort, exactly like the alternate-scroll and
synchronized-output modes either side of it (`CSI > 1 u` is a private sequence a
terminal without the protocol ignores). The pop is unconditional too, which is
what keeps the pair symmetric — `TerminalGuard` no longer carries an `enhanced`
flag, because there is no longer a detection result the two ends could disagree
about.

Measured on a real pty, before and after, same binary path and same seeded
config:

| | kitty push `CSI > 1 u` | first paint |
| --- | --- | --- |
| before | absent | 2.02 s |
| after | sent | 0.01 s |

The 2-second stall was the detection timeout, paid at every launch on any
terminal that did not answer — a second, unreported cost of the same gate.

Behaviour verified end to end by driving a pty and reconstructing the screen
with `pyte`, rather than by reasoning about it: `\r` submits, `\x0a` (Ctrl+J)
inserts a newline, and `\x1b[13;2u` — what a Kitty-protocol terminal actually
sends for Shift+Enter — inserts a newline and leaves the turn unsent. **This is
the live-terminal check the 2026-08-29 entry asked for and could not perform,
and it closes that gap.** The fallbacks were right; the gate in front of them
was not.

Noted, not fixed: first run has its own terminal setup and sends *none* of these
modes — no bracketed paste, no keyboard enhancement, no alternate scroll. It is
a list picker with no text entry, so nothing there depends on them today, but
the two setup paths are a latent divergence and only one of them is described by
this spec.

**Progress (2026-09-08, scrolling: the drain was polling with a no-op waker):**
Reported: "scrolling the chat is severely broken, it is not smooth at all and is
very laggy and jittery."

Measured first, and everything the previous scrolling entries fixed held up. On
a 40-turn / 997-row transcript at 120×36: `Transcript::sync` 0.9µs, `slice` 3.5µs,
a whole frame 350–410µs whether scrolling or idle, and 3.4KB of escape sequences
per wheel notch. Rendering every offset from the bottom up and re-aligning the
frames confirmed all 299 steps moved by exactly one row, so the scroll *maths*
was right too. None of that explains the report, and the reason is that none of
it is where the defect was.

`run_loop`'s input drain — the one added so a wheel flick lands in a single
frame — was `input.next().now_or_never()` over a `crossterm::event::EventStream`.
`now_or_never` polls with a **no-op waker**. `EventStream::poll_next` treats
every poll as a subscription: when nothing is ready it hands the waker it was
given to its background reader thread and sets an "already armed" flag, and a
later poll carrying the *real* task waker finds that flag set and does not
re-register (`crossterm-0.29.0/src/event/stream.rs`). One `now_or_never` that
came up empty therefore left the stream holding a waker that did nothing, and
terminal input could no longer wake the loop.

It still moved, which is why this survived review and every test in the crate:
the loop was woken by whatever else fired — the 120ms spinner tick, a core
event, the pending-redraw timer — and drained the backlog on arrival. Driving
the real loop under a pty at an ordinary scroll rate (40 notches, one per 12ms,
three cursor keys each) put a **median 42ms and a worst case of 100ms** between
a notch and the frame showing it, in bursts landing on the tick boundary, and
painted 14–15 frames where 32 were due. The transcript moved eight times a
second in uneven jumps instead of sixty times a second smoothly — precisely the
reported symptom, and precisely why it read as *jitter* rather than as slowness.

Isolated to be sure of the mechanism rather than the correlation: drain an
`EventStream` with `now_or_never` until it reports pending, write one key into
the pty, then await the stream. The key was not observed for a full second — it
surfaced only when the 1.5s timeout woke the task from outside.

The fix is to stop polling terminal input as a `Stream` at all. A blocking
reader thread (`run.rs`'s `spawn_input_reader`) moves `crossterm::event::read()`
onto a `tokio::sync::mpsc` channel; the `select!` awaits `recv()` and the drain
uses `try_recv`, an ordinary synchronous method that registers no waker and so
cannot disturb the `recv()` the loop is suspended on. That is the same shape
core events already used correctly in the same loop — one `recv()` in the
`select!`, one bounded `try_recv` drain after it — so both halves now work the
same way instead of one being subtly special. Same pty measurement after:
**median 0.0ms, worst case 1.7ms, no gap over 15ms, 31 frames** — the one-frame
coalescing the drain existed for is fully preserved.

Verified against the shipped `run()` itself, not a replica: the real binary
driven under a pty with the screen emulated, so what was asserted is the
rendered display. Scroll *correctness* passes either way — 20 wheel notches move
exactly 60 rows, PageUp/PageDown step, End returns to the live bottom, 30 of 30
notches move the screen. What changed is the response: **median 38.7ms / max
41.8ms before, median 5.6ms / p90 16.9ms / max 20.1ms after** — from roughly two
and a half frames behind the wheel to inside one.

The defect lives entirely in how a future is polled, so no rendering or unit
test in this crate could see it; `tests/input_wakeup.rs` is a source-level guard
against reintroducing either spelling.

Noted, not fixed: because alternate scroll mode delivers wheel notches *as
ordinary cursor keys*, a wheel notch is indistinguishable from a real Up/Down
press, and `App::handle_key` gives a multi-line draft's cursor first refusal on
those. So with a multi-line draft in the composer the wheel moves the caret
instead of the transcript. That is the documented keybinding working as
specified, and it is not separable from the wheel while capture stays off (which
it must, or text selection goes) — but it is a real rough edge worth a decision
of its own rather than a silent change here.

**Progress (2026-09-21, the rebrand: Mjolnir → Aldwin):** A name change, and
the only thing in this crate it moved is a width. `BRAND` went from seven
letters to six, so the identity bar's pad went from three cells to four — the
*position* did not move, because `brand_pad()` has always been
`CONTENT_INDENT - MARGIN_X - BRAND.width()` rather than a literal 3, and the
cwd still lands on cell 13 in every scene
(`the_identity_bar_puts_the_working_directory_on_the_body_column`, unchanged
and still passing). The first-run wordmark is the one place a number had to
move: the rule is the **two-space pad, one space between letters**, from which
`  M J O L N I R  ` derived a 17-cell field and `  A L D W I N  ` derives 15.
The reference frames still spell the old name, so the app and the design now
disagree on that width by design — recorded as `wordmark-letters-are-the-old-name`
in `crates/review/baseline.json` and retired when the design system is renamed
upstream and re-synced (`aldwin-open-tasks.md` entry 2a). `render.snap` and the
three README screenshots were regenerated; stages 0–4 of the review loop are
clean. Everything else was the name itself: crates `mjolnir-*` → `aldwin-*`,
binary `mjolnir` → `aldwin`, config dir `~/.mjolnir/` → `~/.aldwin/` (with
`migrate_legacy_global_dir` chaining both rebrands, newest-first), and the
repository `mjolnir-harness` → `aldwin-agent`.

## References

- .claude/spec/aldwin.md — parent spec; layout decisions, UX posture, Edit friction rules.
- .claude/spec/aldwin-core.md — event/command types, turn/step model, thinking-content contract.
- .claude/spec/aldwin-tools.md — ToolApprovalRequested semantics, ApproveTool command.
- https://ratatui.rs/ — ratatui.
- https://docs.rs/crossterm/ — crossterm terminal backend.

### Design sources

Both live on `claude.ai/design` and are read with the `DesignSync` tool
(`/design-login` first; the tool is main-session only — subagents do not
have it). See `.claude/CLAUDE.md`'s Design System section for the working
notes on fetching and rendering them.

A local copy is checked in at `.claude/design/` — `HANDOFF.md`, `SYNC.md`
and `tokens/{cells,palette,semantic}.css`, with provenance and the
two-project distinction in `IMPORT.md`. Read that before re-fetching.

- **"Design system tokens discussion"** — `https://claude.ai/design/p/25845063-2993-4020-ae58-4e7defc6bfef`
  — the handoff bundle (`Agent TUI v2.dc.html`, `Agent TUI v2 Light.dc.html`,
  a revision log) **and the live token layer**, bound in under
  `_ds/mjolnir-design-system-4ea574fb-…/`. `tokens/semantic.css` there is
  what `palette.rs`'s `--tui-*` fields mirror one-to-one. `tokens/cells.css`
  is the grid: cell 9×20px, frame 120×36 cells, `--margin-x: 27px` (3
  cells), `--label-col: 72px` (8), `--label-gutter: 18px` (2) — body text
  therefore lands on cell 13, with **no `--body-col` token**, deliberately —
  `--bar-top-h: 60px` (3 rows), `--bar-bottom-h: 100px` (5 rows). Its
  `SYNC.md` is the change record. Cell positions exist *only* in the
  `.dc.html`, as pixel values in inline styles — see the 2026-09-03
  Progress entry for why reading it without measuring it is not enough.
- **Mjolnir Design System** — `https://claude.ai/design/p/4ea574fb-4be4-47de-9940-fd38927d6dd8`
  — the *source* project, and currently **stale**. Its guideline cards,
  components, UI kits and `SKILL.md` still state pre-Turn-13 rules; `SYNC.md`
  lists exactly what was never pushed back to it. Do not read values from
  here without checking them against the bound copy above.
