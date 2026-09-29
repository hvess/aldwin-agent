---
name: design-sync
description: How to re-sync the Aldwin design system from claude.ai/design with DesignSync — which of the two projects holds what, and the traps in addressing them. Read before any DesignSync call or design re-sync; not needed to use the local copy in docs/design/.
---

# Re-syncing the design system

The local copy in `docs/design/` (see its `IMPORT.md`) is the working
source. Re-sync only when you need something that copy doesn't carry.

## The two projects

- **"Aldwin"** — `https://claude.ai/design/p/b9de8837-c1b6-4be1-8668-4dee1c585de5`
  `type: PROJECT_TYPE_DESIGN_SYSTEM`. The design system: `README.md`,
  `tokens/`, `guidelines/`, `components/`, `ui_kits/`. The token files are
  the authority for values. It is the only project `list_projects` returns.
- **"Design system tokens discussion"** — `https://claude.ai/design/p/25845063-2993-4020-ae58-4e7defc6bfef`
  `type: PROJECT_TYPE_PROJECT`. Holds **`Aldwin Agent TUI.dc.html`**, the ten
  frames every token value is copied from, plus a copy of the design system
  under `Aldwin Design System/` and a bound copy under `_ds/aldwin-b9de8837-…/`.
  The three copies of `tokens/` were byte-identical on 2026-09-23. The
  Mjolnir-era frames survive under `uploads/AI agent harness TUI design/`.

The frame is the authority on positions (every one is a `var(--…)` from
`layout.css`, so measuring is a token lookup) and on the brand mark, which
exists nowhere else. Fetch it when either question comes up; it is 92 KB.

## Traps in addressing them

Each learned the hard way:

1. **`list_projects` only returns design-system projects.** The discussion
   project — where the frame lives — never appears in it. Address it by UUID.
2. **A stale `updatedAt` proves nothing.** Editing a bound `_ds/` copy does
   not touch the source project.
3. **`DesignSync` is main-session only.** Subagents do not have the tool.
   Fetch the files yourself and hand over paths, not project URLs.
4. **Authenticate with `/design-login`** before the first `DesignSync` call.
5. **A large `get_file` is persisted to a tool-results file, not returned
   inline.** Extract its `content` field with a script; do not paste it.
6. **Render before trusting a reading.** Firefox headless works on this
   machine (`firefox --headless --screenshot out.png file:///…`); build a
   standalone page by inlining the token CSS, since the frame links
   `styles.css` relatively and `<x-dc>`/`{{ }}` are the host's templating.
