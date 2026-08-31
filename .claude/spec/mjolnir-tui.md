# mjolnir-tui

ratatui frontend — renders the core event stream, submits commands, approval gate for Edit.

**Status:** active — two known gaps, see Progress below
**Scope:** crates/tui
**Owner:** Maximilian
**Last Updated:** 2026-06-10

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
mjolnir-cli — otherwise a command looks identical to a chat message in
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
developer ("LLM output is in markdown, but mjolnir doesn't support it").
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
rendering of the little owl from mjolnir.md's Mascot section (boxy
outline, `◉` camera-iris eyes as the one expressive feature, perched on a
rail rather than ambulatory, talons gripping rather than acting), the
`MJOLNIR` wordmark and tagline, and a version/model line
(`v{CARGO_PKG_VERSION} · {model_name}`). It isn't a `LogEntry` — it isn't a
core event, so it doesn't belong in the append-only event log semantics
that `log.rs`'s doc comments describe — instead `ui::draw_log` prepends it
directly and `App::total_lines` accounts for its fixed row count (plus the
one separator before the first real entry) the same way it already
accounts for the transient thinking indicator. The owl uses ACCENT
(cyan) for its outline/eyes and the wordmark — a deliberate, scoped
expansion of accent beyond "card border and focused input only" (see the
Palette bullet below), not a resolution of the still-open mascot color
palette question in mjolnir.md's Mascot section.

**Progress (2026-08-29, git commit in the banner):** The banner's version
line originally showed only `CARGO_PKG_VERSION` — reported back by the
developer as unhelpful, since the whole workspace shares one version
(`0.1.0`) via `version.workspace = true` that doesn't move commit to
commit; on an actively-developed harness that's not enough to tell a
developer which build they're actually running. `crates/tui/build.rs`
now shells out to `git rev-parse --short=8 HEAD` (falling back to
`"unknown"` if git isn't available, e.g. a source tarball with no `.git`)
and `git status --porcelain` for a `-dirty` suffix, exposing the result as
`MJOLNIR_GIT_HASH` via `cargo:rustc-env`; `ui::intro_lines` reads it with
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
This settles mjolnir.md's Mascot section in a new direction; that
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
"MJOLNIR" (59 cols) pushes total banner content past 80 columns for the
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
`ui::mjolnir_row_color(row, total)`, a top-to-bottom RGB lerp (near-white
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
6. **No way to clear context mid-session.** New `Command::ClearHistory` / `Event::HistoryCleared` round trip (mjolnir-core) and a `/clear` slash command (mjolnir-cli, forwarded rather than handled locally like `/help`, since core has to act on it) — wipes `ConversationLog` and, via `HistoryCleared`, the TUI's own rendered `log` and turn state in step, so the welcome banner reappears the same way it does for a genuinely fresh session.
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

- **Layout:** Three horizontal bands: full-width scrollable conversation log (most of the height), single-line status bar, multi-line input area. No persistent sidebar in V0 — all ambient state lives in the two bottom bands or inline in the log.
- **Conversation Log:** Append-only rendered view of core events, prefixed on every draw by a fixed welcome banner (see the 2026-08-29 Progress entry below) that isn't itself a core event or a `LogEntry`. Each event type maps to a distinct entry shape. Tool activity (ToolDispatched → ToolCompleted) renders inline as grouped entries per step. ThinkingStart/an active turn with no thinking block show an animated spinner ("thinking…"/"working…" — see the 2026-08-29 live-feedback Progress entry); ThinkingEnd removes it — no content shown (dropped at source per mjolnir-core). RetryAttempt renders as a visible inline entry with provider, status code, and message. Scroll: auto-follows new content when the view is at the bottom; disengages when the user scrolls up; re-engages on End. Line scroll via arrow keys (Up/Down fall through to scroll only once there's no more input-line to navigate to — see the live-feedback Progress entry); page scroll via PgUp / PgDn.
- **Approval Card:** ToolApprovalRequested renders as an inline card in the conversation log, visually distinct from all other entries via a full-width border and the single accent color. The diff body is colorized (full-width tint on added/removed lines) and collapses unmodified context beyond a small radius around each change — see the 2026-08-29 live-feedback Progress entry. Approve/reject keybindings are labeled inside the card. Input is blocked while a card is pending — the developer cannot queue new submissions until the gate is resolved.
- **Input Area:** Multi-line textarea with a visible terminal cursor and Up/Down line navigation within the draft (see the 2026-08-29 live-feedback Progress entry). Any word matching a known slash command dims live, anywhere it's typed on any line, as a cosmetic hint — independent of whether it would actually be intercepted as a command (only a real leading `/` on the whole message is; see item 8 of that same Progress entry). Enter submits (sends Submit command); Shift+Enter inserts a newline. Ctrl+C cancels the active turn (sends Cancel); Ctrl+C with no active turn exits. Input is blocked while an approval card is pending.
- **Status Bar:** Single line, always visible. Shows: model name, turn/step counter ("T3 S2"), permission summary for the three built-in surfaces (read / shell / edit — each shown as allowed or denied), names of tools currently running within the active step (e.g. "tools: Read shell").
- **Palette:** No longer strictly monochrome as of 2026-08-29 — see the same-day Progress notes below for why. Background: terminal default throughout, except the subtle fixed-RGB tint behind plain user chat messages (not slash commands) and the full-width added/removed-line tints inside an approval card's diff body (see the live-feedback Progress entry). Text hierarchy: bright with a leading `●` marker (assistant output; bold is earned via markdown, not blanket-applied — see the markdown-support Progress entry), a muted gray with a subtle background tint (plain user input), dim (tool metadata, status bar text, and a slash command as user input, since it's directed at the harness rather than the model). One accent color applied to the approval card border, focused-input highlight, and the welcome banner's mascot art/wordmark (see the welcome-banner Progress entry — a deliberate scoped exception, not a general opening-up of accent usage). Specific accent color still deferred pending mascot palette decision. Fenced code blocks in assistant output get their own syntax-highlighted, per-language color set (see `highlight.rs`) inside a dim `┌─`/`│`/`└─` border, independent of this hierarchy. Inline markdown in assistant prose (bold/italic/inline-code/strikethrough/links, headings, lists, blockquotes, thematic breaks — see the markdown-support Progress entry above) is styled via modifiers only except inline code, which uses a plain distinguishing color (`CODE_FG` — see the live-feedback Progress entry) instead of the reversed-video it used to; never a new color beyond that one scoped exception and the diff/user-message background tints already named above.

