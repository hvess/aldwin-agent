# Sync record

Source of the decisions: `Agent TUI v2.dc.html` and `Agent TUI v2 Light.dc.html`
in the consuming project. Everything below was applied to this bound copy.

# Turn 14 — a three-step first run, and a light theme that ladders

The frames moved first and the token layer followed, which is the direction
that leaves this copy able to disagree with them: both files state their whole
palette inline as `--t-*` and read nothing from `semantic.css`. Re-measure
against that block, never against the prose here.

## Applied

**`tokens/cells.css`** — added the first-run step spine: `--step-mark-col`
(`--label-col` + `--label-gutter`, 10 cells) and `--step-content-col`
(`--margin-x` + `--step-mark-col` + `--option-label-col`, cell 29). Both
derived, neither restated. `--option-label-col`'s comment now names the
provider list rather than the model list, since the provider list is the one
that sets the width.

**`tokens/semantic.css`** — the `.tui-light` scope was regenerated whole, and
one role was added to both scopes.

*The light ground ladder is now monotonic.* Every band sinks below the frame
ground, in seven ordered steps:

| Role | Was | Now |
| --- | --- | --- |
| `--tui-ground` | `#fbf9fe` | `#faf7ff` |
| `--tui-break` | `#f0edf5` | `#ede9f6` |
| `--tui-bar-bottom` | `#cec9d8` | `#e0dbea` |
| `--tui-bar` | `#e4e1eb` | `#d3cedd` |
| `--tui-recess` | `#bbb6c5` | `#c6c1d1` |
| `--tui-panel-title` | `#cec6df` | `#bab3c8` |
| `--tui-scrim` | `#aeaab7` | `#a39fac` |

Two of those are corrections rather than adjustments. **`--tui-bar` and
`--tui-bar-bottom` were the wrong way round**: the top bar sat *lighter* than
the composer, which is the reverse of the dark theme's arrangement, so the two
chrome bands read as swapped between themes. And **the turn break no longer
rises** — it was the one band that stepped above the light ground, on the
argument that a separator has to stay visible; it does not need to, because
`#ede9f6` is a full step below `#faf7ff` and nothing else on the screen is.

*The ink got darker.* `--tui-text` `#17151c` → `#0e0c12`, `--tui-body`
`#322e39` → `#1e1b23`, `--tui-code` `#221f28` → `#151219`, `--tui-quiet` and
`--tui-value` `#433f4b` → `#2a282e`, `--tui-label` `#524d5b` → `#39373d`,
`--tui-dim` and `--tui-context` `#615c6c` → `#48464d`.

*The light accents left the ramp.* They now sit past `--color-accent-900`
(`#562c8b`), which the theme had outgrown, so `.tui-light` states them as
literals: `--tui-mark` and `--tui-accent-text` `#4b1f7e`, `--tui-speaker-you`
`#592f8e`, `--tui-gauge-fill` and `--tui-hunk-header` `#7d56b8`,
`--tui-glyph-running` `#4b1f7e`, `--tui-mark-idle` and `--tui-glyph-pending`
`#706c79`, `--tui-gauge-track` `#6a6773`, `--tui-reverse-bg` `#4b1f7e` on
`--tui-reverse-ink` `#faf7ff`. `--color-band-light` moved `#c6b1ef` →
`#c4acf2` and stays the one light value drawn from `palette.css`, because that
token exists for this one fill. The diff pair was re-picked too:
`--tui-add` `#006911` / `--tui-add-code` `#004300` / `--tui-add-row`
`#c9e3c9`, `--tui-del` `#9e1421` / `--tui-del-code` `#6c0003` /
`--tui-del-row` `#facfcb`, on `--tui-diff-box` `#e6e2ee`.

*New role: `--tui-step-done`*, a settled first-run step's `●`. In the dark
theme it is `--color-accent-700`, exactly what `--tui-glyph-done` is, which is
why it looks like a token that did not need to exist. It is: **the two part
company in the light theme**. A finished tool call recedes by going *lighter*
than the accent mark (`#a17adf`), because it sits in a dense run of tool rows
and should fall back; a settled step recedes by going *darker* (`#6941a1`),
because it sits beside an answer that has to stay readable. Spelling both as
one token would force one of those two to be wrong.

