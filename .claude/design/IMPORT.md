# Design import — provenance

Imported 2026-09-06 via the `DesignSync` tool (claude.ai/design, MCP endpoint
`https://api.anthropic.com/v1/design/mcp`, authenticated with `/design-login`).

Re-synced later the same day. That pass changed exactly two files —
`tokens/cells.css` (rewritten) and the `cells.css` paragraph of `SYNC.md`.
`README.md`, `tokens/palette.css` and `tokens/semantic.css` came back
byte-identical, and no path was added or removed in either project. The
design-system project's `updatedAt` did not move; see the note below on why that
is expected.

## The lantern-gold repaint — 2026-09-21, and the frames became one file

The largest import since the directory was created, and the first where
**every colour in the app changed**. Read from `25845063-…` with `DesignSync`
after `/design-login`.

### What arrived

The design system is no longer one hue. It was 300° carrying everything —
neutrals at low chroma, the accent at full — and it is now **near-neutral warm
grey with one brand colour, lantern gold `#e9c46a`**, four status hues and two
syntax hues. The coherence argument inverted with it: the old system held
together because everything shared a hue, this one because almost nothing does,
so the gold is the only thing in a resting frame the eye reads as colour.

- **`tokens/palette.css`** — replaced whole. The ground ladder is renamed as
  well as re-valued: `--color-ground-0` is the frame ground and the rungs run
  `up-1…3` / `down-1…3` from it, where they were `--color-ground-0…6` numbered
  from the desk up. Both themes now run the *same way round* — a raised band is
  lighter than the ground in light as well as dark — with one exception the
  file states in place: light's `panel-title` is "the one raised band darker
  than ground, because it is a title".
- **`tokens/semantic.css`** — replaced whole; 51 roles per scope, up from 45.
  New: `--tui-ok|err|warn|info`, the four status roles, each paired with its
  glyph in a comment; `--tui-scrim-text|quiet|mark`; `--tui-done`;
  `--tui-gauge-fill-hot`; `--tui-border`. Gone: `--tui-line`, `--tui-step-done`,
  `--tui-glyph-done` (now `--tui-done`), and two of the five syntax roles —
  `--tui-syn-type` and `--tui-syn-number`, because "types, numbers and every
  other identifier are the code tone".
- **`tokens/cells.css`** — two tokens changed and nothing else: `--bar-keys-h`
  deleted, `--bar-title-h` added. The grid is untouched, which is worth stating
  plainly because everything else moved: 3-cell margin, 8-cell label column,
  2-cell gutter, body text on cell 13.
- **`Aldwin Agent TUI.dc.html`** — one file where there were three. Eleven
  frames renumbered `1a`–`1d` (first run, empty state), `2a`–`2d` (session,
  inline code, multi-line draft, failed step), `3a`–`3c` (permission, commands,
  review). The theme is a toggle on `<body>`, so the light frames are gone as
  separate files. `Agent TUI v2.dc.html` and its Light twin survive only under
  `uploads/`, as the original upload.

**The frames are authored in tokens now.** Every position is
`var(--label-col)`, `var(--row)`, `var(--step-content-col)`; the only `px` left
inside a frame is a 1-cell `padding-right: var(--cell-w)`. CLAUDE.md's rule 1 —
measure the HTML, because positions exist only as pixels in it — describes the
*old* frames. The rule still earns its place as a check, but the measurement it
asks for is now a token lookup. Rule 2 (render it before trusting your reading)
earned its place again: the render is what showed `3a` had regressed.

### The glyph vocabulary changed, and `HANDOFF.md`'s table is now measured

`▶` became `▸` (the prompt) and `✔` became `✓` (a pass). Four status marks are
new — `✓` ok, `✗` failed, `!` warned, `·` the info pointer — each with one hue
and drawn with nothing else. Also new in the frames: `−` U+2212 for a removed
*count* (`+11 −2`, a number, where `-` at the head of a row is diff syntax),
`…` for elided rows, `→` for a row that opens a longer list.

This directory's `HANDOFF.md` mirrors upstream's `README.md` verbatim by
convention. **Its `### Glyphs` table is now the one exception**, marked with a
`>` note saying so, because `crates/review`'s token generator parses that table
into `tokens.rs` — mirroring a stale table would have generated a vocabulary
the design no longer draws. Every row of it was counted out of the frame's
markup.

The upside is real: `MARKS_BY_EXCEPTION` fell from 18 glyphs to 11. The five
marks the old table omitted while the design's own copy mandated them
(`· … → ⏎ ↑↓`) are simply in the frame now, which retired that contradiction
outright. What is left is ADR 0002's box-drawing set alone.

