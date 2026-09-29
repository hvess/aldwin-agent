# 0011 — The workspace is the only boundary

**Status:** accepted, 2026-09-24. Superseded in part by 0014 (§1 for MCP
servers: they get no workspace root).
**Supersedes:** ADR 0008 §6 (declare reads as reads); ADR 0009 §1 (a
`run` declares a class and a read is held to it), §2 (a deny is a lock) and §3 (an unenforceable read runs
unconfined); ADR 0004 §1 (argv only, never a shell) and §7 (a deny is a
lock), and with them what ADR 0007 §2 built on argv (per-argument path
containment). **Keeps:** ADR 0007 whole otherwise — reach is a list of
roots, and every tool honours it — which is now the entire rule; ADR 0009
§4–§7; ADR 0004 §5's wording of the claim, widened below.
**Amends:** `AGENTS.md`'s non-negotiables "No arbitrary commands", "A
read-declared call is enforced" and "A deny is a lock", which are replaced
by the rule in §1–§3.

## Context

After ADR 0009 the permission model had three parts left, and each was
paying for less than it cost.

- **The class.** Every `run` declared `read` or `write`, and a read ran
  under Landlock with the tree read-only. With no prompt behind it (0009
  §1), a wrong declaration came back to the model as "declare it a write",
  and it did — the declaration protected nothing the model could not
  re-declare its way past on the next call. What it did cost was a second
  concept in every call and a tool description spent explaining it.
- **argv only.** ADR 0004 §1 removed the shell so a grant could name a
  program. With no grants, the reason was gone and the cost remained: no
  pipes, no redirection, and `sh -c` as the escape everyone used anyway.
  ADR 0007 had to contain path-like *arguments* to keep argv honest —
  `path_like`, `first_component_exists`, the incidental-path exemption —
  and said plainly that `bash -c 'cd /elsewhere && …'` walked through it.
- **The deny lock.** A `deny:` entry refused a program by name. Named how?
  By the first word of an argv, which `sh -c` or `env` or a symlink renamed
  at will. A lock that the thing it locks can step around is the
  "promises more than it delivers" ADR 0004 rejected in its own
  alternatives.

Meanwhile the one guarantee that held everywhere was ADR 0007's: every
tool resolves its paths through `Workspace`, and nothing is pointed outside
it. The developer's decision on 2026-09-24 was to make that the rule, and
to make it true of what a program *does*, not only of what it is pointed
at.

## Decisions

### §1 Every process Aldwin starts can write only inside the workspace

`run`'s shell, the language server behind `explain` (it runs build scripts
and proc macros — repository code), and every MCP stdio server run in one
sandbox (`aldwin_tools::sandbox`): **write only beneath the workspace roots
and a short incidental list; read anything; reach any network.**

- The roots are `roots[0]`, the project, plus the project
  `permissions.yaml`'s `roots:` (ADR 0007, unchanged). Every root is
  writable on the same terms.
- The incidental list is the null and random devices, `/dev/tty`,
  `/dev/shm`, `/tmp`, `$TMPDIR` and `~/.cache` — what ordinary programs
  cannot run without — and the package managers' shared stores: `$CARGO_HOME`
  (`~/.cargo`), `$RUSTUP_HOME` (`~/.rustup`), npm's cache (`~/.npm`) and
  `$GOMODCACHE` (`~/go/pkg/mod`), each at its tool's own variable when set.
  Without the stores a build that fetches a new dependency fails; the
  developer chose to allow them (2026-09-24). It is the one place a
  judgement enters the rule, and it is kept short and readable.
