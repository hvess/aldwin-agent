# mjolnir-tui

ratatui frontend — renders the core event stream, submits commands, approval gate for Edit.

**Status:** active — two known gaps, see Progress below
**Scope:** crates/tui
**Owner:** Maximilian
**Last Updated:** 2026-09-02

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
`mjolnir-tui` tests pass (up from 82; new coverage: the inner-width
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
literal pink/magenta palette, which would clash with Mjolnir's own
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
Mjolnir has exactly one focusable widget (the input box) outside of a
modal card, so there is nothing for either feature to navigate between yet;
building either now would be speculative complexity with no current use,
not a UX gap this session actually has. Per-pane tabs (Headers/Body/Query/
...) don't apply either — Mjolnir's "content" is one linear conversation
log, not several independent structured sections. These are noted here as
considered-and-deferred, not silently dropped, in case the interaction
model ever grows enough panes to make them worth revisiting.

92 `mjolnir-tui` tests pass (up from 89; new coverage: `tool_color`
determinism, the live/scrolled badge in both states, its absence during the
hero), full workspace `cargo test`/`cargo clippy -p mjolnir-tui -- -D
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
relative to the shown diff since `mjolnir_tools::diff::unified` emits no
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

Verified two ways: 93 `mjolnir-tui` tests pass (up from 91; two coordinate-
pinned tests — `user_and_assistant_messages_are_visually_distinct`,
`a_slash_command_renders_differently_from_a_plain_user_message`, and three
more — were converted from hand-derived row offsets to `find_row` since the
padding changes shifted them, the same migration this file's history already
describes doing once before for the same reason), `cargo clippy -p
mjolnir-tui --all-targets -- -D warnings` clean; and a throwaway scratch unit
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
actual cause lives in `mjolnir-core`'s base system prompt
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

Verified: `mjolnir-tui`'s existing 93 tests still pass unmodified (none
asserted an exact blank-row count around assistant text, only that *a*
blank row exists between entries — `a_blank_line_separates_consecutive_
log_entries`); `mjolnir-core`'s 11 tests pass unmodified; `cargo clippy -p
mjolnir-tui -p mjolnir-core --all-targets` clean on both touched files.

**Progress (2026-08-31, duplicated-input turn + wrapped-prose padding):**
Two more developer-reported live-use bugs, one in each of a
still-active spec (`mjolnir-tui`) and an already-archived one
(`mjolnir-core`) — noted here since this file is where a developer would
look first for a TUI-surfaced complaint, even though the root cause landed
outside this crate. (1) The developer reported that submitted input
sometimes reached the model duplicated — visible by asking the model to
echo back what was sent. Not a TUI input-handling bug (`handle_key` already
filters to `KeyEventKind::Press`, and crossterm reports paste as ordinary
key events with bracketed paste unhandled/off, so pasted text was never
actually duplicated at the input layer) — the real bug was in `mjolnir-
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

Verified: `mjolnir-tui`'s 94 tests pass (93 + the one new one) and
`mjolnir-core`'s 12 tests pass (11 + the one new one), `cargo build
--workspace` and `cargo test --workspace` clean, `cargo clippy -p
mjolnir-tui --all-targets` clean on the touched file (`ui.rs`); `agent.rs`'s
new test reuses the same `loop { match ev_rx.recv()... { Event::X => break,
_ => {} } }` idiom every other test in that file already uses, including
clippy's pre-existing `single_match` note on that idiom, which this file
already carries elsewhere and doesn't gate on.

**Progress (2026-09-01, queued approvals/prompts):** Developer report: "when
the LLM requests multiple diffs or permissions at once, it breaks the
approval process and the user can only approve one thing." Root cause: both
gates (`App::pending_approval`/`pending_prompt`, `crates/tui/src/app.rs`)
were a single `Option<T>`, but the underlying round trip was never
single-outstanding — `mjolnir-core`'s `Agent::dispatch_tools` drives every
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

Verified: `mjolnir-tui` 98 tests pass (94 + 4 new), full workspace build/
test (306 tests) and `cargo clippy -p mjolnir-tui --all-targets` clean on
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

Test suite: `mjolnir-tui` 104 tests pass (98 + 6 new — the two clipping/
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
(311 tests, 1 ignored, pre-existing) and `cargo clippy -p mjolnir-tui
--all-targets -- -D warnings` both clean; a pre-existing, unrelated
`single_match` clippy failure in `mjolnir-core::agent.rs`'s own test module
(the same idiom mjolnir-tui.md's 2026-08-31 wrapped-row-scroll-math entry
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
exact spacing. `mjolnir-tui` 105 tests pass (104 + 1 new); full workspace
`cargo test` (312 tests) and `cargo clippy -p mjolnir-tui --all-targets --
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
Ctrl+C has always meant (mjolnir-tui.md's 2026-08-29 live-run fix and the
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

Verified: `mjolnir-tui` 109 tests pass (105 + 4 new — a numbered
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
clippy -p mjolnir-tui --all-targets -- -D warnings` both clean. Visually
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
experiences, not a test of the accessor's internals. `mjolnir-tui` 112 tests
pass (111 + 1 new); full workspace `cargo test` (319 tests) and `cargo
clippy -p mjolnir-tui --all-targets -- -D warnings` both clean.

`mjolnir-tui` 111 tests pass (109 + 2 new); full workspace `cargo test` (318
tests) and `cargo clippy -p mjolnir-tui --all-targets -- -D warnings` both
clean.

**Progress (2026-09-02, directory-scope prompt option + humanized prompt
title):** Developer report: permission prompts were "aggressive" for
ordinary reading (each new file under an already-trusted directory
re-prompted individually) and didn't make clear what was actually being
asked ("a human readable explanation... and then underneath in small/
greyed out text what the raw tool call actually is"). Two changes, both
scoped to a pending `PromptPayload::Tool` in the decision panel — no
change to the Approve/Deny binary shape a pending `ToolApprovalRequested`
(Edit) uses, per mjolnir's non-negotiable "Edit is never allowlistable."

(1) `render_prompt_card` now shows a per-kind humanized sentence
("Claude wants to read a file") as the accent/bold title, with the
literal `kind: target` demoted to a dim subtitle underneath
(`humanize_prompt`/`raw_prompt_call`, `ui.rs`) — same information as the
old title, just split by primary/secondary instead of concatenated into
one string a developer had to parse.

(2) A new Tab-toggleable scope for the tier options' persisted pattern
(`App::decision_pattern_scope`, `PatternScope::{Exact,Directory}`) —
available only when the payload says `path_like: true` (threaded from
mjolnir-tools' new `Tool::permission_target_is_path`, through
mjolnir-permissions' `check_tool`) and the target has an enclosing
directory to broaden to (`App::directory_glob`: `"./crates/tui/src/
ui.rs"` → `"./crates/tui/src/**"`; a bare filename with no `/` offers
nothing). Deliberately *not* a 9th option or a doubled allow/deny×scope
list — per this spec's own Pitfall-adjacent discipline (see mjolnir-
permissions.md's "four-tier prompt growing a fifth option... each tier
doubles cognitive load") the 8 tier labels stay exactly as they were;
Tab flips which pattern they'd all persist, shown via a new dim hint
line above the list (`ui::scope_hint_line`, `App::decision_scope_hint`)
— "scope: this file (...) · Tab for this directory (...)" and its
reverse once toggled. The chosen pattern rides in a new `pattern` field
on `PromptResponse::Tool` (mjolnir-permissions), replacing the
dispatcher's old behavior of always persisting the exact target
verbatim — see mjolnir-permissions.md/mjolnir-tools.md's matching
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
scope states, and its absence for a non-path-like target). `mjolnir-tui`
128 tests pass (112 + 16 new); full workspace `cargo test` and `cargo
clippy -p mjolnir-permissions -p mjolnir-tools -p mjolnir-tui
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
   assumed away. See `mjolnir-tui.md`'s Out of Scope bullet below, narrowed
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
   either. `TuiConfig.theme: Option<String>` (mjolnir-config) already exists
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
`mjolnir-tui` 134 tests pass (128 + 6 new: 3 in `app.rs` for item 2, 3 in
`ui.rs` for item 1 — item 4's test replaced an existing one rather than
adding a new one, since the old assertion was pinning down the bug); full
workspace `cargo test` (347 tests) and `cargo clippy -p mjolnir-tui
--all-targets -- -D warnings` both clean.

**Progress (2026-09-02, self-review of the batch above finds a real gap in
item 1):** A manual audit of this session's own diff (correctness pass over
`activity_label`/`run.rs`/`handle_mouse`, plus the mjolnir-config side of
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
`mjolnir-tui` 135 tests pass (134 + 1); full workspace `cargo test` (348
tests) and `cargo clippy -p mjolnir-tui --all-targets -- -D warnings` both
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
legible. `mjolnir-tui` 135 tests pass unmodified (no test hardcoded the old
`Color::White`/`Color::DarkGray` values directly — all reference the `DIM`/
`BRIGHT` constants, which is exactly why none needed touching); full
workspace `cargo test` (348 tests) and `cargo clippy -p mjolnir-tui
--all-targets -- -D warnings` both clean.

Lesson recorded plainly since it's a real process gap, not just a code one:
"tested in a light-mode terminal" needs a real remapped ANSI palette, not
just a flipped default fg/bg — the two are different mechanisms, and this
file's own established verification technique (a real terminal session,
captured, not just `TestBackend`'s text-only dump) is what actually caught
this the second time.

- **Layout:** Five horizontal bands (was four before the 2026-09-02 decision
  panel): the body (full-width scrollable conversation log, or the log
  beside a secondary sidebar — see the 2026-08-31 visual-redesign Progress
  entry), a 1-row status line, the decision panel (zero-height and invisible
  whenever nothing is pending — see the Approval Card bullet below), and the
  multi-line input area. The sidebar is optional, off by a narrow-terminal
  width gate regardless of the developer's own Ctrl+T preference, and never
  taken as license to shrink the log panel below 80 columns when shown —
  ambient state, not a primary layout element competing with the
  conversation.
- **Conversation Log:** Rendered inside a bordered, rounded ratatui panel (see the 2026-08-31 visual-redesign Progress entry) with a `Scrollbar` shown when content overflows the viewport. Append-only rendered view of core events; the welcome banner (see the 2026-08-29 Progress entry below) only shows when the log is empty — mutually exclusive with real entries, not prefixed above them, since the two used to always coexist and that's what made the banner eat real screen space mid-conversation. Each event type maps to a distinct entry shape, most with a leading glyph (● assistant, ▸/✓/✗ tool activity, ⟳ retry, ✗ error, ℹ notice — see the 2026-08-31 entry). Tool activity (ToolDispatched → ToolCompleted) renders inline as grouped entries per step. ThinkingStart/an active turn with no thinking block show an animated spinner ("thinking…"/"working…" — see the 2026-08-29 live-feedback Progress entry); ThinkingEnd removes it — no content shown (dropped at source per mjolnir-core). RetryAttempt renders as a visible inline entry with provider, status code, and message. Scroll: auto-follows new content when the view is at the bottom; disengages when the user scrolls up; re-engages on End. Line scroll via arrow keys (Up/Down fall through to scroll only once there's no more input-line to navigate to — see the live-feedback Progress entry) or the mouse wheel (`App::handle_mouse`, 2026-09-02 — same one-line-per-notch behavior as the arrow keys, and, unlike them, not blocked while a decision is pending); page scroll via PgUp / PgDn.
- **Approval Card / Decision Panel (2026-09-02, superseding "inline in the log" below):** ToolApprovalRequested/PromptRequested no longer render inline in the conversation log while pending — they render in a fixed decision panel directly above the input box (`ui::decision_panel_lines`, driven by `App::pending_approvals`/`pending_prompts`), visually distinct via the single accent color, same as before. The diff body is colorized (full-width tint on added/removed lines) and collapses unmodified context beyond a small radius around each change — see the 2026-08-29 live-feedback Progress entry; an unusually large diff is further truncated (`ui::clamp_panel`) to keep the options list on screen. The decision itself is a numbered, keyboard-navigable list (`App::decision_options`/`ui::render_decision_options` — same-day "numbered, arrow/digit-navigable list" entry), not raw letter shortcuts: Up/Down move a visible `▸` cursor, Enter confirms the selected option, a digit `1`-`9` jumps to and confirms an option directly, and Ctrl+C always resolves the safe one-time decline regardless of cursor position. Input is blocked while pending — the developer cannot queue new submissions until the gate is resolved. Once resolved, the full card (diff included) still renders inline in the log exactly as before, as a permanent historical record — only the *live* interaction moved out of the scrolling log, not the history.
- **Input Area:** Multi-line textarea, rounded border (2026-08-31), with a visible terminal cursor, dim placeholder text when empty, and Up/Down line navigation within the draft (see the 2026-08-29 live-feedback Progress entry). Any word matching a known slash command dims live, anywhere it's typed on any line, as a cosmetic hint — independent of whether it would actually be intercepted as a command (only a real leading `/` on the whole message is; see item 8 of that same Progress entry). Enter submits (sends Submit command); Shift+Enter inserts a newline. Ctrl+C cancels the active turn (sends Cancel); Ctrl+C with no active turn exits. Ctrl+T toggles the sidebar (2026-08-31). Input is blocked while an approval card is pending.
- **Status Line / Sidebar (2026-08-31, superseding the separate header/footer/sidebar trio and the single "Status Bar" further below — see `draw_status_line`'s own doc comment):** A single 1-row status line, positioned directly above the decision panel/input rather than a separate top header and bottom footer: live activity (thinking/idle, with a spinner, plus a descriptive leading word for the rest of an active turn — a named tool in flight, "responding…" once assistant text is streaming, or "working…" while waiting on the first token/tool call of the step; see `ui::activity_label`, 2026-09-02), model name, turn/step counter, any tools currently in flight (colored per name), and a running message count — always this content, even while a decision is pending (2026-09-02: the decision panel is now the one place pending keys show; the status line no longer special-cases them). An optional sidebar (secondary, width-gated — see the Layout bullet above): permission detail, active tools by name with a spinner, turn/step, message count. Neither participates in `ScrollState` — only the log panel scrolls.
- **Palette:** No longer strictly monochrome as of 2026-08-29 — see the same-day Progress notes below for why, extended further in the 2026-08-31 visual-redesign Progress entry (all color constants now live in `palette.rs`). Background: *stale as of the 2026-08-31 visual-redesign entry above, corrected here 2026-09-02* — no longer terminal default throughout. That redesign's opaque-surfaces pass (`BG_BASE`/`BG_ELEMENT`/`BG_INPUT`/`CODE_BG`, `palette.rs`) fills the whole frame and every panel/bubble/box with its own fixed-RGB tier; this wording describing only a "subtle tint" on top of an otherwise-transparent terminal background was never updated to match and had drifted into being actively misleading — see the 2026-09-02 live-feedback batch's light-mode-contrast item, which traced a real bug to exactly this gap between the two (a span left without an explicit foreground, which reads fine against a *transparent* background inheriting the terminal's own contrast pairing, but not against the app's own always-dark fill).

  Text hierarchy: bright with a leading `●` marker (assistant output; bold is earned via markdown, not blanket-applied — see the markdown-support Progress entry), a muted gray with a subtle background tint (plain user input), dim (tool metadata, header/footer/sidebar text, and a slash command as user input, since it's directed at the harness rather than the model). `BRIGHT`/`DIM` are fixed RGB as of 2026-09-02 (see the same-day Progress entry) — they were the one exception to this file's "fixed RGB, not named ANSI" rule until a real light-mode terminal theme (Solarized Light) was shown to remap them into near-unreadable territory against this app's own always-dark surfaces. One accent color applied to the approval card border, focused-input highlight, the log panel's live/scrolled status badge, and the welcome banner's mascot art/wordmark (see the welcome-banner Progress entry — a deliberate scoped exception, not a general opening-up of accent usage) — explicitly *not* widened to ordinary panel borders (log/sidebar/dimmed-input), which use `PANEL_BORDER` instead, keeping accent meaning "this needs your attention" rather than "this is a panel." As of 2026-08-31, `PANEL_BORDER` is a muted tint of `ACCENT`'s own hue (not a `DIM`-gray alias) — a posting-inspired refinement of this same discipline, not an exception to it; see that Progress entry. Specific accent hue itself still deferred pending mascot palette decision. Two genuinely new colors: `WARNING_FG` (amber, retry entries) and the 6-hue `TOOL_PALETTE` (per-tool-name sidebar chips, 2026-08-31). Permission allow/deny states (`access_spans`, shared by header/hero/sidebar) render as small padded chips (colored background) rather than bare colored text, also 2026-08-31. Fenced code blocks in assistant output get their own syntax-highlighted, per-language color set (see `highlight.rs`) inside a dim `╭─`/`│`/`╰─` border (rounded as of 2026-08-31, matching every other panel), independent of this hierarchy. Inline markdown in assistant prose (bold/italic/inline-code/strikethrough/links, headings, lists, blockquotes, thematic breaks — see the markdown-support Progress entry above) is styled via modifiers only except inline code, which uses a plain distinguishing color (`CODE_FG` — see the live-feedback Progress entry) instead of the reversed-video it used to.

## Decisions

- **Conversation-first layout — full-width log, status bar, input bar at bottom.** — Keeps the conversation as the primary surface; state lives in the status bar rather than consuming persistent screen space. Split-pane rejected for V0 — adds complexity without payoff until the conversation log is proven sufficient. *Refined, not reversed, 2026-08-31:* an optional, secondary, width-gated sidebar was added for ambient state (permissions/tools/turn/messages) the header/footer couldn't fit — it is not the primary split-pane this decision rejected: it auto-collapses on narrow terminals and never takes the log panel below 80 columns when shown, so the conversation stays the primary surface either way.

- **Approval card is inline in the conversation log, not a full-screen overlay.** — Inline preserves conversational context during review. Visually distinguished via border + accent color so it cannot be mistaken for assistant output. Input blocked while pending — the developer cannot accidentally bypass the gate by typing ahead. *Refined, 2026-09-02:* per direct developer feedback that a pending card "in the chat" read as "ugly, not clear, disjointed," the *live* card moved out of the scrolling log into a fixed decision panel directly above the input — still not a full-screen overlay, and still visually distinct via the accent color, but no longer part of scrollback while pending (so it can no longer be scrolled out of view, which was the concrete complaint). A resolved decision still leaves the exact same full card inline in the log as a permanent record, unchanged from before — this refines where the *live* interaction happens, it doesn't reverse the "conversational context is preserved" rationale above, since the history is still right there in the log afterward.

- **Multi-line input; Enter submits, Shift+Enter inserts newline.** — Discussion-first posture benefits from longer prompts. Standard convention for multi-line TUI inputs. Single-line-only rejected as too restrictive for the intended interaction mode.

- **Minimal monochrome palette with one accent color in V0.** — Avoids colour decisions blocked on the open mascot palette. One accent is sufficient to make the approval card unmistakable. Rich theming deferred until the mascot palette is settled. *Extended, not reopened, across several 2026-08-29/08-31 Progress entries:* a small, cohesive set of semantic colors (diff add/remove, code, user tint, warning) was added incrementally, each scoped to one clear role — this is still a fixed, hardcoded palette, not the configurable/user-selectable "rich theming" this decision deferred; that remains blocked on the mascot palette question.

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
- Approval card dismissed by an accidental keypress — require an unambiguous labeled key ('y'/'n' or similar), not Enter. *Superseded, 2026-09-02, by explicit developer request:* the decision panel is now a numbered list with a visible `▸` cursor (see the same-day "numbered, arrow/digit-navigable list" Progress entry) — Enter is a valid confirm action again, since the visible cursor removes the original ambiguity ("what would Enter even do here") this Pitfall was guarding against. The underlying concern (no silent, invisible resolution) still holds; the mechanism satisfying it changed.
- Tool-activity entries flooding the log during parallel runs — group by step; collapse completed groups after a short delay.
- Shift+Enter behavior is terminal-dependent — test under kitty, iTerm2, and plain xterm; have a fallback binding.

## Out of Scope

- Mouse support beyond wheel-scroll — keyboard-only otherwise. *Narrowed, not reopened, 2026-09-02:* mouse capture is now on and the wheel scrolls the log (`App::handle_mouse`, see the same-day "live-feedback batch" Progress entry) — a direct fix for a developer report that the wheel couldn't scroll at all while capture was off. Click/drag/move events reach the app too now that capture is on, but nothing is wired to them; the terminal's own native text selection is still reachable behind its usual bypass modifier (Shift-drag on most terminals) instead of on a plain drag. This is the same shape the 2026-08-31 sidebar entry used for the split-pane rejection: a scoped, deliberate carve-out of one specific interaction, not a reopening of "should this app be mouse-driven."
- Color theming system — a small fixed semantic palette exists now (see the Palette bullet and its Decisions entry), but it's hardcoded, not user-configurable; rich/configurable theming is still blocked on the mascot palette decision.
- Syntax highlighting in diff blocks — plain text diff in V0.
- Conversation log search or filtering.
- Split-pane layout as the *primary* layout element — still deferred. What exists as of 2026-08-31 is a secondary, optional, width-gated sidebar for ambient state, not a split-pane the developer's attention is meant to divide between — see the Conversation-first layout Decision entry above.
- Session persistence, conversation save/restore — out of V0 per parent spec.
- Web client rendering — V1.

## References

- .claude/spec/mjolnir.md — parent spec; layout decisions, UX posture, Edit friction rules.
- .claude/spec/mjolnir-core.md — event/command types, turn/step model, thinking-content contract.
- .claude/spec/mjolnir-tools.md — ToolApprovalRequested semantics, ApproveTool command.
- https://ratatui.rs/ — ratatui.
- https://docs.rs/crossterm/ — crossterm terminal backend.
