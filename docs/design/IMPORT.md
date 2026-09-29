# Design import — provenance

## Re-sync — 2026-09-29, code has its own ink

`Aldwin Agent TUI.dc.html` was fetched again from the discussion project
and replaces the local copy verbatim (89 KB). The README and
`tokens/colors.css` in the design-system project and in the bound `_ds/`
copy were fetched and are unchanged.

What the frame changed:

- **`--code`**, a teal (`oklch(0.8 0.085 212)` dark, `oklch(0.5 0.1 218)`
  light), set by the frame's script with `setProperty` and declared in no
  token file (baseline `code-ink-is-the-frames-not-colors-css`). The
  frame draws in it every name of code: `limit.rs` and `settings.toml` in
  prose, a detail row's target, the working line's target (the script
  marks where it begins, `["Reading router.rs", 8]`, and the highlight
  passes over it) and `router.rs` in the comment field's label. The
  first fetch also drew `config` and `100` in frame H's draft; the
  developer took that out the same day ("we won't support rich text in
  input fields"), and the second fetch, imported here, draws the draft
  in `label`.
- **The canvas lost its theme button and its accent picker**; the theme
  is a prop again. Neither is drawn inside a window.

The generator reads `--code` from the frame (`frame_roles`) until
`colors.css` declares it.

## Re-sync — 2026-09-29, the working line

`Aldwin Agent TUI.dc.html` was fetched from the discussion project and
replaces the local copy verbatim (92 KB). The README, all five token files
and `glyphs.html` in the design-system project were fetched and match this
directory; the design system's `StatusBar.jsx` and `ContextBar.jsx` still
draw the old footer, and the README still says "Nothing else animates".
The frame outranks them, as it did `⎋` on 2026-09-25; baseline
`footer-is-the-frames-not-the-components` records it.

What the frame changed:

- **A new section, "Aldwin · working"**: `W1`, frame B with its footer
  animated by the page's script, and `W2`, that animation as rows 100ms
  apart. The status line says what Aldwin is doing in a few plain words;
  a new phrase types in three characters a frame, then holds while a
  highlight runs along it (`--label`, then `--label` over `--label2` at
  70% and 40%); the running `●` blinks to `○` every half second in amber;
  the timer follows in `label3`; after 30s with nothing new the `●` sits
  as a grey `○` and the phrase starts with "Still". Under
  `prefers-reduced-motion` the script draws nothing new.
- **Every footer opens with a state glyph** in the mark column: amber `●`
  while working, `label3` `○` otherwise. Frames B–D's footer is the
  working line (`Writing the limiter in limit.rs  1m 02s`) with no key.
- **Key hints lose a space**: `↑↓ Choose`, `esc Close`, `? Keys`. The
  working section says "esc always stops, and the launch card says so
  once", but frame A's card names no key; the app follows frame A
  (baseline `launch-card-names-no-stop-key`).
- **The context bar loses its label**, and its segments are `█`, not `━`.
- **Frame F** types `/` and lists `clear` and `exit`, `clear` completed in
  the field; baseline `frame-command-list-is-not-the-products` updated.

The generator reads two new things from the frame: the bar's glyph
(`GAUGE_CELL`) and the highlight's mixes (`HIGHLIGHT_*`). `…` is no
longer drawn by any frame, so baseline `ellipsis-marks-shortened-text`
licenses it.

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
| `frames/Aldwin Agent TUI.dc.html` | `25845063-…/Aldwin Agent TUI.dc.html`, verbatim, 89 KB since the second 2026-09-29 re-sync |

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
  are summarised in `docs/spec/aldwin-tui.md`'s 2026-09-23 entry; the
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
2026-09-23 and 2026-09-25); `/changes` and `/undo` will not be built
(2026-09-27). The frames'
placeholders and frame J's `↺ Undo`, which the import recorded against the
product, are gone from the frame.

## Traps that survived the replacement

- `list_projects` returns design-system projects only. The discussion
  project — where the frame lives — is addressed by UUID.
- `DesignSync` is main-session only; subagents do not have it.
- Authenticate with `/design-login` before the first call.
- A stale `updatedAt` proves nothing.