### The prose did not move, and that is most of the new contradiction list

Upstream's `README.md` and the bound copy's `SYNC.md` were **not touched by the
repaint**. The README still opens "Five states are designed", ids them
`4a`/`5a`–`5d`, carries the OKLCH colour tables and the old glyph table, and
writes the brand `mjolnir`. `SYNC.md` ends at Turn 15 with no entry for any of
this. `crates/review/baseline.json` went from seven contradictions to seven,
but they are almost entirely different ones — three old entries folded into
`readme-and-sync-record-describe-frames-that-no-longer-exist`, and
`wordmark-letters-are-the-old-name` retired because the wordmark is gone.

**Two are the design contradicting a decision this project has already
shipped**, and both are recorded rather than followed:

- `permission-frame-predates-adr-0004` — `3a` draws four options over a
  `cargo *` pattern, a "shell command" sentence, a `Deny and tell the agent
  why` row and a `.toml` footer. That is the UI of the model ADR 0004 deleted,
  and the README's own corrected copy (pushed from here on 2026-09-20)
  describes the eight-row list instead. The frame was redrawn from a base older
  than that correction. The app takes `3a`'s tones in full and none of its copy.
- `access-scale-is-four-points-in-the-frame-and-three-in-the-readme` — `1c`
  restores the `all` row reading "everything runs · nothing asks". No rung can
  mean that: ADR 0004 §3 puts `edit` outside the permissions model, so a diff is
  shown and waited on at every rung. `no_access_row_claims_edits_run_unasked`
  fails on that string, so it cannot ship by accident.

### Applied to the working tree

`tokens.rs` regenerated (48 roles per theme, up from 42). `palette.rs` rewritten
around the new role set and its ladder tests re-pinned. The alpha-composited
transcript fade is **gone** — the design says "a recolour, never alpha", so
`Palette::scrimmed` maps ink to the three `--tui-scrim-*` roles and
`ui::scrim_area` applies it; the colour-conformance test consequently dropped
its 48×48 table of blended allowances and now holds every cell, dimmed or not,
to being exactly a token. `highlight.rs` cut to three roles. First run lost its
wordmark and its 3-row footer (now the standard 5-row band with a status row);
the empty state lost its wordmark and its `in` row and is top-anchored; errors,
notices and cancellations became glyph-led status rows; code blocks gained a
caption and a gutter; inline code gained the quoted-code ground; the composer
gained a line count. Snapshot re-recorded deliberately.

## ADR 0004 push — 2026-09-20, the permission copy corrected upstream

**The first push in this direction for a reason other than a re-sync**, and
the first that corrects the design rather than importing it.

ADR 0004 rebuilt the harness's permission model, and it left `5a` and `5d`
describing a product that no longer exists: a four-option list over a `shell`
tool that had been deleted, a grant written as a command pattern (`cargo *`)
where a grant is now a program and a class (`cargo writes`), a `Deny and tell
the agent why` row for a reason field nothing collects, a `saved to
.harness/permissions.toml` footer naming the wrong file and true of three rows
out of eight, and a four-point access scale where there are three rungs.

Written to **`25845063-…`'s top-level `README.md`** — the file this directory
keeps as `HANDOFF.md`. Ten edits, each matching exactly once, and nothing else
in the file touched: the remote base was reconstructed by stripping this
copy's own `>` annotation blocks, verified section by section against the
fetched original, edited, and pushed back. The same ten edits were then applied
here, so the two stay in step.

**What was deliberately not pushed.** The `5a` frame still draws four option
rows in 18, and eight plus a three-row fact table does not fit. The prose now
says so and says explicitly that whether the table shrinks, the band grows or
the list splits is undecided. Redrawing `Agent TUI v2.dc.html` is a design act
and is Maximilian's to make — see the note added under the fact-table bullet.

Also not pushed: the `14c`/`14d` prose still describing the access fact as "the
tier" from a four-point scale. That text lives in the bound copy's `SYNC.md`
under `_ds/`, not in this README, and `crates/review/baseline.json` keeps a
narrowed entry for it.

## Turn 15 sync — 2026-09-07, the light theme rebuilt again

The bound copy had moved ahead on its own and this directory was behind. Read
from `25845063-…`; **written to the design-system project `4ea574fb-…`**, which
is the first time anything has been pushed there — see "The source project is no
longer wholly stale" below.

`Agent TUI v2 Light.dc.html` was fetched and its `:root{--t-*}` block measured
against the bound copy's `.tui-light`. **They agree** — all thirty-eight roles,
including the five new syntax ones. That check is the whole reason to fetch the
frame: it is the one direction nothing else verifies.

