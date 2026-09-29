# ADR 0017 — The model reloads the settings, and takes in only roots the review wrote

**Status:** accepted, 2026-09-29
**Amends:** ADR 0007 and ADR 0011 — `roots:` in `.aldwin/permissions.yaml`
stays the one way to widen the workspace; this decides who may apply it
while Aldwin runs
**Affects:** `aldwin-tools` (the `reload` tool, `Workspace::take_roots`,
`Staging::approved`), `aldwin-cli` (`/reload`), `aldwin-config`
(`PermissionsConfig::parse`), `aldwin-core` (the system prompt,
`DispatchContext::notice`)

## Context

The settings were read at startup and by the developer's `/reload-config`,
which was listed only in `/help`. The model could not reload: after an
approved edit to a settings file it had to ask the developer to type the
command, and a path outside the workspace was refused with a message
telling the developer to do the same.

The developer asked for three things: the command shorter, in the `/`
menu, and one the model can invoke itself. The last one meets a
boundary. `.aldwin/permissions.yaml` is inside the workspace, so a command
started by `run` may write it without the review; its `roots:` are the
workspace; and `run`'s sandbox reads the roots again on every call. A
reload the model could call after such a write would let it widen what it
can write without the developer seeing anything.

## Decision

### §1 `/reload`, in the menu

`/reload-config` is `/reload`, the eighth row of the `/` menu (the
developer's call, 2026-09-29). The old name still works, because the
headers of settings files written before the rename print it.

### §2 A `reload` tool

The model has a `reload` built-in: it reads every settings file again, as
`/reload` does, and applies `roots:`. It observes disk, so staged edits are
reviewed before it runs (ADR 0009 §4) and it reads what the approve wrote.
It is a built-in, not an MCP tool, because it acts on Aldwin's own state.

### §3 A new root only from the review

The review remembers what each approve last wrote (`Staging::approved`).
`reload` reads `permissions.yaml` once; when that text is exactly what the
review last wrote, its roots are taken in, parsed from that same text so no
second read can be swapped under it. Otherwise a root that would widen the
workspace is withheld and named, and waits for the developer's `/reload`;
a root that narrows it, or lies inside a current root, is applied either
way (`Workspace::take_roots`, `Widening::Withheld`). Each root is resolved
once and the path checked is the path kept, so a symlink swapped between a
check and a store cannot widen it either. The developer's `/reload` and
startup apply every root, as before (`Widening::Allowed`): they are the
developer's act.

A reviewed text is trusted once: every reload, the developer's or the
model's, forgets what approves wrote (`Staging::forget_approved`), so a
text the developer has since moved on from — restored by a `git checkout`,
say — cannot widen the workspace again.

When the model's reload changes the roots, or withholds or drops one,
Aldwin tells the developer itself (`DispatchContext::notice`), in the words
startup and `/reload` use (`TakenRoots::notice`), as ADR 0007 §1 has reach
said out loud; the model's result quotes the same sentence.

The developer chose this over withholding every new root from the model
(which would leave an approved roots edit waiting for a keystroke) and over
applying every root (which would let a command widen the boundary).

## Consequences

- An approved edit to `roots:` takes effect in the same turn: the model
  edits, the developer approves, `reload` takes it in.
- A root written by hand in an editor also waits for `/reload` when the
  model reloads; the result names it, and the prompt tells the model to say
  so.
- `provider.yaml` and `mcp.yaml` are still set up only at startup; a reload
  of either takes effect at the next start, and the tool's description says
  so.
- What an approve wrote is kept for the session only; after a restart the
  startup read has already applied the file.
