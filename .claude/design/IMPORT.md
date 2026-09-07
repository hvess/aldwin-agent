# Design import — provenance

Imported 2026-09-06 via the `DesignSync` tool (claude.ai/design, MCP endpoint
`https://api.anthropic.com/v1/design/mcp`, authenticated with `/design-login`).

Re-synced later the same day. That pass changed exactly two files —
`tokens/cells.css` (rewritten) and the `cells.css` paragraph of `SYNC.md`.
`README.md`, `tokens/palette.css` and `tokens/semantic.css` came back
byte-identical, and no path was added or removed in either project. The
design-system project's `updatedAt` did not move; see the note below on why that
is expected.

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

## The two projects, and which one is live

There are two distinct objects on claude.ai/design, and they are easy to
confuse because one is embedded in the other.

| | Design system | Discussion project |
| --- | --- | --- |
| UUID | `4ea574fb-4be4-47de-9940-fd38927d6dd8` | `25845063-2993-4020-ae58-4e7defc6bfef` |
| Name | Mjolnir Design System | Design system tokens discussion |
| `type` | `PROJECT_TYPE_DESIGN_SYSTEM` | `PROJECT_TYPE_PROJECT` |
| Holds | tokens, 18 guideline cards, components, UI kits | the five `.dc.html` frames + a **bound copy** of the design system under `_ds/mjolnir-design-system-4ea574fb-…/` |

**The bound copy inside the discussion project is the current one.** Turn 13
decisions were applied there and *not* propagated back to the design-system
project — `SYNC.md` carries an explicit "Not applied — outside this copy" table
listing what is still stale upstream (all 18 guideline cards, every component,
the UI kits, `templates/`, `SKILL.md`).

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

All five are verbatim copies **except** `HANDOFF.md`, which carries two added
block-quote notes (on the grid table and on first run) marking sections that
Turn 13 superseded. Those two notes are annotations by the importing session,
not upstream text. Everything else in that file is as fetched.

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
- `screenshots/` (33 PNGs), `uploads/` (16 PNGs + a zip).

## Where this contradicts the working tree

Read `SYNC.md` in full before touching `crates/tui`. The three that bite:

1. **Grid.** Label column 12 → **8 cells**; body column 17 → **13**.
   `crates/tui/src/ui/grid.rs:31` has `LABEL_COL_WIDTH = 12` (and a doc comment
   citing `--label-col: 108px`, now `72px`), so `CONTENT_INDENT` resolves to 17
   where the system now says 13. `.claude/spec/mjolnir-tui.md`'s 2026-09-03 grid
   entry is on the same old numbers. `HANDOFF.md`'s own grid table also still
   says 12/17 — `tokens/cells.css` is the authority, not the prose.

   As of the re-sync there is **no `--body-col` token**, deliberately. Cell 13 is
   a consequence of `--margin-x` + `--label-col` + `--label-gutter`, and the file
   carries a standing instruction not to add a fourth statement of it. The Rust
   side already models it that way — `CONTENT_INDENT` is derived from the same
   three constants at `grid.rs:33`, so only `LABEL_COL_WIDTH` needs to change.

   New in the same pass: `--diff-sign-col` (2 cells) and `--diff-code-col`
   (`--margin-x` + `--gutter-line-no` + `--diff-sign-col` = **cell 11**), which
   the review pane's "N more lines" trailer hangs on. The three `--body-h-*`
   row-budget values were dropped — a frame's body band is `1fr` and is never
   given a computed height. `cells.css` now also states the rule that every
   token in it must be applied through a `var()` somewhere, or deleted.
2. **Borders.** There are none, anywhere inside a frame. Every boundary is a
   step on the seven-rung ground ladder (`--color-ground-0…6`). Commits
   `439808b`, `be7f1fa` and `47104cd` are all border/underline work and are
   pointed the wrong way.
3. **Palette.** Regenerated in OKLCH on **one hue, 300°** — neutrals at low
   chroma, accent at full chroma, accent `#be9df7`. The Nocturne indigo, harbor
   teal `#30b5aa` and dusty azure `#84aed9` are all retired; `palette.css` says
   do not reintroduce them. `crates/tui/src/palette.rs` mirrors `--tui-*`
   one-to-one, so it needs the seven new roles: `--tui-recess`, `--tui-break`,
   `--tui-panel-title`, `--tui-add-row`, `--tui-del-row`, `--tui-reverse-bg`,
   `--tui-reverse-ink`; and `--tui-modal-line` is gone.

Smaller, same direction: the top bar carries no `▌` and shows `mjolnir` rather
than the project name; `▌` now means selection or caret only; permission is a
bottom-anchored full-width panel with numbered options `1`–`4`, not a centred
modal; within-group facts are parted by ` · `, and the 6-cell gap survives only
between the brand and everything else.