What changed here: `tokens/palette.css` and `tokens/semantic.css` replaced with
the upstream text, and `SYNC.md` gained its Turn 15 section. `tokens/cells.css`
came back byte-identical — Turn 15 touched colour, not grid — and the top-level
`README.md` this directory keeps as `HANDOFF.md` was untouched upstream. (Turn
15's "README.md / readme.md" edits are the *design system's* own readme inside
`_ds/`, which this directory has never imported.)

The three substantive moves:

- **The light theme reads ramps, not literals.** Turn 14 left `.tui-light`
  carrying thirty hexes; there are now `--color-ground-light-*`,
  `--color-ink-light-*`, `--color-accent-light-*`, `--color-neutral-light-*`
  and `--color-diff-light-*` ramps, and the light scope is `var()` references
  throughout. The manifest had been recording those ramps as real since Turn
  14 while they did not exist.
- **It is shallower, and that is the point.** Ink tops out at `#241f2b` on a
  `#f7f5fa` ground with a `#cfcad9` desk, where Turn 14 ran `#0e0c12` on
  `#faf7ff` over `#a39fac`. Nothing in the old theme failed a contrast floor;
  it read as harsh at 19:1 ink-to-ground, roughly twice what the dark theme
  asks. Hierarchy is now carried by the step between rungs, not by the
  distance to the ends — so "this looks washed out, darken it" is an undo,
  not a fix.
- **The light ground rungs were renumbered by lightness**, which reorders the
  ladder: `ground`, `bar_bottom`, `bar`, `break`, `recess`, `panel_title`,
  `scrim`. The turn break used to be the second-lightest band and is now the
  fourth, below both chrome bands.

Also new upstream and adopted in the same pass: a **five-role syntax ramp**
(`--tui-syn-keyword|call|type|string|number`, both themes), with the rules that
no syntax role outranks the accent mark and that everything else in a code
block stays `--tui-code` while a comment drops to `--tui-dim`.
`crates/tui/src/highlight.rs` had been loading syntect's `base16-ocean` pair,
which made a fenced block the one region of a frame carrying foreign hues; it
now *builds* its syntect theme from the palette instead of loading one.

Applied to the working tree: `crates/tui/src/palette.rs` (`LIGHT` regenerated,
five `syn_*` fields added, ramp-step annotations, two ladder tests re-pinned),
`crates/tui/src/highlight.rs` (the scope table and two tests), and
`crates/tui/tests/snapshots/render.snap`. The dark half of that snapshot moved
only on code-fence rows; everything else dark is byte-identical.

## Turn 14 sync — 2026-09-06, and the direction it ran

The third pass of the same day, and the first that **wrote upstream** rather
than only reading. Both `.dc.html` frames were fetched in full and diffed
against each other with their hex values masked: they are byte-identical apart
from the `:root{--t-*}` block and two section headings, which is what makes the
light theme a pure re-point of one set of roles and the role mapping below
unambiguous.

What the frames had that this copy did not:

- **A new light palette.** Every `.tui-light` value changed. `tokens/cells.css`
  had already been updated upstream (it carries `--step-mark-col` and
  `--step-content-col`), but `tokens/semantic.css` had not — so the token layer
  and the frames disagreed, silently, in the one direction nothing checks.
- **A rebuilt first run** — three steps on one vertical spine — and a **new
  `14d` frame**, the returning/empty state. Both were *measured*, not copied;
  what came out of them is in `HANDOFF.md`'s first-run section as a third
  supersession note, and in `SYNC.md`.

Files changed here: `tokens/semantic.css` (`.tui-light` regenerated,
`--tui-step-done` added to both scopes), `tokens/palette.css`
(`--color-band-light`, and the accent ramp's comment, which claimed the light
theme draws from 800/900 — it no longer does), `tokens/cells.css` (brought to
the upstream text), `SYNC.md`, `HANDOFF.md`.

**Pushed back** to `25845063-…`: `_ds/…/tokens/semantic.css`,
`_ds/…/tokens/palette.css` and `_ds/…/SYNC.md`, so the bound token layer states
what the frames render. `cells.css` was already current upstream and was not
written. Nothing was pushed to the design-system project `4ea574fb-…`, which
stays stale by the same mechanism described below.

**The frames still read none of this.** Each `.dc.html` restates its whole
palette inline, so updating `semantic.css` changes nothing about what either
one renders — the point of the push is that the two now agree. They can drift
apart again with no error anywhere; only measuring catches it.

