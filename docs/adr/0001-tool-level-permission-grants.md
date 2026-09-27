# ADR 0001 — Grants are per tool and per program, not per command string

**Status:** superseded in full by ADR 0004, 2026-09-20

> **Note, 2026-09-20.** Everything this record decides has been replaced.
> `shell` no longer exists, so the grant unit it reasons about has no
> referent, and §5's three-point access scale is now the `ask`/`read`/`write`
> rung held in `permissions.yaml` rather than a preset that expands into
> grants. The body is left as written because its central argument — that a
> read/write axis is unsound over a tool taking a whole command line — is
> correct, and ADR 0004 answers it by deleting the tool rather than by
> disagreeing.

**Originally accepted:** 2026-09-06
**Supersedes:** the exact-argv grant unit described in `aldwin-permissions.md`
**Affects:** `aldwin-permissions`, `aldwin-tui` (prompt copy, first run), `.aldwin/permissions.yaml`

## Context

A grant is persisted as an opaque `kind:pattern` string — `read:./crates/tui/src/**`,
`shell:cargo test -p gateway limit:: -- --nocapture`. The engine matches
`kind` exactly and `pattern` as a glob against a per-kind target: a file path
for path-shaped tools, the assembled argv for shell-shaped ones.

For `read` that works: the pattern is a directory glob, so a grant reads as
"this tool, in this directory". For `shell` it does not. The target is the
whole command line, so the unit of consent is one exact invocation. Approving
`cargo test -p gateway` says nothing about `cargo test -p tui`, and a session
spent running a test suite is a session spent answering the same question with
different arguments.

That was reported directly, and the note survives in `app.rs`: *"are we
approving the tool? the directory?"*. The previous pass answered it by
**stating** the grant precisely rather than changing its shape — `GrantSummary`
now names the literal rule a save would write. That fixed the ambiguity without
fixing the friction.

Two constraints bound any answer:

- **Default-deny.** No tool may act without an explicit grant, and there is no
  "obviously safe" carve-out (`AGENTS.md`).
- **Edit is never allowlistable.** Enforced structurally, not by policy:
  `Engine::check_tool` returns `PromptRequired(PromptPayload::Edit)` for
  `edit_class` tools *before* consulting any allow or deny list
  (`engine.rs:92`).

## Decision

### 1. Tools are classified three ways, and the class decides the grant unit

| Class | Tools | Grant unit | Grantable? |
| --- | --- | --- | --- |
| Non-mutating | `read`, `explain` | the tool, over a directory | yes |
| Executing | `shell` | the **program** (`argv[0]`), over a directory | yes |
| Mutating | `edit` | — | **never** |

The engine's matching is unchanged. `kind:pattern` stays, and so does the glob
matcher — what changes is which patterns the TUI *offers*. A `shell` prompt for
`cargo test -p gateway limit::` now proposes `shell:cargo *`, not the argv.

This is what the design system's own permission copy has said all along:
*"Always allow `cargo *` in this project"*. Program plus wildcard. The code was
implementing its top tier one notch narrower than the copy described.

### 2. Editing is de-scoped from the permissions model entirely

`edit` is not a tier, not an option, and not expressible as a grant. Every edit
is a conscious approval with a diff, always. `engine.rs:92` is the enforcement
and does not move.

This is what makes the rest of the model safe to coarsen: the blast radius of a
tool-level grant is bounded by the fact that the one tool that exists to modify
the tree cannot be granted at all.

### 3. Scope is directory × duration, which the existing scopes already are

"This directory" is the project scope — `.aldwin/permissions.yaml` in the
project root. "Always" is that file; "session" is the in-memory session grant
already held by the engine. No new scope is introduced; the three that exist
(session, project, global) are relabelled in the UI to say what they mean.

### 4. A directory with no permissions file is a first run

Entering a project that has no `.aldwin/permissions.yaml` is the trigger for
the first-run screen (design system screen `5d`). The screen asks two questions
— model, then access posture for this directory — and writes both.

### 5. The access scale is three points, not the design's four

The design system specifies a four-point scale, "in order, widening", and is
explicit that it is "not a three-level dim line and not a slider". Its wording
assumed writes were grantable:

> `ask` · `read` (reads run, writes and commands ask) · `write` (reads and
> writes run, commands ask) · `all` (everything runs, nothing asks)

With editing de-scoped there is no write tier to have, and `write` collapses
into `read`. A fourth point was drafted as `run` — "reads run; each new program
asks once, then runs" — and then dropped, because it describes no distinct
state: program-level grants make "asks once per program, then remembered" the
*default* prompt behaviour, so `run` and `read` would persist an identical set
of grants and differ only in prose.

The tiers are therefore the three states that genuinely exist, distinguished by
what is written to `permissions.yaml`:

| Point | Grants written | Meaning |
| --- | --- | --- |
| `ask` | none | every tool asks, every time |
| `read` | `read:**`, `explain:**` | reads run; commands and edits ask |
| `all` | those plus `shell:*` | reads and any command run; edits ask |

Edits ask under all three — that is the floor, and it is not a tier.

**This is a deliberate deviation from the design system**, and the design is
right to resist it in general: a three-point scale was explicitly rejected there.
The reason it wins here is that the fourth point could only be bought by
inventing policy the harness does not have (pre-granting a guessed set of
"project" commands, or making one tier refuse to persist shell answers). A named
choice that writes the same config as its neighbour is a worse outcome than one
fewer choice. If a fourth honest state appears later, the scale should grow back.

## Consequences

**The claim the product can make gets narrower, and should be stated that way.**
Not "nothing writes without your approval" — a program-level shell grant can
still write. `cargo test` executes build scripts and test code, `git checkout`
mutates the tree, `find -exec` runs anything. The true claim is **"no `edit`
lands without a diff you accepted"**. The design system's readme currently says
*"no write lands without a diff the user has accepted"*, which overstates it and
wants rewording to name `edit`.

This was considered and rejected as fixable: classifying shell commands as
read-only *looks* tractable and is not soundly enforceable, because the commands
a developer most wants to allowlist are exactly the ones that run arbitrary code.

**Anyone wanting the stronger claim must keep `shell` per-command.** That option
remains open and is a one-line change to what the TUI offers; it is not taken
here because the friction it buys is the friction that was reported as the
problem.

**Existing grants keep working.** `kind:pattern` is unchanged and the matcher is
unchanged, so every persisted entry — including exact-argv ones written by
earlier builds — still matches exactly as before. There is no migration. New
grants are simply coarser than old ones, and an old exact-argv entry is a valid,
if narrow, program grant.

**A program-level grant is coarser than what the developer literally saw.** The
prompt must therefore state the grant it would write, not the command that
triggered it — the discipline `GrantSummary` already exists to enforce. Widening
the unit makes that statement more important, not less.

## Alternatives rejected

- **Decompose `shell` into narrow tools** (`cd`, `ls`, `grep`, …). Per-tool
  approval becomes meaningful, but a general `shell` is still needed as an
  escape hatch, which reintroduces the same problem behind a longer tool list.
- **A read/write axis over `shell`.** Unsound: `ls` reads and `rm -rf` writes,
  and only the command string distinguishes them — precisely the information
  tool-level approval discards.
- **Allow write-always grants.** Would contradict `AGENTS.md`'s Edit constraint
  and the product's own premise. Explicitly declined: editing is de-scoped
  instead, which keeps both intact.
