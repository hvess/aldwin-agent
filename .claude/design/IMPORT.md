# Design import — provenance

## Re-sync — 2026-09-25, the frame only

`Aldwin Agent TUI.dc.html` was fetched from the discussion project and
replaces the local copy verbatim. Nothing else moved: the README, all five
token files and `glyphs.html` were fetched and match this directory byte
for byte. The frame's stylesheet link now points at the bound copy,
`_ds/aldwin-b9de8837-…/styles.css`, rather than `Aldwin Design System/`.

What the frame changed:

- **The caret** is a 2px `--accent` bar between cells
  (`width:2px;margin-right:-2px`), not a `label` block.
- **No placeholders.** Every empty field is the `›` and the caret.
- **`esc`, not `⎋`.** Every footer (`esc  Stop`, `esc  Close`) and frame
  D's caption. The README and `glyphs.html` still list `⎋` as "stop,
  close"; the frame no longer draws it.
- **Frame C** drops `Space  Hide Details`: its footer is `esc  Stop`, as
  B's is.
- **Frame F** is rebuilt: the commands on a `--panel` band directly on the
  field (`padding:24px 0`, rows `margin:0 1ch`), the current row on
  `--field`; names without a slash, the typed part `label` and the rest
  `label2`; the field shows `/c`, the caret and the top match's rest in
  `label3` ("The text turns blue only once it spells a real command"); the
  footer is `↑↓  Choose  ↩  Run  esc  Close`, with no status word and no
  shortcut column.
- **Frame J** drops `↺  Undo`; its footer is the context bar alone.

`tokens.rs` regenerates unchanged: the glyph table still comes from
`glyphs.html`, and the new frame draws no codepoint the old one did not.

## The Aldwin Design System — 2026-09-23, a replacement

The design system was **replaced, not revised**. Its own README says so:
"It replaces the Mjolnir design system … None of Mjolnir's values carry
over." Every file this directory held before this date is gone; the
history of the Mjolnir era (three token files, `HANDOFF.md`, `SYNC.md`,
seven syncs between 2026-09-06 and 2026-09-21) is in git under the
previous revision of this file.

Read with `DesignSync` after `/design-login`, from the two projects below.

### Where it lives now

| | Design system | Discussion project |
| --- | --- | --- |
| UUID | `b9de8837-c1b6-4be1-8668-4dee1c585de5` | `25845063-2993-4020-ae58-4e7defc6bfef` |
| Name | Aldwin | Design system tokens discussion |
| `type` | `PROJECT_TYPE_DESIGN_SYSTEM` | `PROJECT_TYPE_PROJECT` |
| Holds | tokens, 12 guideline cards, 16 components, one UI kit | `Aldwin Agent TUI.dc.html` (the ten frames), a top-level copy of the design system under `Aldwin Design System/`, and a bound copy under `_ds/aldwin-b9de8837-…/` |

The Mjolnir project `4ea574fb-…` no longer appears in `list_projects`.
The old frames survive only under `uploads/AI agent harness TUI design/`
in the discussion project.

**The frame is the authority, and the three copies of the system agree.**
The README states every token value is copied from `Aldwin Agent TUI.dc.html`.
`tokens/*.css` in the design-system project, in `Aldwin Design System/`
and in `_ds/…/` were fetched and are byte-identical.

### What is in this directory

| File | Source |
| --- | --- |
| `README.md` | `b9de8837-…/README.md`, verbatim |
| `tokens/colors.css` | `b9de8837-…/tokens/colors.css`, verbatim |
| `tokens/layout.css` | `b9de8837-…/tokens/layout.css`, verbatim |
| `tokens/typography.css` | verbatim |
| `tokens/elevation.css` | verbatim |
| `tokens/motion.css` | verbatim |
| `guidelines/glyphs.html` | `b9de8837-…/guidelines/glyphs.html`, verbatim — **the generator's glyph source** (see below) |
| `frames/Aldwin Agent TUI.dc.html` | `25845063-…/Aldwin Agent TUI.dc.html`, verbatim, 71 KB |

The frame is imported this time. Under Mjolnir the frames were "left for a
targeted fetch" because they were large and restated their palette inline;
this one is 71 KB, reads its tokens through `styles.css`, and is the only
place two things exist at all: the brand mark's 108 cell colours, and the
per-row positions of every component. The generator reads it.

### Not imported

- `tokens/fonts.css`, `tokens/base.css`, `styles.css` — a Google Fonts
  import and canvas body defaults; nothing a terminal consumes.
- `components/**` (`.jsx`, `.d.ts`, `.prompt.md`) and `_ds_bundle.js` — the
  React components. Their `.prompt.md` files were read during the import and
  are summarised in `.claude/spec/aldwin-tui.md`'s 2026-09-23 entry; the
  frame carries the same positions and colours and is what the app is
  measured against.
- `guidelines/*.html` other than `glyphs.html` — specimen cards restating
  the tokens. `layout-columns.html` was fetched and checked against the
  frame; it agrees.
- `ui_kits/tui/index.html`, `screenshots/`, `uploads/`.

### How the app is generated from it

`crates/review/src/tokens.rs` reads `tokens/colors.css`, `tokens/layout.css`,
`guidelines/glyphs.html` and the frame, and emits `crates/tui/src/tokens.rs`:

- **Colours** are `oklch()` literals (the light theme mixes in six hexes).
  The generator converts OKLCH to sRGB itself — there is no ramp indirection
  to resolve any more, and no hex table to look values up in. The canvas
  roles (`--canvas-*`) are documentation and are not carried; `--chrome` and
  `--dot` are the mock's title bar and are not carried either — a terminal
  has no title bar to draw.
- **The grid** is `ch` and `px` where `1ch` is a cell and `--row: 24px` is a
  row. Only the tokens the app consumes are emitted.
- **The glyph table** is parsed from `guidelines/glyphs.html`, the one
  machine-readable copy: the README carries the same fourteen marks as a
  prose sentence. Every codepoint in the frame was counted and checked
  against it; the frame also draws `↑↓`, `⌄` and `▀`-style half blocks,
  which the generator adds from the frame itself.
- **The brand mark** is 18×6 `<span>`s, each a `linear-gradient(top 50%,
  bottom 50%)` of `color-mix(in oklch, var(--fill) N%, var(--win))`. That
  is exactly a `▀` cell with an independent foreground and background, so
  the generator emits a `MARK` matrix of per-cell (top, bottom) mixes and
  the app draws it with 108 half-block cells.
- **The context bar** ramps its filled segments through `color-mix` at 55,
  70, 85 and 100 % of `--fill` over `--track`; those four mixes are emitted
  as `GAUGE_RAMP`.

### Where the design and the product disagree

Recorded in `crates/review/baseline.json` under `contradictions`. As of the
2026-09-25 re-sync the one about scope is the command list: the design's
commands include `/changes` and `/undo` (frame F, `CommandRow`), where the
product ships `/resume /model /quit /clear /theme` (the developer's calls,
2026-09-23 and 2026-09-25); both are open-tasks entry 27. The frames'
placeholders and frame J's `↺ Undo`, which the import recorded against the
product, are gone from the frame.

## Traps that survived the replacement

- `list_projects` returns design-system projects only. The discussion
  project — where the frame lives — is addressed by UUID.
- `DesignSync` is main-session only; subagents do not have it.
- Authenticate with `/design-login` before the first call.
- A stale `updatedAt` proves nothing.
