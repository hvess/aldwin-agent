# Handoff: Agent harness TUI

## Overview

A terminal user interface for an LLM agent harness (the same product category as Claude Code or OpenCode). The design goal is a TUI that reads like a polished desktop application while staying inside what a cell-grid terminal framework such as [ratatui](https://ratatui.rs) can actually draw: one glyph per cell, one-cell-thick borders, block characters for shading, truecolor foreground/background per cell.

Five states are designed: the live session, a permission prompt, diff review, the command list, and first run. Both a dark and a light theme are provided.

## About the design files

The files in this bundle are **design references created in HTML** — prototypes showing intended look, layout and content, not production code to copy. The implementation target is a Rust TUI (ratatui/crossterm assumed) or an equivalent terminal framework. Recreate the layouts using that framework's own primitives (`Layout`, `Block`, `Paragraph`, `List`, `Table`, `Span`/`Line` styling); do not attempt to port HTML/CSS.

**There are no borders in these frames.** Every boundary is carried by ground colour: bands sit at different tones and the step between them is the separation. Nothing is stroked, so nothing needs a `Block::bordered()` — build each band as a rect with its own `Style::bg` and let the tonal step do the work. Rules, pane dividers, the inline diff box outline and the permission panel's accent edge are all gone; the outer window keeps its radius in the mock as a tonal edge only, drawn as a 2px band of the chrome tone rather than a stroke.

The tonal order, ground outward: recessed fields sit lowest, then the composer band, the turn-break rows and the review file pane, then the transcript ground, then the chrome bars highest (inverted in the light theme, where every chrome band steps *darker* than the near-white ground). A boundary is legible when consecutive bands are at least one full step apart; two adjacent bands never share a tone.

## Design system

These frames are now bound to the **Mjolnir Design System**, not the Nocturne system they were drawn on. Note that the borderless rebuild below re-ramps the grounds, neutrals and accent locally in each file — see "Borderless rebuild" — so the frames no longer read the system's colour tokens directly, though the hue families and the cell grid are unchanged. Nocturne's grounds and neutrals survive unchanged inside the system's palette, so the rebind moved one thing visually: the accent is the system's own and its OKLCH-regenerated ramp, where it was Nocturne's violet. (The accent has since been re-picked in the system twice; the current value is dusty azure `#84aed9` — see Token resync.) Every accent step maps one-to-one by name, so no accent decision in these screens was re-made.

What changed in the files:

- Each file loads the design system's token layer and bundle from `_ds/mjolnir-design-system-4ea574fb-…/`. The Nocturne link and the hand-rolled Google Fonts links are gone — the design system's `styles.css` imports JetBrains Mono and Inter itself.
- Hardcoded hex is now tokens: the diff pair reads `--tui-add` / `--tui-del` / `--tui-add-bg`, the desk `--color-desk`, the bottom bar `--tui-bar-bottom`, the inline diff box `--tui-diff-box`.
- Type and grid read tokens too: `--text-cell` for the 15/20 cell style, `--font-mono`, `--font-ui` for annotation, `--frame-w` / `--frame-h` for the 1080 × 720 window, `--radius-frame` for its one rounded corner.
- The light file carries `class="tui-light"` on its root, so the semantic roles resolve to Aldwin's light values — including the selection band being *darker* than the ground, and the light desk `#c9cee4`.
- In the first exploration (`1a`–`1c`) the amber `#c9a97e` used for the running spinner and the `M` file marker had no equivalent in Aldwin and is now `--tui-glyph-running`, which is what the glyph table specifies for `◐`.

## Token resync

The bound snapshot in `_ds/` has been refreshed from the design system twice. No frame edit was needed for anything colour-valued, since every colour in these files reads a token.

### Second resync — the accent moved (since reversed)

The accent went from harbor teal `#30b5aa` to **dusty azure `#84aed9`**, ramp regenerated in OKLCH at the azure hue on the same lightness scale. Every accent surface in all five screens followed on reload: the `▌` marks, the `you` speaker label, the caret and composer prompt, the gauge fill, the `@@` hunk header, the modal border, the selection band and the running spinner. The diff green also nudged to `#70cf75` (light `#0a7520`), which widens the gap to the accent — the system now notes added at hue 145 sitting 104 degrees clear of the accent at 250, where under the teal accent the two were separated mostly by chroma.

Worth noting for the light theme: azure's accent-600 `#567ea7` is a lighter, lower-contrast mark than teal's `#008f85` was, so light-mode `▌` marks and the gauge fill read softer against the `#f3f5fe` ground than they did before.

### First resync — the diff pair and two new tokens

- **The diff pair was re-picked.** Added moved off the handoff's sage to a yellow-green on a `.13` tint, removed to a warm red `#e86c68` on `.14`, both at roughly double the chroma. The system's note is that added now sits at hue 145, clear of the teal accent at 187, so a change never reads as an accent mark — the two are separated by hue as well as chroma. Light theme goes darker and stronger instead. These arrived through `--tui-add` / `--tui-del` and their `-bg` tints with no edits to the frames.
- **New tokens `--tui-add-code` / `--tui-del-code`** give the code text on a diff row its own colour, tinted toward the sign instead of staying neutral. This one needed adopting by hand: the 11 added rows in the review pane (`5b`) and the 3 in the transcript's inline diff (`4a`) now read `--tui-add-code` where they read `--tui-code` before. No removed rows are visible in either screen, so `--tui-del-code` is unused for now.

## Borderless rebuild

Two requirements arrived after the token resync: **no borders anywhere**, and **more contrast** — the frames were too dark for their text ramp to stay legible. Both were applied to all five screens in both themes.

### What replaced each border

| Was | Now |
| --- | --- |
| 1px rule under the top bar, above the bottom bar | nothing — the bars are their own ground, a step off the transcript |
| Flat turn separators in the transcript | one full-width row of the composer's tone |
| Step separators in first run | the same full-width tone row |
| Vertical 1px rule between the review panes | the file pane drops to the composer tone; the diff pane stays on the transcript ground |
| Vertical rule inside the command panel | the explain pane drops to the recessed tone; the list stays on the panel tone |
| Rule above the permission options | one row of the recessed tone |
| Accent-700 rule along the permission and command panel top edge | the panel's own ground step, with the accent-tinted title band doing the announcing |
| Inline diff box outline in the transcript | a recessed field, no outline |
| 1px outline on the terminal window | a 2px band of the chrome tone around the radius — a tonal edge, mock chrome only |

### The colour logic

**The palette is generated, not picked.** Every value in both themes comes out of one OKLCH specification — a lightness, a chroma and a hue per role — rather than being chosen individually. That is the fix for the fault the hand-tuned versions kept running into: when each value is solved on its own for contrast, the hues drift apart, and a frame of blue-grey grounds under a violet accent with slightly green greys reads as disjointed no matter how correct each pair measures.

Three rules define the system:

1. **One hue, 300°, for everything.** The neutrals and the accent are the same hue. The neutrals carry it at a chroma of 0.010–0.036, low enough to read as grey but high enough that they are visibly kin to the accent; the accent carries it at 0.058–0.130. Nothing in a frame is a foreign colour. The only exceptions are the two diff hues, 148° and 25°, which have to be unmistakably not-the-accent.
2. **Lightness climbs in even perceptual steps.** The bands run L 0.160 → 0.265 → 0.325 → 0.390 in the dark theme and 0.785 → 0.845 → 0.915 → 0.985 in the light one. Because the steps are even in OKLCH they are even to the eye, which is what gives the bands their boundaries now that none of them is stroked.
3. **Chroma falls as lightness rises.** A neutral at L 0.965 carries 0.010 and one at L 0.390 carries 0.026, so the tint never becomes a colour cast in the highlights or muddy in the shadows.

Measured against those rules the palette holds: the band ladder is 1.26 / 1.22 / 1.30 (recess to bar, 2.01); the muted ramp is 1.30 and 1.32 apart and clears 3.3:1 at the dim step on the chrome bar, which is the lightest band and therefore binding in a dark theme; the light theme's ramp clears 3.0:1 on its recessed field, which is the darkest band and binding there.

**The title row is a lift, not a well.** The panel title rows in the permission and command screens have gone through three treatments. They began on an accent field, which read as a filled accent band and broke the guide's rule; they then took the recessed tone, which made them the *darkest* strip in the frame — a hole where a header belongs. They now have their own step at L .470, one above the panel rather than five below it, at a chroma low enough (0.032) that it stays a neutral lift and the selection band remains the only accent fill besides the gauge. The right-flush fact on those rows moved from the gauge-fill accent step to the `you` step, which clears 3.8:1 on the lighter field where the old one measured 2.2:1. Reusing it forced the pane roles to be reassigned, because a tone can only be reused where it is never adjacent to itself. The command panel's explain pane sits on the transcript ground — it steps 1.30:1 off the list column beside it, 1.27:1 off the header row above, and 1.22:1 off the footer below. The review file pane sits on the recessed tone, stepping 1.27:1 off the diff pane beside it, 2.01:1 off the top bar and 1.55:1 off the footer; the separator row inside that pane consequently inverts, running a row of the *lighter* ground through a darker pane. Every adjacency in the five screens is now at least 1.15:1, and no band touches itself.

### Dark theme, borderless

| Role | Token | Hex | OKLCH |
| --- | --- | --- | --- |
| Desk (outside the window) | `--t-desk` | `#0c0a11` | L .150 C .016 |
| Recessed field, review file pane | `--t-recess` | `#0f0b15` | L .160 C .020 |
| Panel title rows | `--t-title` | `#5d576b` | L .470 C .032 |
| Transcript ground, command explain pane | `--t-ground` | `#27232f` | L .265 C .022 |
| Composer, footer, break rows | `--t-bar-lo` / `--t-break` | `#36313f` | L .325 C .024 |
| Chrome bars, panels | `--t-bar` | `#474251` | L .390 C .026 |
| Primary text | `--t-text` | `#f4f2f9` | L .965 C .010 |
| Code on a neutral row | `--t-code` | `#ece9f3` | L .940 C .014 |
| Body / agent prose | `--t-body` | `#e3dfeb` | L .910 C .016 |
| Tool names, stdout, quiet labels | `--t-quiet` | `#c9c5d2` | L .830 C .018 |
| Section labels | `--t-label` | `#b1adbb` | L .755 C .020 |
| Dim metadata (3.3:1 on the chrome bar) | `--t-dim` | `#9a95a4` | L .680 C .022 |
| Accent text | `--t-accent-text` | `#dfd1fb` | L .885 C .058 |
| `you` speaker label | `--t-accent-you` | `#ceb6fb` | L .820 C .098 |
| Accent mark, caret, live state | `--t-mark` | `#be9df7` | L .760 C .130 |
| Gauge fill, `@@` header | `--t-accent-mid` | `#a081d5` | L .665 C .125 |
| Finished tool `●` | `--t-accent-deep` | `#7f64ab` | L .560 C .110 |
| Selection band | `--t-band` | `#604788` | L .455 C .105 |
| Gauge track | `--t-track` | `#605a6c` | L .480 C .030 |
| Unselected `▌` | `--t-mark-idle` | `#5d576a` | L .470 C .030 |
| Diff added: sign / code / tint | `--t-add` / `-code` / `-bg` | `#5ed476` / `#9ceaa7` / `rgba(94,212,118,.14)` | L .780 / .870 C .170 / .120 H 148 |
| Diff removed: sign / code / tint | `--t-del` / `-code` / `-bg` | `#f66d67` / `#ffa8a0` / `rgba(246,109,103,.14)` | L .700 / .820 C .170 / .110 H 25 |

Hue is 300° for every row above except the diff pair. Listed darkest band to lightest, then the ink from brightest to dimmest — which is also the order of the OKLCH lightness column.

### Light theme, borderless

The same specification with the lightness scale inverted. Bands, lightest to darkest: ground `#fbf9fe` (L .985), bars `#e4e1eb` (L .915), composer, footer and break rows `#cec9d8` (L .845), recessed field and the review file pane `#bbb6c5` (L .785), panel title rows `#cec6df` (L .840 C .036); the command explain pane sits on the ground, desk `#aeaab7` (L .745).

Ink, darkest to lightest: text `#17151c` (L .200), code `#221f28` (L .245), body `#322e39` (L .310), quiet `#433f4b` (L .375), labels `#524d5b` (L .430), dim `#615c6c` (L .485). The ramp is spaced 1.23–1.29:1 per step and calibrated against the **recessed field**, the darkest band and the binding one in this theme, where dim holds 3.3:1 — it sits on that band in three places, the review file pane, the inline diff field and the panel title rows. Accent: marks and gauge fill `#6d41a9` (L .480 C .160), the `you` label `#5c3093`, accent text and the finished `●` `#562c8b`, selection band `#c6b1ef` (L .800 C .088 — on a light ground the band is darker than the page, not lighter), unselected `▌` `#9690a3`, gauge track `#938da0`. The idle mark is set against the recessed field, not the chrome bar, because the review file list sits on the recessed tone — it holds ~1.6:1 there and ~2.4:1 on the bar. Diff added `#007324` with code `#004914` on `rgba(0,115,36,.16)`; removed `#b00a1d` with code `#69040d` on `rgba(176,10,29,.14)`.

The chroma curve runs the other way in this theme — the accent needs *more* chroma to hold against a white ground (0.160 against the dark theme's 0.130) and the neutrals slightly more too, since a tint is harder to see in the highlights.

> **Superseded by Turn 14 — every value in the two paragraphs above is
> retired.** `tokens/semantic.css`'s `.tui-light` scope is the authority, and
> `SYNC.md`'s Turn 14 section carries the full was/now table. Three claims
> here are now actively wrong rather than merely old, so do not read around
> them:
>
> - **"bars `#e4e1eb` … composer, footer and break rows `#cec9d8`"** had the
>   two chrome bands the wrong way round — the top bar sat *lighter* than the
>   composer, the reverse of the dark theme. It is now bar `#d3cedd` under a
>   bar-bottom of `#e0dbea`.
> - **The break row no longer rises above the ground.** The light ladder is
>   monotonic now: ground `#faf7ff` · break `#ede9f6` · bar-bottom `#e0dbea` ·
>   bar `#d3cedd` · recess `#c6c1d1` · panel-title `#bab3c8` · scrim
>   `#a39fac`, each a full step below the last (min 1.127:1, which is wider
>   than the dark ladder's own narrowest rung at 1.011:1).
> - **The accents are no longer ramp steps.** `#6d41a9` / `#5c3093` /
>   `#562c8b` are gone; the marks are `#4b1f7e`, past the ramp's `accent-900`
>   floor, and are stated as literals in `.tui-light`.
>
> Also new in Turn 14: `--tui-step-done`, which splits the finished-`●` role
> in two because the tool glyph recedes *lighter* than the accent here
> (`#a17adf`, 3.11:1 on the ground — the same weight its dark counterpart
> holds at 3.15:1) while a settled first-run step recedes *darker*
> (`#6941a1`).

## Revision log

00000. Permission (`5a`) and first run (`5d`) — the harness's permission model
was rebuilt and these screens describe its terms. A grant is a **program and a
class** (`git: read`) where it was a command pattern (`cargo *`); the option
list is **eight rows**, allow and deny mirrored across once / session / project
/ everywhere, where it was four; the shell tool the screen was drawn around no
longer exists, so a call names a program and an argument list and carries a
declared class; and first run's access scale is three rungs rather than four.
The entries below are left as written — they record what was true when they
were made.


0000. Rules (all three files, 15 in total) — every freestanding rule was a gradient fading to transparent over its outer 48px, inherited from Nocturne's signature. They are now flat single-colour rules running edge to edge, one step more muted than the structural borders they used to match. This affects the turn separators in the transcript, the rule above the permission options, and the step separators in first run.

000. Top bar — every within-group gap was 6 cells, the same gap that parts unrelated groups, so related facts read as if they were unrelated. Facts inside a group are now ` · ` apart: directory and branch (`4a`, `5a`, `5c`), model / gauge / cost (`4a`, `5a`, `5c`), and `3 files · +98 -2` in the review bar (`5b`). The 6-cell gap survives only where it does real work: between the brand and everything else.

0. Top bar (`4a`, `5a`, `5c`, `5d`) — the identity slot held the project name (`gateway`, and `harness` in first run) behind an accent `▌`. It now holds the harness name `aldwin` with no glyph, sitting on the 3-cell content margin. Nothing is lost, since `~/src/gateway` sits immediately to its right, and the pip was marking nothing — `▌` now appears only where it means selection or a caret. Title rows lost their pips for the same reason: `review changes` in `5b` and `permission` in `5a` now start on the 3-cell margin like `commands` in `5c`, which never had one.

> **Measured 2026-09-06 — the gap here is not `--group-gap`.** This section,
> and the summary line elsewhere that says "the 6-cell gap survives only
> between the brand and everything else", both leave the impression that the
> directory sits six cells after the name. It does not. `4a`, `5a`, `5c` and
> `5d` all carry exactly **three** spaces there, which after the 3-cell
> margin and the seven letters of `aldwin` puts the directory on **cell 13**
> — the body column. `--group-gap`'s six cells are real, but they part two
> unrelated groups: `review changes` / `3 files` in `5b`'s title bar, and the
> key hints in every footer. Aldwin shipped six cells here for three weeks
> on the strength of the prose above; see `.claude/spec/aldwin-tui.md`'s
> audit entry of the same date.

0. Permission (`5a`) — options are numbered `1`–`4` and the trailing shortcut-key column is gone. The number is a direct-pick accelerator, and the keys that were on the rows moved into the footer: `↑↓ to move`, `1-4 to pick`, `⏎ to confirm`. Before that, the screen was a centred 80-cell modal over a dimmed-and-scrimmed session. It is now a bottom-anchored full-width panel on `5c`'s structure, covering the composer rows, since input is disabled while a permission is open. The modal geometry tokens (`--modal-w`, `--modal-x`, `--modal-y`) are consequently unused by these screens.

After the first handoff, six grid/content inconsistencies were corrected in both theme files. They are reflected in the tables below; the older copies of these files are superseded.

1. Session (`4a`) — the `you` label column was 11 cells with a 3-cell gutter; it is now 12 + 2 like every other turn (the body column lands on cell 17 either way).
2. Commands (`5c`) — the `you` turn's body started on cell 18; now cell 17.
3. Diff review (`5b`) — `mod.rs` read `+0 -0`; it is `+3`, and the top-bar total is `+98 -2`.
4. Diff review (`5b`) — the hunk carried three context rows and one removed row inside a file marked `new file`. A new file's hunk is all added rows, so those four are gone and the trailing note now reads `73 more added lines below`. The header and gutter followed: a `-0,0` hunk starts at line 1, so it is `+1,84` over rows numbered 1–11, and 11 shown + 73 noted reconciles with the `+84` file stat.
5. Session (`4a`) — the inline diff box read `78 more lines` under a `+84` stat; with three rows shown it is `81 more lines`, matching the same file's arithmetic in `5b`.
6. First run (`5d`) — the `account` label was neutral-500 against `access` at neutral-600; both inactive step labels are neutral-600, and only the active step's label is accent-400.

## Fidelity

**High fidelity.** Colors, cell metrics, content and every column position are final. The frames measure exactly 120 columns × 36 rows; each cell is 9 × 20 CSS px in the mock, so any px value in the HTML divides cleanly: 9px = 1 column, 20px = 1 row, 27px = 3 columns, 108px = 12 columns.

## The grid

| Quantity | Value |
| --- | --- |
| Terminal size designed for | 120 cols × 36 rows |
| Cell | 9 × 20 px in the mock (mono advance width 0.6em at 15px) |
| Left / right margin for all content | 3 cells |
| Speaker label column | 12 cells wide, starting at the left margin |
| Body text column | starts at cell 17 (3 margin + 12 label + 2 gutter) |
| Top bar | 3 rows (60px), rule below |
| Bottom bar | 5 rows in the session (rule, blank, prompt, blank, status, blank) |
| Gap between groups inside a bar | 6 cells |

> **Superseded by Turn 13 — see `SYNC.md` and `tokens/cells.css`.** The label
> column is **8 cells** and the body column starts at **cell 13**. The table
> above is the pre-Turn-13 grid and is retained only because the prose below
> still quotes it.

Anything right-aligned (line counts, `+84`, key hints, cost) is flush to the right margin — in ratatui, `Alignment::Right` inside the same rect, not padded strings.

## Design tokens

From the Nocturne design system (`_ds/nocturne-.../styles.css`). Terminals need explicit RGB, so the resolved hexes are listed.

### Dark theme (default)

| Role | Token | Hex |
| --- | --- | --- |
| Ground | `--color-bg` | `#161826` |
| Chrome bars (top bar) | `--color-surface` | `#232532` |
| Bottom bar / raised ground | — | `#1b1d2b` |
| Primary text | `--color-text` | `#e9e9ed` |
| Body text (agent prose) | `--color-neutral-300` | `#cfd3e5` |
| Muted label | `--color-neutral-600` | `#75798c` |
| Dim metadata | `--color-neutral-700` | `#595d6c` |
| Borders, rules, gauge track | `--color-neutral-800` | `#3f424d` |
| Accent (marks, caret, live state) | `--color-accent` | `#84aed9` (superseded — see Borderless rebuild) |
| Accent text on ground | `--color-accent-300` | `#c1d7ee` |
| Speaker label "you" | `--color-accent-400` | `#95bce4` |
| Gauge fill, `@@` hunk header | `--color-accent-600` | `#567ea7` |
| Completed tool glyph, panel border | `--color-accent-700` | `#406181` |
| Selection band | `--color-accent-900` | `#202d39` |
| Diff added sign and gutter | `--tui-add` | `#70cf75` |
| Diff added row background | `--tui-add-bg` | `rgba(112,207,117,0.13)` over ground |
| Diff added code text | `--tui-add-code` | `#a0e8a1` |
| Diff removed sign and gutter | `--tui-del` | `#e86c68` |
| Diff removed row background | `--tui-del-bg` | `rgba(232,108,104,0.14)` over ground |
| Diff removed code text | `--tui-del-code` | `#ff9d96` |

Accent values are the design system's current ones (dusty azure). Frames reference the tokens, not these hexes.

### Light theme

Same layout, ramps flipped. Ground `--color-neutral-100` `#f3f5fe`; bars `--color-neutral-200` `#e4e7f5`; text `--color-neutral-900` `#292b31`; body text `--color-neutral-800` `#3f424d`; muted `--color-neutral-600` `#75798c`; dim `--color-neutral-600`; borders `--color-neutral-300` `#cfd3e5`; gauge track `--color-neutral-500` `#9397ab`; gauge fill and accent marks `--color-accent-600` `#567ea7`; accent labels `--color-accent-700` `#406181`; **selection band `--color-accent-300` `#c1d7ee`** (on a light ground the band must be darker than the page, not lighter); diff added `#0a7520` on `rgba(10,117,32,0.16)` with code `#094112`, removed `#b31124` on `rgba(179,17,36,0.14)` with code `#621417`.

Rule for both themes: the accent is a **mark or a line, never a filled field**. The only accent fills are the 1-cell `▌` selection marks, the faint selection band, and the gauge fill.

### Glyphs

| Glyph | Meaning |
| --- | --- |
| `▌` | accent mark: selected row, caret. Never in a top bar or a title row. |
| `●` | tool call finished (accent-700) |
| `◐` | tool call / process running (accent) |
| `○` | pending (neutral-700) |
| `✔` | hunk accepted (diff green) |
| `▶` | composer prompt |
| `█` | gauge fill / track segment (context bar, progress) |
| `+` `-` | diff signs, in the diff colors |

Content inside a frame is separated by **a full row of a different ground**, never by a rule. The turn break and the step separators in first run are one 1-row band of the composer's tone running edge to edge; panes are parted by each carrying its own tone. In a terminal that is a single `Style::bg` on a one-row rect, so nothing here needs approximating. (Nocturne's fading-rule signature, the `--rule-fade` tokens, and the flat-rule treatment that replaced them are all unused by these screens.)

## Screens

### 1. Session (`4a` in the files)

The primary view. Purpose: read what the agent is doing and type the next instruction.

Layout, top to bottom:

1. **Top bar**, 3 rows, on the chrome tone `--t-bar` — a step above the transcript ground, with no rule below it.
   - Left group: `aldwin` in primary text on the 3-cell margin — no glyph before it — then 6 cells, working directory (`~/src/gateway`) muted, then ` · ` — one cell, a dim `·`, one cell — and the branch (`main`) in neutral-400 with a dirty marker `*` in accent. Directory and branch are one group describing where the session is pointed, so they sit tight against the dot rather than taking the 6-cell gap that parts groups. The dot marks that they are two different facts. The slot holds the harness name, not the project name; the repository is already named by the working directory beside it. The top bar carries **no accent mark**: the name is the brand, and a pip there indicated nothing.
   - Right group, on the same ` · ` rhythm: model (`sonnet-4.6`) muted, ` · `, context gauge — 4 cells of `█` in accent-600 then 6 cells of `█` in neutral-800 then ` 38%` — ` · `, session cost (`$0.42`) in primary text. Gauge and percentage stay unseparated, since the number reads the bar.
2. **Transcript**, bottom-anchored (new content grows upward from the bottom bar).
   - Each turn is a row of two columns: a 12-cell label column holding the speaker (`you` in accent-400, `harness` in neutral-400) on the first row and the time (`09:42`, neutral-700) on the second; then the content column starting at cell 17.
   - Turns are separated by a blank row, a 1-row band of the composer tone, and another blank row.
   - Agent prose is neutral-300. One blank row between prose and a tool group.
   - **Tool call line**: glyph, 2 spaces, tool name padded to 6 characters in neutral-600 (`read  `, `grep  `, `write `, `edit  `, `bash  `), then the target — path in primary text for mutations, neutral-300 for reads. Right-aligned result summary in neutral-700 (`412 lines`, `7 hits in 3 files`) or the diff stat (`+84` in diff green, `+11 -2`).
   - A running tool uses `◐` and puts its name in accent-300; its stdout follows indented 3 cells in neutral-600, with a trailing accent `▌` as the live cursor.
   - **Inline diff**: a recessed field on `--t-recess`, no outline, inset to the body text column. Each row: a 5-cell right-aligned line number in `--t-label`, the `+` sign, then the code. Both the sign and the code take `--t-add-code` here rather than the sign/code split the review pane uses, because a tinted row over the recessed field is the darkest backdrop in the light theme and the mid-lightness sign green measures only 2.7:1 on it; the code colour holds 4.8:1 light and 5.9:1 dark, and the neutral label step holds 3.5:1 for the gutter. The review pane's own hunk keeps the full three-role split — neutral-free gutter and sign in `--t-add`, code in `--t-add-code` — because its diff sits on the much lighter transcript ground. Each row: 5-cell right-aligned line number (neutral-700), `+ `, then the code. Added rows carry the added-row background across the full box width. A final row reads `  81 more lines` in neutral-700 with an empty gutter.
3. **Bottom bar**, full width, `--t-bar-lo` — a step below the transcript ground, no rule above: blank row, prompt row (accent `▶`, 2 spaces, the draft text, accent `▌` caret), blank row, status row (`◐  working   41s` — glyph in accent, text neutral-600 — with `esc to stop` right-aligned in neutral-700), blank row.

Deliberately **not** in this view: a key-hint legend and a "files changed" counter. Both were removed as noise.

### 2. Permission prompt (`5a`)

Purpose: approve or deny one tool call. **Full-frame panel anchored to the bottom of the frame, on the same structure as the command panel in `5c`** — not a centred modal. The panel takes the composer's rows as well as its own, because input is disabled while a permission is pending: there is nothing to type into, so the prompt row is not drawn at all. The transcript above stays in place at 35%, and there is no scrim.

Panel: full frame width, 18 rows, ground the chrome-bar tone. The tonal step off the transcript is the whole boundary — there is no rule along its top edge:

- Title row on `--t-title`, full width: `permission` in accent text on the
  3-cell margin, no glyph; right-aligned, in the `you` accent step, the
  **program** — which is what `bash` always was here. Since ADR 0004 that is
  the grant key, the thing every option row below quotes, so a `run` call on
  `git` right-flushes `git` and not the tool name `run`. For a built-in the
  two coincide (`read` is both), which is why this needs saying: the slot has
  one meaning, and it is the program. Same row as `commands` in `5c`.
- Blank row, then the sentence. There is no shell to run a command in: a call
  names a program and an argument list, and it carries a class the agent
  declares for it — `The agent wants to run git, declared a write.` A call
  declared a read says what that means, since the harness enforces it rather
  than trusting it: `The agent wants to run git, declared a read. It runs
  read-only, with no network.` In neutral-300.
- Blank row, then the command block: a recessed field on `--t-recess`, no border and no accent bar, with the command on one row 2 cells in — `$` in accent-400 then the command in primary text, and a blank half-row above and below.
- Blank row, then the key/value table on the frame's own columns: labels in the
  12-cell label column, values from cell 17. `in` is the working directory.
  **`writes` and `network` are now real facts rather than the guesses they were
  when this was drawn** — a read-declared call runs with the project read-only
  and the network unreachable, so they read `refused` and `off`; a
  write-declared one runs unconfined. **Open question for this screen:** three
  fact rows plus eight option rows do not fit an 18-row panel, so the harness
  currently states those two facts inside the sentence above and draws `in`
  alone. Whether the table shrinks, the band grows, or the list splits is a
  design decision this note deliberately does not make.
- One row of the recessed tone, blank row.
- Eight option rows, flush to the frame's left edge like the command rows in `5c`: the selected one has an accent `▌` and the accent-900 band, the rest a neutral-800 `▌`. Text in body colour with the matched pattern one step quieter.
- **Each option is numbered `1`–`8`**, one cell after the mark and two cells before the label, so option text starts at cell 6. Typing a number picks that option directly, which replaces the right-flush key column the rows used to carry (`⏎`, `a`, `shift-a`, `d`). Arrows still move the selection and `⏎` still commits it; all three keys are named in the footer rather than on the rows. The number is accent-300 on the selected row and neutral-600 on the rest. Nothing is right-aligned in these rows now.
- Footer row on the bottom-bar tone, sitting where the composer's status line would be. Left, three key hints on `5b`'s pattern — key in the accent, verb one step quieter, groups 6 cells apart: `↑↓ to move`, `1-8 to pick`, `⏎ to confirm`. Nothing is right-flushed here any
  more: the slot held `saved to .harness/permissions.toml`, which named the
  wrong file (it is `.aldwin/permissions.yaml`) and was true of three rows out
  of eight — the two `once` rows save nothing and the session rows never touch
  disk. Each row states its own reach instead. No `esc to close` — a permission
  has to be answered, so the escape is `Deny once`.

This screen is authored entirely in the `--tui-*` semantic roles, so the one markup is identical in both theme files.

Copy, verbatim, in this order — allow and deny mirrored across the same four
scopes, with the program and the class quoted one step quieter inside each
sentence:

`Allow once` / `Allow cargo writes for this session` /
`Always allow cargo writes in this project` / `Always allow cargo writes everywhere` /
`Deny once` / `Deny cargo writes for this session` /
`Deny cargo writes in this project` / `Never allow cargo`.

A grant is a **program and a class**, not a command pattern — `cargo *` became
`cargo writes`, because the class belongs to the call (`git status` is a read,
`git push` is a write, same binary). The eighth row is deliberately blunter
than the rest: the whole program, every class, everywhere. It is a lock —
nothing narrower overrides it — which is why the deny side is drawn at all
rather than left to a config file.

`Deny and tell the agent why` is gone: nothing in the round trip carries a
reason and no step collects one, so the row named something the product does
not do.

### 3. Diff review (`5b`)

Purpose: accept or reject the agent's changes hunk by hunk. Full-frame, no modal.

- Top bar, same geometry as the session: `review changes`, then `3 files · +98 -2`; right `against main` and `^d closes`.
- Left pane, 33 cells, on the recessed tone `--t-recess` with no rule on its right edge — it is the one band on this screen that steps down rather than up, which parts it from both the diff pane beside it and the top bar above it: the word `files` in neutral-700, then one row per file — selection `▌` + band, status letter (`A` diff-green, `M` neutral-400), file name, right-aligned stat (`+84`, `+11 -2`, `+3`). A row of the recessed tone, then `hunks in limit.rs` and one row per hunk with `✔` accepted / `◐` current / `○` pending. Pinned to the bottom of the pane: `1 of 3 accepted` and a 18-cell gauge.
- Right pane: file path with `new file` right-aligned, the `@@ -0,0 +1,84 @@ impl RateLimit` header in accent-600, blank row, then the unified hunk. Because `limit.rs` is a new file the hunk is entirely added rows — no context and no removed rows; the gutter is diff-green throughout, then sign, then code in neutral-200. Below the hunk, `73 more added lines below` in neutral-700 indented to the code column (cell 11 of the pane, not the sign column). The removed-row treatment is therefore only shown in this screen's stats (`+11 -2`); its colors are in the token table above.
- Bottom bar, 3 rows: `y accept hunk`, `n reject`, `a accept file` (keys colored, labels muted) with `⏎ apply and continue` right-aligned.

### 4. Commands (`5c`)

Purpose: run a slash command. Typing `/` in the composer lifts a full-width panel off the bottom bar — it is not a floating centered palette.

- The transcript behind dims to ~35%.
- Panel: 11 rows on the chrome tone, no top border. Header row on `--t-title`: `commands` in accent-300, `7 of 22` right-aligned in accent-600.
- Left list, 48 cells, on the panel tone with no rule at its right edge — the explain pane beside it drops to the recessed tone instead: one row per command — `▌` mark, command name in a 14-cell column, description. Selected row: accent `▌`, accent-900 band, primary text, description in accent-400. Others: neutral-800 `▌`, neutral-300 name, neutral-600 description.
- Right pane explains the highlighted command: name in accent-300, blank row, three rows of neutral-300 prose, blank row, then two gauge rows — label in a 10-cell column (neutral-600), value in a 9-cell column, then fill/track/percentage. `now 76.2k … 38%` and `after 9.4k … 5%`.
- Bottom bar shows the live filter: `▶  /co▌` with `esc to close` right-aligned.

### 5. First run (`5d`)

Purpose: settle account, model and access before the first prompt.

- Top bar carries `aldwin` and the working directory, with the version right-aligned.
- One line of neutral-300 prose, three rows down: `A terminal agent in this repository. Three answers and it starts.`
- Three steps, each on the same 12-cell label column as a transcript turn, separated by one row of the break tone:
  - `account` — `●` in diff-green, `dev@proton.ch`, right-aligned `signed in` in neutral-700.
  - `model` (label in accent-400, `step 2 of 3` beneath in neutral-700) — prose `Pick a default. /model changes it later.`, then three option rows in the same selected/unselected treatment as elsewhere: `sonnet-4.6 balanced · 200k`, `opus-4.6 slower, deeper`, `haiku-4.6 fast, cheap`.
  - `access` — prose `How much runs without asking.` then one neutral-700 row:
    `ask · read · write`. Three points, not four. The answer is stored as the
    scope's standing rung in `permissions.yaml` and is the same setting a
    developer edits later — not a preset that expands into grants and then
    stops existing. `full access` had no distinct meaning to offer: editing a
    file always shows a diff and waits, under every rung, so no point on this
    scale can mean everything runs.
- Bottom bar: `⏎ continue`, `↑↓ choose`, and `config → ~/.aldwin/` right-aligned.

> **Superseded by Turn 13 — see `SYNC.md`.** First run was rebuilt: wordmark,
> positioning line, **two** steps, a four-point access scale, and **no account
> step**.
>
> **Superseded again (frame re-fetched 2026-09-06).** The two steps are now
> **`provider` then `access`**; there is no `model` step at all. Measured off
> `Agent TUI v2.dc.html`'s own inline styles in the discussion project
> (`25845063-…`), since neither this prose nor the token CSS states these:
>
> - Step shape: name in the 8-cell label column, `step n/m` beneath it in
>   `--t-dim` (exactly 8 cells — hence the slash), and in the body column one
>   row of `--t-body` prose, one blank row, then the option rows. Active
>   step's label in `--t-accent-you`, inactive in `--t-label`; **both** state
>   their number.
> - `provider` prose: `Where the model runs. /model picks a model once the
>   session starts.` Rows, in the shared 16-cell option field: `anthropic`
>   (`claude models · ANTHROPIC_API_KEY`), `google` (`gemini models ·
>   GOOGLE_API_KEY`), `openai` (`gpt models · OPENAI_API_KEY`), `ollama`
>   (`local models · no key`), then `more` (`the full provider list`) with a
>   `→` flush to the 3-cell right margin. `more`'s name is `--t-label` and its
>   purpose `--t-dim` — one step quieter than a real option.
> - `access` prose: `Which actions run without asking. /access changes it
>   later.` Four rows, none selected while `provider` is the active step.
> - The selected row's purpose text is `--t-accent-text`, not `--t-quiet`.
> - Footer: `⏎ continue`, then `↑↓ choose`, with the config location right.
>
> Aldwin ships three of the four provider rows (`ollama` cannot work while
> `api_key_env` is required), three access points rather than four, and drops
> the `/access` clause — see `.claude/spec/aldwin-tui.md`'s entry of the same
> date.
>
> **Superseded a third time — Turn 14, measured 2026-09-06.** First run is now
> **three** steps (`provider`, `model`, `access`), and the shape changed as
> well as the count: rather than paginating, **all three sit on screen from
> the start as one vertical spine**. A settled step collapses to its answer, the
> open step expands into its list, and a step still to come previews what it
> will ask. Frames `14a`/`14b`/`14c` in both files; `14d` is new and covered
> below. Measured off the frames' own inline styles, since neither this prose
> nor the token CSS states any of it:
>
> - **The spine's two columns are new tokens** (`tokens/cells.css`):
>   `--step-mark-col` = `--label-col` + `--label-gutter` = **10 cells**, and
>   `--step-content-col` = `--margin-x` + `--step-mark-col` +
>   `--option-label-col` = **cell 29**. So a step row is: glyph on the 3-cell
>   margin, step name on the body column (cell 13) in the shared 16-cell
>   option field, and the step's content on cell 29 — *whatever* that content
>   is. A settled answer, a purpose line and an expanded option list all hang
>   on one column.
> - **The `step n/m` counter is gone.** The three glyphs carry the sequence
>   instead: `●` settled (`--t-step-done`), `▌` open (`--t-mark`), `○` pending
>   (`--t-mark-idle`). The label column is empty on every step row.
> - Step name colours: settled `--t-label`, open `--t-accent-you`, pending
>   `--t-dim`. The settled step's *answer* is `--t-text`; a pending step's
>   preview line is `--t-dim`.
> - **One blank row between steps**, not the 3-row `--section-gap-h`. That gap
>   survives only once, between the positioning line and the first step.
> - Option rows sit at cell 29, so within a row the mark is at 29, the name at
>   32 and the detail at 48 (`▌`, two spaces, the 16-cell name field). Same
>   colours as before.
> - `14c` shows **nothing preselected on `access`** — every mark idle, no
>   band — and its footer reads `⏎ start session` rather than `⏎ continue`.
>   Neither is what Aldwin ships; see the departures note below.
> - Footer is two hints only: `⏎ continue`, `↑↓ choose`. There is no `← back`.
> - **The wordmark is padded by TWO spaces at each end**, not one:
>   `  M J O L N I R  `, a 17-cell reverse-video field. Aldwin shipped 15 on
>   the strength of a Turn 13 reading that said "one space".
>
> **`14d` — returning / empty state.** New frame, and the one a returning
> developer actually opens into: what `aldwin` shows in a known repository,
> and what `/clear` leaves behind. Bands are `--bar-top-h` / `1fr` /
> `--bar-bottom-h` (3 / 28 / 5).
>
> - Top bar, left: `aldwin` in `--t-text`, three spaces, cwd in `--t-quiet`,
>   ` · ` with the dot in `--t-dim`, branch in `--t-body`, dirty `*` in
>   `--t-mark`. Right: the model id in `--t-quiet` — no gauge and no cost,
>   because neither exists yet.
> - Body is **bottom-anchored** (`justify-content:flex-end`), so the content
>   sits against the composer rather than centred: wordmark, blank row, then
>   three facts on the ordinary 8-cell label column — `in` (cwd, three spaces,
>   branch, `*`), `provider` (`anthropic · sonnet-4.6`), `access` (the tier) —
>   blank row, `Ask for a change, or / for commands.` in `--t-dim` with the
>   `/` itself in `--t-quiet`, blank row.
> - There is **no version and no commit** on this screen. The version lives in
>   first run's top bar; the session's top bar carries the model instead.
> - Bottom bar, five rows: blank, `▶  ▌` (both `--t-mark`), blank, `ready` /
>   `^d closes` both in `--t-dim`, blank.
>
> Aldwin's departures from `14a`–`14d`, each with a reason recorded where it
> is made: `access` keeps its preselected `ask` row (a list with no selection
> made `⏎` a no-op — see `first_run::FirstRun`), three access points rather
> than four (ADR 0001), `⏎ continue` on every step, `config → ~/.aldwin/`
> rather than `~/.harness/config.toml`, and the `←` key stays bound but
> unhinted.

## Interactions & behavior

Static frames were requested, so no motion is specified beyond these implied behaviors:

- **Keyboard model**: plain — arrows/`↑↓` move, `⏎` confirms, `esc` closes an overlay or interrupts a run, `/` opens commands, `^d` opens diff review, single letters are the shortcuts shown in each footer.
- **Transcript** is bottom-anchored and scrolls; tool output streams into the row it belongs to, with `◐` and a trailing `▌` cursor while live, becoming `●` plus a right-aligned summary when done.
- **Spinner**: `◐` should cycle through a quarter-block or braille sequence at roughly 100ms per frame; keep it in the accent.
- **Context gauge** redraws whenever the token count changes; `/compact` shows a projected "after" value before running.
- **Permission rules** chosen in 5a persist to `permissions.yaml` — the
  project's `.aldwin/` for the two `in this project` rows, `~/.aldwin/` for
  the two `everywhere` rows. The session rows live in memory only and the two
  `once` rows save nothing. A deny is a **lock**: nothing narrower overrides
  it, so a locked call is refused without drawing a prompt at all — there is
  no answer at a prompt that would lift it.
- **Selection** anywhere is band + `▌` mark together — never one alone. There is no hover state; a terminal has no pointer.

## State

- Session: transcript entries (user turn, agent prose, tool call with status/result/diff), token count and cost, cwd and git branch with dirty flag, composer draft, run state (`idle` / `working` with elapsed seconds), pending permission request.
- Diff review: file list with per-file stats, per-hunk accept/reject/pending, current file and hunk index.
- Commands: filter string, filtered list, highlighted index.
- First run: current step, chosen model, chosen access level, account state.

## Type

The mocks use JetBrains Mono at 15px/20px, weight 400 (500 for the badge chips outside the frames). In a terminal the user's own font is used — the only requirement is that the design assumes a **single-width monospace cell** and no letter-spacing. No bold is required anywhere; hierarchy is color and position.

## Assets

None. No images, no icons — every mark is a Unicode box-drawing or block character, listed in the glyph table above. The design system's icon recommendation (Phosphor) does not apply in a terminal.

## Files

- `Agent TUI v2.dc.html` — dark theme, all five screens. Session frame id `4a`; permission `5a` (bottom panel); diff review `5b`; commands `5c`; first run `5d`.
- `Agent TUI v2 Light.dc.html` — light theme, same five frames and ids.
- `Agent TUI.dc.html` — the first exploration: three alternative session directions (`1a` rail, `1b` panels, `1c` ledger) plus early overlay states, on the dark theme. Useful for the rejected alternatives; superseded by v2.
- `support.js`, `_ds/nocturne-.../styles.css` — runtime and design tokens the HTML needs in order to open in a browser.

Open either file directly in a browser to inspect the frames; use the browser's element inspector to read exact positions, which are all multiples of the 9 × 20px cell.
