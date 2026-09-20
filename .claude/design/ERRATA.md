# Errata — where this reference disagrees with itself

`HANDOFF.md` is prose written across fifteen turns of design work, over a
token layer that was rebuilt twice underneath it. In a number of places its
words and its own CSS state different values, and in a few its words and its
own rendered frames do. Every entry below is one of those places.

**This file is part of the reference, not commentary on the product.** It is
the one addendum a blind judge may read, and the rule that keeps it so is:

> An entry may cite **only** a measurement of the design system's own files
> or frames, or a numbered ADR. Never a preference, never "the app does it
> this way", never a rationale for a product decision.

That rule is what makes this admissible where the conformance catalogue is
not. The catalogue holds classifications, source reasoning and previous
scores — the three things a blind judge must not have. This holds only the
reference correcting itself, which is reference material by definition. If an
entry here cannot name a file, a line and a measured value, it does not
belong here; it belongs in `.claude/spec/mjolnir-design-conformance.md`.

## Why it exists

On `run-1789850385`, six independent blind judges spent 890K tokens. Three of
them re-raised the missing `--t-recess`; four re-derived the turn-break tone
contradiction; three the glyph-table contradiction; three the brand-gap
contradiction. All four questions were already settled, and three of them had
been settled by measuring this design system's own HTML. That is a large
share of six careful readings spent on closed questions, and it recurs every
run because the frames still show what the judges are measuring.

---

## A. The prose predates the token layer

In each row the frames render the **token**, and the token is correct. The
prose is Turn 12-or-earlier text that the Turn 13–15 rebuilds did not revisit.

| prose | says | token | resolves to |
| --- | --- | --- | --- |
| `HANDOFF.md:243`, `:258` | the turn break is "the composer's tone" | `--tui-break` | `--color-ground-2` `#1e1a26` — two rungs from the composer's ground-4 |
| `HANDOFF.md:259` | agent prose is neutral-300 | `--tui-body` | `--color-neutral-200` `#e3dfeb`, which `HANDOFF.md:88`'s own table also states |
| `HANDOFF.md:257` | `harness` is neutral-400 | `--tui-speaker-agent` | `--color-neutral-300` `#c9c5d2` |
| `HANDOFF.md:262` | the inline gutter is `--t-label` **and**, nine lines later, neutral-700 | `--tui-label` | `--color-neutral-400` `#b1adbb`. The two halves are in one paragraph |
| `HANDOFF.md:262` | the inline diff is "a recessed field on `--t-recess`" | `--tui-diff-box` | and `semantic.css:57-59` says outright that a block does **NOT** use `--tui-recess` |
| `HANDOFF.md:264` | the result summary is neutral-700 | `--tui-dim` | neutral-700 is `--tui-mark-idle` in the current role set and is not an ink role at all |
| `HANDOFF.md:278` | the unselected `▌` is neutral-800 | `--tui-mark-idle` | neutral-700 `#5d576a`. Neutral-800 is `#474251` — the panel's own ground, a 1.00:1 invisible glyph |
| `HANDOFF.md:278` | the selection band is accent-900 | `--tui-band` | `--color-band-dark` `#604788`, which `palette.css` introduces as "a real accent fill now, not a faint tint" |
| `HANDOFF.md:265` | a running tool's name is accent-300 | — | accent-300 is `--tui-speaker-you`, the *user's* step. No ink role resolves there for an agent row |
| `HANDOFF.md:180-181` | the label column is 12 cells and body starts at 17 | `cells.css` | 8 and 13. Already marked superseded at `HANDOFF.md:186-189` |

## B. Measured in the rendered frames, not in the prose

`CLAUDE.md`'s first design rule: measure the handoff HTML; reading it is not
enough. These are the places that rule has already been applied, with the
date the frame was fetched.

1. **There is no `--t-recess` anywhere in `5a`.** The prose uses the word
   twice — `HANDOFF.md:275` for the command block and `:277` for the row
   above the options — and the markup uses it zero times. The command field
   is `--t-ground`; between the last fact row and the first option there is a
   single `<div style="height:var(--row)"></div>` and nothing else. Measured
   off `Agent TUI v2.dc.html` 2026-09-19.

   Consequence worth knowing, and **not** a defect in the app: `--t-ground`
   inside a `--t-bar` panel steps *down* in the dark theme and *up* in the
   light one, where it is the lightest rung in the ladder. The design's own
   markup produces a bright card on the panel in light.