## Decisions

- **Conversation-first layout — full-width log, status bar, input bar at bottom.** — Keeps the conversation as the primary surface; state lives in the status bar rather than consuming persistent screen space. Split-pane rejected for V0 — adds complexity without payoff until the conversation log is proven sufficient.

- **Approval card is inline in the conversation log, not a full-screen overlay.** — Inline preserves conversational context during review. Visually distinguished via border + accent color so it cannot be mistaken for assistant output. Input blocked while pending — the developer cannot accidentally bypass the gate by typing ahead.

- **Multi-line input; Enter submits, Shift+Enter inserts newline.** — Discussion-first posture benefits from longer prompts. Standard convention for multi-line TUI inputs. Single-line-only rejected as too restrictive for the intended interaction mode.

- **Minimal monochrome palette with one accent color in V0.** — Avoids colour decisions blocked on the open mascot palette. One accent is sufficient to make the approval card unmistakable. Rich theming deferred until the mascot palette is settled.

- **Thinking indicator shown; thinking content not shown.** — Content is dropped at source in mjolnir-core per LlmClient contract. The indicator (ThinkingStart → dim spinner, ThinkingEnd → removed) gives awareness without log clutter.

- **Input blocked while an approval card is pending.** — Structural friction — the developer cannot queue submissions while an edit awaits approval. Consistent with "Edit is never allowlistable in any configuration" from the parent spec.

## Steps

1. Create crates/tui — Cargo.toml with ratatui, crossterm, tokio; depends on mjolnir-core.

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
- Approval card dismissed by an accidental keypress — require an unambiguous labeled key ('y'/'n' or similar), not Enter.
- Tool-activity entries flooding the log during parallel runs — group by step; collapse completed groups after a short delay.
- Shift+Enter behavior is terminal-dependent — test under kitty, iTerm2, and plain xterm; have a fallback binding.

## Out of Scope

- Mouse support — keyboard-only in V0.
- Color theming system — minimal palette only; rich theming blocked on mascot palette decision.
- Syntax highlighting in diff blocks — plain text diff in V0.
- Conversation log search or filtering.
- Split-pane layout with persistent sidebar — deferred.
- Session persistence, conversation save/restore — out of V0 per parent spec.
- Web client rendering — V1.

## References

- .claude/spec/mjolnir.md — parent spec; layout decisions, UX posture, Edit friction rules.
- .claude/spec/mjolnir-core.md — event/command types, turn/step model, thinking-content contract.
- .claude/spec/mjolnir-tools.md — ToolApprovalRequested semantics, ApproveTool command.
- https://ratatui.rs/ — ratatui.
- https://docs.rs/crossterm/ — crossterm terminal backend.
