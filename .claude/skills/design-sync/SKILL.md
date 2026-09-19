---
name: design-sync
description: How to re-sync the Mjolnir design system from claude.ai/design with DesignSync — which of the two projects is live, and the four traps in addressing them. Read before any DesignSync call or design re-sync; not needed to use the local copy in .claude/design/.
---

# Re-syncing the design system

The local copy in `.claude/design/` (see its `IMPORT.md`) is the working
source. Re-sync only when you need something that copy doesn't carry.

## Which project is live

There are two projects on `claude.ai/design`, and **which one is live is
not obvious**:

- **"Design system tokens discussion"** — `https://claude.ai/design/p/25845063-2993-4020-ae58-4e7defc6bfef`
  `type: PROJECT_TYPE_PROJECT`. Holds the five `.dc.html` frames *and a
  bound copy of the design system* under
  `_ds/mjolnir-design-system-4ea574fb-…/`. **That bound copy is the current
  token layer.** Its `SYNC.md` is the change record and carries an explicit
  "Not applied — outside this copy" table.
- **"Mjolnir Design System"** — `https://claude.ai/design/p/4ea574fb-4be4-47de-9940-fd38927d6dd8`
  `type: PROJECT_TYPE_DESIGN_SYSTEM`. The *source* project. **Partly synced
  on 2026-09-07** — its token layer, `README.md`/`readme.md`, manifest and
  guideline cards were brought to Turn 15; its `components/`, `_ds_bundle.js`,
  `ui_kits/` and `templates/` still state pre-Turn-13 rules. `IMPORT.md`'s
  "The source project is no longer wholly stale" section lists exactly which
  is which. Still read values from the bound copy, not from here.

## Traps in addressing them

Each learned the hard way:

1. **`list_projects` only returns design-system projects.** The discussion
   project — the live one — never appears in it. Address it by UUID.
2. **A stale `updatedAt` proves nothing.** Editing the bound `_ds/` copy
   does not touch the source project, so `4ea574fb-…` can sit at an old
   date while the design moves underneath it.
3. **`DesignSync` is main-session only.** Subagents do not have the tool.
   Fetch the files yourself and hand over paths, not project URLs.
4. **Authenticate with `/design-login`** before the first `DesignSync` call.