*Also*: `--tui-glyph-done` in light was `--color-accent-900` and is now
`#a17adf`, per the above.

**`tokens/palette.css`** — `--color-band-light` re-pointed, and the accent
ramp's comment corrected: it claimed the light theme draws its marks from
800/900, which stopped being true here. The ramp is not extended downward to
cover the new light accents deliberately; those steps are what the *dark*
theme is generated from, and rungs nothing in the dark theme uses would make
the ramp a place to look up rather than a thing to read.

## Not applied — the frames' own inline palette

Neither `.dc.html` file reads `semantic.css`; each restates the whole palette
in a `:root{--t-*}` block. Nothing above changes what either frame renders,
which is the point — this copy was brought up to what they already show. The
consequence is that the two can drift again silently, and only a measurement
catches it.

# Turn 13 decisions

## Applied

**`tokens/palette.css`** — regenerated. One hue (300°) carries neutrals at low
chroma and the accent at full chroma. Added the seven-step ground ladder
`--color-ground-0…6`, the `--color-band-*` selection fills, the inline-diff
ground and the diff row fills. Removed the Nocturne neutrals, the teal/azure
accents and `--color-divider`.

**`tokens/semantic.css`** — added `--tui-recess`, `--tui-break`,
`--tui-panel-title`, `--tui-add-row`, `--tui-del-row`, `--tui-reverse-bg`,
`--tui-reverse-ink`. Removed `--tui-modal-line`. Both themes re-pointed; the
light theme's frame ground is now the lightest surface and every other band
sinks below it, except the turn break, which rises.

**`tokens/cells.css`** — rewritten twice. First: label column 12 → **8 cells**,
body column 17 → **13**; added `--option-label-col` (16 cells), `--break-h`,
`--wordmark-h`, `--section-gap-h`, `--panel-permission-h`; removed the modal
x/y/width positions; `--bar-bottom-h` corrected 101 → 100 (not a cell multiple).

Then the grid rework: the file now holds only two primitives (`--cell-w`,
`--cell-h`) and derives every landmark from them with `calc()`, so nothing is
restated as a literal and widening the line-number gutter moves the diff code
column with it. Added `--row` (the unit of vertical spacing), `--stdout-indent`,
`--option-detail-col`, `--hint-label-col`, and `--diff-sign-col` /
`--diff-code-col` — the last two replacing an inline `calc()` that had been
restated in three places, which also stopped the diff sign offset being spelled
as `--label-gutter`, a token with nothing to do with diffs. Dropped `--body-col`
and the three `--body-h-*` values: nothing applied any of them. Frames are now
`display: grid` with an explicit `grid-template-rows` per screen, so the bands sum
to 36 rows by construction; both design files were converted, all 282 px literals
inside the frames replaced with tokens, the 72 spacer divs collapsed, and the
command list's option field brought back from 14 cells to the system's 16.

**`tokens/elevation.css`** — dropped the hairline edges from the shadows and
removed `--rule-fade` / `--rule-fade-narrow`. Added `--frame-bezel`.

**`_ds_bundle.js`** — removed every `1px solid` stroke inside a frame
(`TerminalFrame`, `TopBar`, `BottomBar`, `Modal`, the inline diff, both pane
dividers). `TurnBreak` is now a sunk one-row band instead of a fading rule.
`Modal` is a bottom-anchored full-width panel, not a centred bordered box.
`Turn`, `StepRow` and `MetaRow` moved to the 8-cell label column. `BarIdentity` lost its
leading `▌`. **New `Wordmark` component.** `FirstRunScreen` rebuilt: wordmark,
positioning line, two steps, four-point access scale, no account step.
`FadingRule` and `Scrim` both kept but documented as retired — nothing inside a
frame is stroked, and the overlay is a single dimmed layer with no scrim. The
glyph table no longer lists "session identity" as a use of `▌`.

**`README.md` / `readme.md`** — Colour and Borders rewritten; new **Brand mark**
and **First run** sections; States gained the shared option-row pattern; the
Turn 13 removals listed under Deliberately absent; `Wordmark` and `TurnBreak`
added to the index and inventory.

