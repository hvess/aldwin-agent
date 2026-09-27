# ADR 0004 — A permission is a declared class, an enforced sandbox, and a lock

**Status:** accepted, 2026-09-20. Superseded by 0009 (§1–§4, §6, §8) and 0011 (§1, §7); §5 stands, widened by 0007 and 0011.
**Supersedes:** ADR 0001 in full. Amends ADR 0003 §2 and §4 (the option list).
**Affects:** `aldwin-permissions`, `aldwin-tools`, `aldwin-config`, `aldwin-tui`,
and the non-negotiables in `AGENTS.md`

## Context

The model this replaces keyed a grant to `kind:pattern` — a tool name and a
glob over that tool's target. ADR 0001 widened the unit from an exact argv to
a program (`shell:cargo *`) to stop a test session being one prompt per
invocation; ADR 0003 made each prompt row a sentence that quotes the rule it
would write. Both were improvements to the *statement* of a grant. Neither
touched what a grant could actually promise.

What it promised was thin, and the thinness had one source: **`shell` took an
arbitrary command string.** One tool, one opaque argument, whose real nature —
read or destroy — lived in text nobody had classified. Every consequence
followed from that:

- A grant could not carry a read/write distinction, because the tool it was
  granting had no fixed nature. ADR 0001 rejected a read/write axis on exactly
  this ground and was right to, given the shape it was reasoning about.
- A pattern grant could be walked past. `shell:cargo test*` matches
  `cargo test && anything`, because the string went to a shell interpreter and
  a glob has no concept of a metacharacter. `shell.rs` documented this as an
  accepted risk class.
- The product's claim had to be narrowed to `edit` alone, since a program-level
  grant runs build scripts and rewrites trees.

Reopened from first principles in discussion on 2026-09-20. The question that
moved it was not "how do we classify `shell`" but "should a tool that runs
arbitrary unclassified commands exist at all".

## Decision

### 1. There is no arbitrary command. A program runs only if a grant names it

`shell` is replaced by a tool that takes a **program and an argument list**,
and the argv is executed directly — `execve`, not `sh -c`. `&&`, `|`, `;`,
backticks and `$(…)` are therefore not syntax; they are ordinary argument
characters with no power to chain a second command onto an approved first one.

`sh` and `bash` are programs like any other. Granting one is granting arbitrary
execution, which is now a visible, deliberate act rather than the default
condition.

Pipelines and redirection are lost. If they are needed later they return as a
structured list of stages, each stage a program with its own grant — not as a
string.

### 2. A grant is a program and a class

| entry | meaning |
| --- | --- |
| `git: read` | any `git` invocation that is a read runs |
| `cargo: write` | any `cargo` invocation runs; `write` includes `read` |

The class belongs to the **call**, not to the program — `git status` is a read
and `git push` is a write, and they are the same binary. This is the correction
that makes a read/write axis sound where ADR 0001 found it unsound: the axis
was never wrong, it was being applied at the wrong granularity.

### 3. Three classes, and `edit` is not grantable

`read` observes. `write` changes something and cannot be shown as a diff
first. `edit` modifies files and **always** shows a diff — it is outside the
permissions model entirely, reachable by no scope, no default, and no list.
Aldwin's `edit` tool holds it; an MCP tool may hold it once the developer has
said which argument is the path and which is the content, which is later work.

Until that exists, the guarantee is worded narrowly and honestly: *Aldwin's
`edit` tool never lands without a diff you accepted.* An MCP server you have
granted `write` can modify files without one, and the documentation says so.

### 4. The agent declares the class; the sandbox enforces it

Every tool call carries a class declared by the agent, which has the command in
front of it and knows that this `git` is `status` and not `push`.

The declaration is never trusted. A call declared `read` executes **for real,
in a sandbox with the source tree read-only and the network unreachable**. If
it completes, it was a read — demonstrated, not predicted. If the kernel
refuses it, we stop and ask the developer whether to allow it as a write; on
yes it re-runs with the tree writable, which is safe precisely because nothing
landed the first time.

This is why the declaration needs no veto list, no shipped table of read-safe
invocation shapes, and no trial run. A wrong declaration cannot cause damage.
It causes a failed call and a question.

Two judgement calls remain ours and are shipped as readable data: the
**incidental-write allowlist** (`.git/index`, `$TMPDIR`, build caches) without
which `git status` cannot run as a read, and the fallback on platforms with no
enforcement primitive — where a `read` grant cannot be honoured, so every call
asks.

