# Mjolnir

## Project Overview

Mjolnir is a Rust TUI coding agent — a discussion-first harness where the developer's understanding is the product, not the agent's throughput. It is not a mobile SDK project. Do not apply mobile SDK, FFI, Android, or iOS framing here.

Workspace: seven Cargo crates under `crates/`. Specs for all seven live in `.claude/spec/`. Read the relevant spec before working on any crate.

## Language & Platform

All code is Rust. Idioms are Rust idioms — do not translate patterns from Kotlin, Swift, or other languages. The relevant references are the Rust Book, std docs, and crate documentation (ratatui, crossterm, reqwest, rmcp, serde).

## Spec Workflow

Specs are in `.claude/spec/` — read before implementing. As of 2026-08-29, four (config, core, llm, cli) are archived under `.claude/spec/archive/` — implemented, tested, and audited with no known gaps. The remaining three (permissions, tools, tui) stay active, each with a dated Progress note on its one or two known gaps. When a spec step is completed, note it; when all steps are done, move the spec to `.claude/spec/archive/`.

## Design System

The TUI's visual design is not invented locally — it is imported. Two
projects on `claude.ai/design`, both readable via the `DesignSync` tool
(authenticate with `/design-login` first):

- **Mjolnir Design System** — `https://claude.ai/design/p/4ea574fb-4be4-47de-9940-fd38927d6dd8`
  The token layer and its `readme.md`. `styles.css` imports
  `tokens/{fonts,palette,semantic,cells,typography,elevation,motion,base}.css`.
  `semantic.css` holds the `--tui-*` roles that `crates/tui/src/palette.rs`
  mirrors one-to-one; `cells.css` holds the grid. The readme carries the
  fixed glyph vocabulary (`▌ ● ◐ ○ ✔ ▶ █ + -` — if a mark is needed and it
  is not in that table, do not draw one) and the Content Fundamentals
  (third-person "The agent", lowercase labels, sentence-case prose).
- **Agent TUI v2** — `https://claude.ai/design/p/25845063-2993-4020-ae58-4e7defc6bfef`
  The authoritative handoff bundle and the more recent of the two:
  `Agent TUI v2.dc.html`, `Agent TUI v2 Light.dc.html`, plus a revision log.
  Where this and the token project disagree, **this one wins** — it records
  later decisions (e.g. the top bar carries no accent mark).

Three things about using them, each learned the hard way:

1. **`DesignSync` is main-session only.** Subagents do not have the tool.
   Fetch the files yourself and hand over paths, not project URLs.
2. **Measure the handoff HTML; reading it is not enough.** Neither the token
   CSS nor the component prose states cell positions. They exist only as
   pixel values in the HTML's inline styles, and have to be divided by the
   cell size in `cells.css` (9×20px; the frame is 120×36 cells) to become
   grid coordinates. A design pass that skipped this step produced a layout
   that was wrong in every column while matching every colour exactly.
3. **Render it before trusting your reading of it.** Headless Chromium
   works, but under snap confinement it silently no-ops writes outside
   `/root` — copy the input there and write screenshots there too, or you
   get a reported success and no file.

`.claude/spec/mjolnir-tui.md`'s Progress entries record what was measured
and what it corrected; read the 2026-09-03 grid entry before touching
layout in `crates/tui/src/ui.rs`.

## Key Constraints (non-negotiable)

- Default-deny permissions: no tool may act without an explicit grant. No "obviously safe" carve-out.
- Edit is never allowlistable: friction on Edit is structural, not a setting.
- Discussion-first: resting state is conversation. Action only on explicit developer signal.
- No Anthropic wire types past `LlmClient`: audit at the trait boundary, not after.