**`_ds_manifest.json`** — the token table was regenerated by re-parsing the four
rewritten CSS files, so every recorded name, value, kind and scope now matches
what those files actually define (145 → 177 entries). `--color-divider`,
`--tui-modal-line`, `--rule-fade` and `--rule-fade-narrow` are gone; the ground
ladder, band fills, reverse-video roles and new cell metrics are in. `Wordmark`
registered. Guideline-card metadata re-worded where it stated superseded rules:
the accent, grounds and column-position cards, the frame component card, and the
fading-rule card (now marked retired).

**`_adherence.oxlintrc.json`** — `--tui-modal-line` pruned; allowlist and kind
map reconciled against the regenerated token table.

**`_ds_bundle.js` — grid pass.** The components had kept the grid as baked
arithmetic while the design files moved onto tokens, so the same "tokens are
dead" defect survived one layer down. Now fixed: 18 `height: 20`, 16
`padding: '0 27px'`, the two line-number gutters, the label column, both pane
widths and the key-hint group gap all read from `cells.css`. Six genuine bugs
came out of that sweep:

- `BottomBar` and `CommandPanel` sized themselves `rows * 20 + 1` — a leftover
  compensating for the 1px borders that were removed earlier, so an 11-row panel
  rendered 221px and pushed every band below it half a pixel off.
- `DiffHunk`'s "N more lines" trailer was inset 9 cells when the code column is
  11, so the trailer did not line up with the code it described.
- `CommandBlock` used `height: 10` — half a row — above and below the command,
  and an off-grid `paddingLeft: 16`. Both now whole cells; its doc comment no
  longer claims half a row.
- `SessionTranscript` used the same half-rows between tool lines.
- `ReviewScreen` had a `height: 19` spacer, the other half of a deleted
  `19px + 1px rule` pair.
- `CommandPanel` painted its title row with `--tui-band`, the selection colour,
  instead of `--tui-panel-title`; and its option field was still 14 cells.

`CommandPanel` now declares `grid-template-rows: var(--row) 1fr` like the frame
does, and `TerminalFrame` takes a `bands` prop. Verified by mounting the session
and commands frames: both exactly 1080×720, bands summing to 720, and **zero
off-grid heights** anywhere in the tree.

## Known, not yet decided

The line-number gutter leaves a 2-cell gap before the sign in the review pane and
a 1-cell gap in the inline transcript diff. Both are on-grid and neither reads as
wrong, so it is recorded rather than changed.

## Not applied — outside this copy

These live in the design-system project and are unreachable from here. They still
carry the old rules:

| Path | What needs doing |
| --- | --- |
| `components/frame/TopBar.jsx` | drop `BarIdentity`'s `▌`; add the `Wordmark` component |
| `components/frame/FadingRule.jsx` | rewrite `TurnBreak` as a sunk row; mark `FadingRule` retired |
| `components/frame/TerminalFrame.jsx`, `BottomBar.jsx` | remove the `--tui-line` strokes; add the bezel |
| `components/overlays/Modal.jsx` | bottom-anchored panel, no border, risen title row |
| `components/overlays/OptionRow.jsx` | update the doc comment — the band is a solid fill now |
| `components/transcript/Turn.jsx` | `labelCells` 12 → 8 |
| `components/transcript/InlineDiff.jsx`, `components/review/*` | grounds instead of borders |
| `guidelines/` (18 cards) | the colour, grid, brand and glyph cards all show superseded values |
| `ui_kits/tui/` (5 screens, dark + light) | first-run rebuild, permission panel, no rules |
| `templates/harness-session/` | strokes and label column |
| `SKILL.md` | whatever restates the colour or border rules |

## Pre-existing bug, unrelated to this sync

Every screen in `ui_kits/tui/` fails to render (React #130). Each destructures
`window.MjolnirDesignSystem_4ea574` at module-eval time, but the bundle writes
`__ds_ns.X = __ds_scope.X` only at the very end, after those screens have
already run — so every component reference in them is `undefined`. The
components themselves are fine; only the demo screens are affected. Fixing it
means either exporting into the namespace per-component-file or having the
screens resolve the namespace inside their render.