### 5. Scope says where a rule applies; the project root bounds what is reached

Four scopes: **turn**, **session**, **project**, **global**. Global means *in
every project*, not *anywhere on the filesystem*.

Reach is a separate, structural boundary: every tool's path arguments must
resolve inside the project root, which is the directory Aldwin was launched
in. No grant can point a tool out of the tree.

This bounds what we point at, not what a running program does. `cargo` writes
`~/.cargo`; anything with a network grant can send what it read anywhere. The
claim is *no tool is pointed outside your project by us*.

### 6. Each scope carries a default rung: `ask` → `read` → `write`

The standing answer for anything no entry covers. `ask` prompts for everything;
`read` runs read-classified calls; `write` runs both. `edit` asks under all
three — it is the floor, not a rung.

When two scopes disagree, **the narrower file wins outright**, in both
directions. A project may be opened up without loosening every project, and
locked down without touching the global file.

### 7. A deny is a lock

A deny cannot be overridden by anything narrower — not a project file, not a
session, not a single turn. The prompt does not offer allow; it says where the
rule lives. Changing it is a deliberate edit to that file, made outside the
moment that wanted it.

Explicit entries outrank the default rung in both directions: a denied program
stays denied under `write`, an allowed one runs under `ask`. Within one scope,
deny beats allow.

### 8. The prompt is eight rows, one list

Four allow tiers and four deny tiers, mirrored, each row stating the rule it
writes:

```
1  allow once                           5  deny once
2  allow git writes for this session    6  deny git writes for this session
3  always allow git writes here         7  deny git writes in this project
4  always allow git writes everywhere   8  never allow git
```

Drawn as a single vertical list, not two columns. Rows 2–4 and 6–7 are
class-specific, so a `git: read` grant survives a `git: write` deny. Row 8 is
the whole program, everywhere, and is the lock of §7.

This amends ADR 0003 §2, which had five rows and no deny tiers, and §4, whose
single `Deny` row is now four. ADR 0003's §1 stands unchanged and governs
these rows: each is one sentence that states its own rule.

## Consequences

**The product can make a stronger claim than before, and a different one.**
Not "the model is careful" but "a call that claims to be a read is executed
where writing is impossible". That is checkable by a developer who does not
trust us, which is the only kind of claim worth making here.

**Capability is genuinely lost.** No pipelines, no redirection, no shell
one-liners, no program that nobody has granted. Some of that returns as
structured pipeline stages; some of it is simply the cost of §1, taken
deliberately.

**A sandbox is now a core component**, not a detail of the permission engine.
It is the thing the whole model rests on: without enforcement, §4 degrades to
trusting the agent's word, and every other decision here degrades with it.

**Platforms without an enforcement primitive get a worse product**, honestly
labelled. Linux has Landlock. macOS has an equivalent. Windows effectively does
not, and there a `read` grant cannot be offered at all.

**Every persisted grant from the old model is meaningless** — `kind:pattern`
described a tool and a glob, and neither survives. This is the first change in
the project's history with no compatible reading of existing files.

## Alternatives rejected

- **Keep arbitrary `shell`, classify per program from a shipped table.** The
  table is a soundness cliff: `git` mutates, `find -exec` runs anything, and a
  chained string starts read and ends destructive. It labels a command a read
  and is sometimes wrong, which is the worst failure mode available.
- **Delete `shell`, replace it with purpose-built tools** (`git_status`,
  `run_task`). True classifications, real capability loss, and constant
  pressure to reintroduce an escape hatch.
- **Trust the agent's declaration outright.** Usable, and it makes the
  guarantee "the model is right, and nothing it read talked it out of being
  right" — in a harness whose job is reading untrusted repository content.
- **Trust the declaration, with a shipped veto list** of shapes that can never
  be a read (`git push`, `rm`, `--force`). Cheap and closes the worst tail,
  but it is our judgement doing the work, and it is a list that is wrong the
  first time someone writes a destructive program we did not anticipate.
- **Run the command in a sandbox first to predict its effect, then run it for
  real.** Doubles the cost, is defeated by non-determinism, and a filesystem
  diff cannot see the effect that matters most: `curl -X DELETE` and `git push`
  change nothing locally.
- **Most-restrictive-wins for the default rung.** Makes a single project
  impossible to open up without loosening every project.
- **A deny as a pre-answer rather than a lock.** A denylist anything can shrug
  off is not a guarantee, and was rejected for the same reason the whole model
  was reopened: it promises more than it delivers.
