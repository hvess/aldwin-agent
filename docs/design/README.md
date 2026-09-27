# Aldwin Design System

Aldwin is a coding agent that runs in a terminal. You describe a change in plain words; Aldwin reads and runs what it needs without asking, says what it is doing, and never edits a file directly. Every edit opens as a full-window review, and nothing is saved until you approve.

This design system is derived from one source: **`Aldwin Agent TUI.dc.html`** in this project, the Apple-inspired redesign of the agent TUI. Every token value here is copied from that page. It replaces the Mjolnir design system, which described an earlier, denser version of the product (permission prompts, reverse-video wordmark, JetBrains Mono 15/20, hue-300 palette). None of Mjolnir's values carry over.

## Index

- `styles.css`: the file consumers link. `@import`s only.
- `tokens/colors.css`: dark roles on `:root`, light roles on `.tui-light`, canvas colours
- `tokens/typography.css`: Geist Mono cell text, system UI for the canvas
- `tokens/layout.css`: the 1ch × 24px grid, columns, window measures
- `tokens/elevation.css`: window shadow, badge and control radii
- `tokens/motion.css`: caret timing and `@keyframes caret`
- `tokens/fonts.css`, `tokens/base.css`: Geist Mono import, body and link defaults
- `components/`: `frame/`, `conversation/`, `brand/`, `overlays/`, `review/`
- `guidelines/`: foundation specimen cards
- `ui_kits/tui/`: click-through of Launch, Working, Question, Review and Saved
- `Aldwin Agent TUI.dc.html`: the source design, all ten frames
- `SKILL.md`: agent-skill entry point

### Components

- frame: `Frame` (plus `Blank`, `Spacer`), `Field`, `StatusBar` (plus `KeyHint`), `ContextBar`
- conversation: `UserEcho`, `Prose`, `Disclosure` (plus `DetailRow`), `PlanStep`
- brand: `Mark`, `LaunchCard`
- overlays: `QuestionPanel` (plus `OptionRow`), `CommandRow`
- review: `ReviewHeader` (plus `ReviewBody`), `FileTree` (plus `FolderRow`, `FileRow`, `ProgressDots`), `DiffRow` (plus `DiffHeader`), `CommentField`

There is no Button, Input, Card, Badge or Toast. The product has none. Actions are key glyphs at the field's right edge and in the footer.

## Content fundamentals

- **Lead with a sentence.** Every screen opens with a plain-language line about what is happening: *Looking at how requests move through the gateway.* *Nothing limits requests yet. Adding a limit for each key.* Technical detail sits one disclosure below, exact and unabridged: `src/gateway/mod.rs  412 lines`.
- **No jargon on the surface.** The plan is written as outcomes: *Count requests per key*, *Turn away requests over the limit*, *Check that it works*. Never command names or tool names.
- **Person.** Aldwin speaks as itself without "I" in status lines; "you" for the user (*Waiting for you*, *Nothing is saved until you approve*). The product never says "we".
- **Sentence case** for prose, labels and keys: *Ready*, *Working…*, *Stop*, *Hide Details*, *Commands*. The launch card uses capitalised fact labels: *Project*, *Branch*, *Model*.
- **Facts are short and right-flush**: `412 lines`, `7 matches`, `+11 −2`, `38%`, `◆ 1`.
- **Questions always offer a yes, a no, and "Chat about this."** One line of question, one line of why.
- **Footers name only the keys that work right now.** Glyph, two spaces, verb: `⎋  Stop`, `↑↓  Choose`, `↩  Select`. Review hides keys behind `?  Keys`.
- **Results in one sentence**: *Done. Each key now gets 100 requests a minute, read from settings.*
- No emoji, no exclamation marks, no "just", "simply", "easily".

## Visual foundations

- **Grid.** Geist Mono 14px on 24px rows. Every horizontal measure is whole `ch`: 3ch margin, 2ch mark column, so prose starts at 5ch. Groups in the footer sit 5ch apart. Vertical spacing is blank rows, never padding.
- **Colour.** Neutrals are OKLCH at hue 260 with chroma under 0.01. One accent, blue at hue 255: **blue means you**. Your prompt, your selection, your comments, your next action, and nothing else. Amber means running. Green and red appear only in diffs. Three text tones: `--label`, `--label2`, `--label3`.
- **Grounds, not borders.** Nothing inside a window is stroked. Bands are distinct grounds: `--win` for the conversation, `--tint` for the echoed prompt and file tree, `--panel` for a question, `--field` for the input and the current row, `--select` for the selection label.
- **Light theme.** `.tui-light` on `<body>` swaps every role. The ladder inverts: grounds step darker as they rise.
- **Window.** macOS-style: 38px title bar with three neutral 12px dots and a centred `project — aldwin` title in system UI 12px. 10px radius, one ambient shadow (`--shadow-window`). The only rounded corner and the only shadow.
- **Type.** One size and line height inside a window. Weight 600 only for the review title, file path, question and "Aldwin" in the launch card. System UI is used only for glyphs that render better in it (`✓ ● ○ ↩ ⎋ ↺ ⌄ ›`) and for the canvas around frames.
- **Context bar.** Ten `━` segments; filled ones ramp from 55% toward full accent so the leading edge is brightest, empty ones sit in `--track`. (An earlier note chose `▬`; the page as built uses `━`.)
- **Selection.** A blue `▎` edge, brighter text, and a one-line label above the field naming what is selected. Diff rows keep their green or red ground when selected.
- **Comments** ride at the end of their line, `◆ Use config`, like an Xcode annotation. The code is never broken up.
- **Folding.** Unchanged runs collapse to `⋯  141 lines`.
- **Motion.** The caret blinks at 1.05s, stepped. Nothing else animates; state changes are instant. Reduced motion stops the caret.
- **No imagery**, no gradients inside a window except the mark's half-block cells, no transparency, no hover states (a terminal has no pointer).

## Iconography

There are no icon files. Every mark is a Unicode character: `›` prompt and current, `✓` done or read, `●` running, `○` pending, `▎` selected edge, `◆` comment, `⋯` folded, `━` context segment, `↩` send or add, `⌃↩` approve, `⎋` stop or close, `↺` undo, `/` commands, `?` keys. If a mark is needed and it is not in this list, do not draw one.

## Brand mark

An open **A** drawn in half-block cells, 18 × 6, filled with the accent and darkening upward into the window (`Mark`). It appears only in the launch card, beside version, project, branch and model. There is no logo file and no wordmark. Elsewhere the product names itself in the title bar: `gateway — aldwin`.

## Caveats

- Geist Mono loads from Google Fonts; there are no local font files.
- `--syn` and `--call` are defined but not applied in any current frame; the review diff is not syntax-highlighted.
- Component cards and the UI kit load the generated bundle, which only exists once this project's file type is set to Design System.