- Linux: Landlock. The ruleset handles every write right the kernel's ABI
  knows (ABI 1's ten, `REFER` from 2, `TRUNCATE` from 3) and no read right,
  and grants the handled rights beneath each root and incidental path. A
  rule is on an inode, so a symlink out of a root does not carry write
  access with it. macOS: `sandbox-exec` with `(allow default)`,
  `(deny file-write*)`, then `(allow file-write* (subpath …))` per root and
  incidental path, written and canonical forms both.

**Reads are open, on purpose.** A program has to read its interpreter, its
libraries and `/etc`, and the boundary exists to keep the developer's
files unchanged except through the review, not to keep them unread.
**The network is not restricted — a stated non-goal.** A command can send
what it read anywhere; confining that is a different product, with its own
allowlist and its own failure modes, and nothing here pretends to do it.

### §2 `run` takes a shell command

`{command, cwd?, timeout_secs?}`, run as `/bin/sh -c <command>`. Pipes,
redirection and `&&` work. There is no class, no program field, and no
reading of the string: `path_like`, `first_component_exists` and the
argument half of ADR 0007 §2 are deleted, because the sandbox contains what
a command does, which a reading of its text never could. `cwd` is still
resolved through `Workspace` — outside, or through a symlink that leaves,
is refused before anything is spawned. A timeout still kills the process
group and keeps the partial output (ADR 0007 §5).

### §3 Where the sandbox cannot be built, the developer is told once

On a system with neither primitive — a kernel without Landlock, a Mac
without `sandbox-exec`, any other platform — every process runs unconfined,
and the session says so **once, at startup**: *"Commands can write outside
the workspace on this system: …"*. At startup rather than at the first
`run` (ADR 0009 §3's choice) because MCP servers start with the session
and the language server with the first `explain`; the notice is about the
session, not a call.

On a system that *can* confine, a sandbox that fails to build refuses the
call — *"the sandbox could not be built, so nothing ran"* — rather than
running it unconfined. The one weakening allowed is the stated one.

### §4 Aldwin's own tools still refuse paths outside the workspace

`read`, `edit`, `explain`'s path and `run`'s `cwd` resolve through
`Workspace` exactly as ADR 0007 §3 describes, symlinks included. And
`Staging::write_all` resolves each staged path again immediately before
writing it: a directory swapped for a symlink while a review is open must
not carry an approved write out of the workspace.

### §5 `deny:` is parsed and does nothing; `aldwin-permissions` is gone

`permissions.yaml` keeps `roots:`. `allow:`, `default:` and `deny:` are
still *parsed*, uninterpreted, so every file an earlier Aldwin wrote loads —
v1's `kind:pattern` strings included, which no longer need moving aside —
and a file that says something through one is reported once at startup,
as `allow:` and `default:` were. The `aldwin-permissions` crate, which had
become the deny lock and nothing else, is deleted; the stale-key check is
`Config::stale_permissions`. The workspace is seven crates again.

## Consequences

- **The claim is one sentence and it is enforced:** nothing Aldwin runs can
  change a file outside your workspace, and nothing inside it changes
  through Aldwin's own tools except by an approved review. Where the system
  cannot enforce the first half, you are told at startup.
- **Writes the old write-class `run` could make are gone,** except into
  the incidental list. A command that must write anywhere else outside the
  workspace fails with a permission error. The package stores are on the
  list, so each is a directory where what a command writes persists
  outside the workspace — the price of a build that can fetch.
- **Landlock ABI 1 kernels (before 5.19) cannot handle `REFER`,** and on
  them the kernel refuses every rename across directories under a ruleset,
  inside the workspace too. Stated, not worked round.
- **The dispatcher knows no tool by name.** A tool's descriptor says whether
  it `observes_disk`; `run` and every MCP tool do, and the review opens
  before them (ADR 0009 §4, unchanged).
- **`run`'s input changed shape**, `{program, args, class}` to `{command}`.
  Transcripts from before carry the old shape, and the conversation's row
  for a run has to read the new one.

## Alternatives rejected

- **Keep the class, with the whole tree read-only for a read.** It protects
  nothing a re-declaration does not undo, and the notice ADR 0007 recorded
  — a model declaring `ls` a write 69 times — is what it teaches.
- **Keep `deny:` as a lock over the first word of a command.** A lock the
  command can rename its way past; the lock ADR 0004 rejected.
- **Restrict the network too.** A worthwhile product of its own, not a
  line in this one; an allowlist of hosts is a configuration surface this
  ADR does not want to invent in passing.
- **Keep package-manager stores locked**, so dependencies are fetched
  outside Aldwin. Every agent build that adds a dependency would fail.
- **A `writable:` key per project.** A configuration surface for what four
  fixed, named paths answer.