2. **The inline diff's gutter is a 5-cell field, and its number is 4 cells.**
   `--gutter-line-no-inline` is 5; the markup is `flex: 0 0 45px; text-align:
   right; padding-right: 9px` — a 5-cell field carrying a 4-cell number and
   one cell of separation, so the number right-aligns to cell 16 and cell 17
   is the separation. Reading "a 5-cell right-aligned line number" as "the
   number ends at 17" is the misreading; four separate judges have made it.

3. **The option row's three tones are `--t-mark-idle`, `--t-label` and
   `--t-quiet`.** The frame's own option rows read:

   ```html
   <span style="color:var(--t-mark-idle)">▌</span><span>  </span>
   <span style="color:var(--t-label)">2</span><span>  </span>
   <span style="color:var(--t-body)">Allow </span>
   <span style="color:var(--t-quiet)">cargo test</span>
   <span style="color:var(--t-body)"> for this session</span>
   ```

   So the number is `--t-label` unselected and `--t-accent-text` selected,
   the idle mark is `--t-mark-idle`, and the quoted pattern is `--t-quiet`.
   `HANDOFF.md:278-279`'s "accent-300 … neutral-600 … neutral-800" is stale
   in all three places. Measured 2026-09-19.

4. **Option text starts at cell 6.** `HANDOFF.md:279` states it, and the same
   sentence's own arithmetic — "one cell after the mark and two cells before
   the label" — yields 5. The stated 6 is what the frame draws.

## C. The reference states two different numbers

Neither half is stale; the document simply says both.

1. **The brand-to-cwd gap.** `HANDOFF.md:254` (`4a`) says `mjolnir`, then "6
   cells", then the cwd, and `--group-gap` is 6; `HANDOFF.md:389` (`14d`)
   says `mjolnir`, "three spaces", cwd. Those put the cwd on cell 16 and cell
   13. Cell 13 is also what the 3/8/2 grid derives for every other row in the
   system.

2. **The contrast floor.** `SYNC.md`'s Turn 15 note calls **4.67:1** "the
   project's minimum" while rejecting an added `+` at 4.51:1.
   `HANDOFF.md:109` calibrates `--t-dim` — the dimmest ink role — at
   **3.3:1**, and sets the idle `▌` at ~1.6:1 on the recessed field and
   ~2.4:1 on the bar. These cannot all be global. The harness holds 3.3:1 for
   ink and 1.6:1 for marks, being the two the design calibrated rather than
   asserted.

3. **The panel's dim.** `HANDOFF.md:269` says the transcript behind a
   permission panel "stays in place at 35%". Computed against the design's
   own tokens, 35% puts the dimmed body at **2.706:1** in dark and
   **1.975:1** in light — below the 3.3:1 the same document calibrates for
   its dimmest ink, and below anything legible in light. The stated opacity
   and the stated floor are not jointly satisfiable, and the design gives no
   third number.

4. **The closed glyph table is contradicted by the design's own copy.**
   `HANDOFF.md:232-241` closes the table at `▌ ● ◐ ○ ✔ ▶ █ + -`, and
   `IMPORT.md` restates it as closed. The same document then mandates `·` as
   the top bar's within-group separator (`:142`, `:254-255`), `→` on first
   run's `more` row (`:336`), `⏎` and `↑↓` in two footers (`:280`, `:341`),
   and presupposes `…` for clipping. ADR 0002 had to carve out the
   box-drawing set by hand for the same reason.

## D. Departures a numbered ADR authorises

These are differences from `HANDOFF.md` that a decision record already
settled. A frame showing one is conformant.

| the frame shows | instead of | authority |
| --- | --- | --- |
| a markdown table drawn with `┌ ┬ ┐ ├ ┼ ┤ └ ┴ ┘ ─ │` | no table component at all | ADR 0002 |
| five permission options, footer `1-5 to pick` | `5a`'s four | ADR 0001 §5, ADR 0003 §2 |
| each option a sentence stating its own rule | a name + detail pair | ADR 0003 §1 |
| no `Tab` scope row, no grant-summary row | ADR 0001 §3's summary row | ADR 0003 §3 |
| `Deny` | `Deny and tell the agent why` | ADR 0003 §4 — the product collects no reason |
| a footer ending at the key hints | `saved to .harness/permissions.toml` | ADR 0003 §3 — true of one tier in five |
| `Approve` / `Deny` with no tier on an edit | any allowlist option | ADR 0001 §2 — `edit` is outside the permissions model |
| three access points | `5d`'s four | ADR 0001 §5 |

Note what is **not** on this list: the permission panel's title still reads
`permission` on an edit approval, which ADR 0001 §2 puts outside the
permissions model. That is an open question, not a settled departure.