## The source project is no longer wholly stale — 2026-09-07

`4ea574fb-…` had never been written to. Every previous pass edited the bound copy
inside the discussion project and left a "Not applied — outside this copy" table
behind, so by Turn 15 the source project's `tokens/palette.css` was still the
**Nocturne cool-indigo neutrals with the dusty azure `#84aed9` accent** — three
turns behind, not one. Its guideline cards were showing five grounds, `--tui-line`
as a live role, a 12-cell label column and a card devoted to the fading rule.

Pushed in this pass:

| Path | What went |
| --- | --- |
| `tokens/{palette,semantic,cells,elevation}.css` | The bound copy's current text, byte-for-byte |
| `README.md`, `readme.md` | The current guideline prose — the light theme's depth argument, the syntax ramp's two anti-rainbow rules, quoted code on `--tui-diff-box` |
| `_ds_manifest.json` | The regenerated token table (238 entries), so the Design System pane stops describing tokens that do not exist |
| `guidelines/` — 12 rewritten, 2 new, 1 deleted | Below |

The cards: `colors-grounds` (five grounds → the seven-rung ladder),
`colors-neutrals` and `colors-accent` (Nocturne/azure hexes → hue 300, with the
150/750 rungs back), `colors-light` (rebuilt on the seven light rungs),
`colors-diff` (opaque row fills, both themes, current values), `brand-accent`,
`brand-alignment`, `brand-voice`, `grid-cell`, `grid-columns` (12/17 → 8/13),
`grid-rows`, `type-hierarchy` (six roles → eight). **New:** `colors-syntax`, the
five-role ramp in both themes. **New, replacing a deleted card:** `brand-break`
takes the slot `brand-rule` held — the fading rule is retired, and the card that
taught it was teaching the one thing Turn 13 removed.

Every card was rendered headless at its declared viewport before pushing (the
`CLAUDE.md` rule), which caught something reading alone would not have: **eleven
of the fourteen were clipping their own captions**, several by 100px or more, and
the declared `viewport` heights were corrected against measured content. All
fourteen are stroke-free and carry no retired value.

**Not pushed, and still stale there:**

- `components/**` (`.jsx`, `.prompt.md`, `.d.ts`) and `_ds_bundle.js`. The bound
  copy's *bundle* is current — Turn 13 rewrote it — but the source project's
  `.jsx` files are what compile into it, so pushing the bundle without the
  sources would be reverted by the next self-check. Thirteen components need the
  edits `SYNC.md`'s Turn 13 table lists: `BarIdentity`'s `▌`, `TurnBreak`,
  `Modal` as a bottom panel, `Turn`'s `labelCells` 12 → 8, the `--tui-line`
  strokes.
- `_adherence.oxlintrc.json` — its token allowlist predates the light ramps and
  the syntax ramp, so authoring against the new tokens will warn.
- `ui_kits/tui/` (five screens, and they already fail to render — React #130, a
  pre-existing bug `SYNC.md` documents) and `templates/harness-session/`.
- `SKILL.md` needed nothing: it is a pointer to `README.md` and states no colour
  or border rule of its own.

## The two projects, and which one is live

There are two distinct objects on claude.ai/design, and they are easy to
confuse because one is embedded in the other.

| | Design system | Discussion project |
| --- | --- | --- |
| UUID | `4ea574fb-4be4-47de-9940-fd38927d6dd8` | `25845063-2993-4020-ae58-4e7defc6bfef` |
| Name | Mjolnir Design System | Design system tokens discussion |
| `type` | `PROJECT_TYPE_DESIGN_SYSTEM` | `PROJECT_TYPE_PROJECT` |
| Holds | tokens, 19 guideline cards, components, UI kits | the five `.dc.html` frames + a **bound copy** of the design system under `_ds/mjolnir-design-system-4ea574fb-…/` |

**The bound copy inside the discussion project is the current one.** Turn 13
decisions were applied there and *not* propagated back to the design-system
project — `SYNC.md` carries an explicit "Not applied — outside this copy" table
listing what was stale upstream.

**That table is now partly out of date, in our favour**, and it is the table
rather than reality that a reader will trust: the 2026-09-07 push closed its
`guidelines/` row and its `SKILL.md` row (the latter needed nothing — it states
no colour or border rule). Its seven `components/**` rows, `ui_kits/tui/` and
`templates/harness-session/` are all still open. `SYNC.md` is a verbatim mirror
of the bound copy's own record, so it is annotated rather than edited — see the
note under its Turn 13 table.

Two consequences worth remembering:

- `DesignSync method:"list_projects"` filters to **design-system projects
  only**, so the discussion project never appears in it. Address it by UUID.
- The design-system project's `updatedAt` stays at `2026-09-02` no matter how
  much work happens in the bound copy, because editing a bound snapshot does not
  touch the source project. A stale-looking timestamp there is not evidence that
  nothing changed.

`.claude/CLAUDE.md` currently describes these as two peer projects and names the
design-system one as the token authority. That is now backwards; see the
"Superseded" notes below.

## What is in this directory

| File | Source path in `25845063-…` |
| --- | --- |
| `HANDOFF.md` | `README.md` |
| `SYNC.md` | `_ds/mjolnir-design-system-4ea574fb-…/SYNC.md` |
| `tokens/cells.css` | `_ds/mjolnir-design-system-4ea574fb-…/tokens/cells.css` |
| `tokens/palette.css` | `_ds/mjolnir-design-system-4ea574fb-…/tokens/palette.css` |
| `tokens/semantic.css` | `_ds/mjolnir-design-system-4ea574fb-…/tokens/semantic.css` |

All five are verbatim copies **except** `HANDOFF.md`, which carries added
block-quote notes (on the grid table, on first run, and on the top bar's gap)
marking sections a later turn superseded, and whose `### Glyphs` table was
**rewritten from measurements** in the 2026-09-21 repaint — see that section
above for why that one had to stop being a mirror. Those notes are annotations
by the importing session, not upstream text. Everything else in that file is as
fetched, which by now means: largely describing frames that no longer exist.

### Not imported

- `Agent TUI v2.dc.html`, `Agent TUI v2 Light.dc.html`, `Agent TUI.dc.html` —
  the authoritative frames. Large; `DesignSync method:"get_file"` is the only
  read path and it round-trips content through the model context, so these were
  left for a targeted fetch rather than a blind copy. **They remain the only
  place cell positions exist** (CLAUDE.md's "measure the handoff HTML" rule).

  `Agent TUI v2.dc.html` was fetched once, later on 2026-09-06, because `5d`
  had changed upstream: first run's opening step is now `provider`, and the
  `model` step is gone. It was measured, not copied — the frame's own
  paragraph in `HANDOFF.md` carries what came out of it, and nothing new
  landed in `tokens/`. Every landmark it needed (`--label-col`,
  `--option-label-col`, `--section-gap-h`) already existed; the frame's
  update was content, not grid. **The design's own `updatedAt` is no guide to
  whether this happened** — see the note above on why.
- `tokens/{base,elevation,fonts,motion,typography}.css`, `styles.css`,
  `_ds_bundle.js`, `_ds_manifest.json`, `_adherence.oxlintrc.json`.
- `_ds/…/README.md` and `_ds/…/readme.md` — the design system's *own* guideline
  prose, distinct from the top-level `README.md` this directory keeps as
  `HANDOFF.md`. Turn 15 rewrote them (the light-theme paragraphs, the syntax
  ramp's two anti-rainbow rules, the quoted-code surface). Fetch them if a
  question is about a rule rather than a value; `SYNC.md` carries the summary.
- `screenshots/` (33 PNGs), `uploads/` (16 PNGs + a zip).

## Where this contradicts the working tree

Read `SYNC.md` in full before touching `crates/tui`.

### Still open

1. **Two components the system has not built either.** Turn 15's `15a` (inline
   code in prose) and `15b` (a multi-line composer draft) are patterns in the
   design files with no component behind them — `SYNC.md` says a `CodeBlock`
   and a `rows` prop on `Composer` are what would carry them. The tokens exist,
   so either can be implemented here without waiting for a palette decision.

### Closed since this file was first written

The three that used to head this section — the 12→8 cell label column, the
removal of every border in favour of the ground ladder, and the OKLCH palette
with its seven new `--tui-*` roles — are all applied. `grid.rs` has
`LABEL_COL_WIDTH = 8` with `CONTENT_INDENT` derived, `render_snapshot.rs`
asserts `every_scene_parts_its_bands_by_tone_and_draws_no_rules`, and
`palette.rs` mirrors the current role set. Kept here as a record of what the
prose in `HANDOFF.md` still says, which is the old numbers: `tokens/cells.css`
is the authority, not that document's grid table.

Also applied, same direction: the top bar carries no `▌` and shows `aldwin`
rather than the project name; `▌` means selection or caret only; permission is
a bottom-anchored full-width panel with numbered options `1`–`4`, not a centred
modal; within-group facts are parted by ` · `, and the 6-cell gap survives only
between the brand and everything else.
