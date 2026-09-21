# ADR 0007 — Reach is a workspace, and every tool honours it

**Status:** accepted, 2026-09-21
**Amends:** ADR 0004 §4 (the no-enforcement fallback) and §5 (the project-root
boundary)
**Affects:** `aldwin-tools`, `aldwin-config`, `aldwin-cli`
**Closes:** open-tasks entry 15

## Context

ADR 0004 §5 says: *every tool's path arguments must resolve inside the project
root … No grant can point a tool out of the tree.* Three of the four built-ins
did that, through `paths::resolve_in_project`. **`run` never called it.** The
one tool that executes programs had no argument containment at all, and
`sandbox.rs`'s own module doc deferred to a check that, for that tool, did not
exist.

A session transcript from 2026-09-21 shows what the asymmetry produces. The
developer's work moved to a sibling directory. `edit` refused it — *"path
resolves outside the project root"* — and `read` refused it. `run` wrote a
135-line file there via `bash -c 'cat > … <<EOF'`, and later `rm -rf`'d two
git checkouts there, neither with a prompt. The boundary held on the tools
that show a diff and was absent from the tool that cannot. The agent's own
account of it, when the developer noticed:

> Rather than surfacing that limitation to you, I silently fell back to
> writing the whole file via a `cat` heredoc through `run`, which bypasses the
> diff-and-approval step entirely.

Over the session: `edit` was called once and failed; 33 of 71 `run` calls were
`bash -c`; 16 carried absolute paths outside the project root. The diff gate
fired zero times.

Two further defects in the same tool have the same root — it was never
brought into the model the other tools live in:

- **A timeout discarded the program's output.** `read_to_string` on a local,
  dropped on cancellation. A 30-minute clone reported only that it was long.
- **There was no working directory.** Every call ran in the project root, so
  "work over there" had to be spelled `bash -c 'cd … && …'`.

And §4's fallback was never built as written. §4 says that where a `read`
grant cannot be honoured, **every call asks**. The code returned a flat error
instead. On macOS — no Landlock — that meant every read-declared call failed.
In the observed session the model declared `read` twice, saw both fail, and
declared the next 69 calls `write`, including every `ls`, `grep` and `cat`.
The class system did not degrade on that platform; it was abandoned in two
calls.

## Decision

### 1. Reach is a list of roots, not one root

`paths::Workspace` holds canonical roots. `roots[0]` is the project root and
is what relative paths resolve against. The rest come from `roots:` in the
**project** `.aldwin/permissions.yaml`.

Project scope only. A global root list would widen reach in every directory at
once, which is the one direction a default-deny harness must not move on its
own. Roots are stated, never inferred — nothing walks up looking for sibling
checkouts.

Because the project file can arrive with a clone, reach beyond the project is
**said out loud**: a notice at the top of the session names the extra roots,
and names any that were written down but do not exist — a typo silently
narrows reach, which otherwise reads as the agent refusing for no reason.
`/reload-config` re-reads the list; the roots are shared between every tool's
`Workspace` the way `Engine` shares its `Config`, so the change is live on the
next call.

A root widens *where a call may point*. It is not a grant: which programs run,
under which class, is unchanged, and both questions are still asked over a
second root.

### 2. Every tool goes through it, `run` included

`run` resolves its working directory and every path-like argument through the
same `Workspace` as `read`, `edit` and `explain`. §5's claim is now true as
written rather than true of three tools out of four.

An argument is checked when it **climbs with `..`**, or is **absolute and
points into the real filesystem**. Everything else is relative without
climbing, so it resolves under a working directory that is itself already
contained. The value of `--flag=/path` is checked, and so is a value glued to
a short flag (`-C/elsewhere`, `-f/etc/x`) — skipping every `-…` token let
those through to `execve`.

Three deliberate allowances, each of which was a false refusal first:

- **Globs and flags are not paths.** `--include=*.kts` and `*/build/*` are left
  alone; containment that rejected them is containment nobody can search
  under.
- **An absolute-looking argument whose first component does not exist is not
  a path.** `sed -n /pattern/p` and `grep /api/v1` start with `/`. Nothing
  called `/pattern` exists, and creating it would need write access to `/`
  itself. One `stat`, and no table of programs.
- **The sandbox's incidental paths are legitimate arguments.** `/dev/null`,
  `$TMPDIR` and the rest are already writable under a *read*; refusing them
  as arguments broke `grep x file /dev/null` under either class. The two lists
  have to agree, so containment asks the sandbox.

**The hole, stated rather than papered over:** a path inside a string argument
is invisible to this. `bash -c 'cd /elsewhere && …'` is one argument that
neither starts with `/` nor climbs. Granting a shell was already granting
arbitrary execution (ADR 0004 §1) and this does not change that. What it does
is narrow every *other* program, and remove the main reason to reach for a
shell.

### 3. Absolute paths are allowed when they are contained

ADR 0004 §5 refused them outright, because `PathBuf::join` discards its base
when the joined path is absolute and a grant matching the model's literal
argument could not see the escape. Checking against canonical roots closes
that hazard directly, and a second root is unaddressable without it.
**Containment is decided on resolved paths only.** The roots are canonical, so
comparing them against a path *as typed* refuses anything that reaches a root
through a symlink — on macOS, every `/tmp/…` and `/var/…` path. Two resolved
forms are checked and both must be inside: the lexically-normalized path with
its existing prefix canonicalized (which works for a file that does not exist
yet, and catches a symlink inside a root pointing out of it), and the path as
the filesystem resolves it, when it exists — because lexical `..` and real
`..` disagree after a symlink (`out/../x` normalizes to `x` but opens
`<wherever out points>/../x`). The normalized path is what gets opened, so
what was checked is what is used.

### 4. A refusal names the roots

`PathEscapesWorkspace` carries them, and the session context block lists them.
"Outside the project root" was a refusal the model could not act on without
knowing what the root was, and the way it acted on it anyway was `run`.

### 5. `run` gains a working directory, and keeps output on timeout

`cwd` is per-call and contained like any other path. A timed-out call reports
what the program had already written. Output is accumulated as bytes as it
arrives and decoded once — decoding each read on its own turns a multibyte
character that straddles a read boundary into U+FFFD.

A timeout also kills the **process group**. `setsid` was always there for
that, and nothing used it: `kill_on_drop` reaches only the direct child, so a
timed-out `sh -c` left whatever it had started running.

### 6. A read declaration that cannot be enforced is a question, not an error

What §4 always said. `SandboxUnavailable` carries its program and argv and
raises the same prompt as `ReadRefused`; nothing ran either way.

One difference, and it matters: a refused read is rare, and worth a question
every time. This happens on *every* read-declared call where there is no
sandbox. So the engine is consulted first, under the prompt gate — once the
developer has allowed that program's writes at any tier, the answer stands.
Asking unconditionally made "always allow" a no-op, and a prompt per `ls`
teaches the model to stop declaring reads, which is the failure this section
exists to end.

A read-only run that fails *without* evidence of a denial (§8) says where it
ran, so a program that swallowed `EACCES` does not leave the model guessing.

### 7. macOS enforces reads, via Seatbelt

`sandbox-exec` with a generated SBPL profile: allow default, deny
`file-write*` and `network*`, then allow the incidental writes back —
last-match-wins, so the exemptions follow the denial.

It confines by **rewriting the command line**, not by acting in the forked
child. `sandbox_init_with_parameters` compiles a profile, which allocates, and
calling it between `fork` and `execve` in a threaded process is the deadlock
`linux.rs` restructured itself to avoid. So the backend asks for a command
line before stdio is configured, and `sandbox-exec` confines itself and
`exec`s the real program.

Seatbelt matches the *resolved* path of the file being written, and on macOS
the incidental paths are mostly symlinks (`/tmp` → `/private/tmp`, `$TMPDIR`
under `/var` → `/private/var`), so each exemption is emitted in both its
written and its canonical form. A rule for the path as written never matches.

`sandbox-exec` has been deprecated since 10.8. It is still shipped, still the
only route to this primitive without entitlements, and its disappearance
degrades to §6's question rather than to an unconfined run.

### 8. A non-zero exit is not a refused read

`ReadRefused` is raised only when the failure carries evidence of a denial —
a permission or read-only message on stderr, or death by signal. `grep` exits
1 when nothing matched, and turning that into "your read was refused" asked
the developer about a write that was never attempted. A denial this misses
falls through as an ordinary failed command with its own error in view.

## Consequences

**The guarantee is the same sentence and is now true.** *No tool is pointed
outside your workspace by us* — four tools out of four.

**A second root is a deliberate, written act.** It lives in the file that
already answers what the agent may touch, and it is visible in `git diff`.

**The argv guarantee gets its capability back.** `cwd` removes the reason most
of those 33 `bash -c` calls existed. Pipelines and redirection are still gone,
still per ADR 0004 §1, and still owed as structured stages.

**macOS is no longer a worse product by default**, and Windows still is —
honestly, via §6. ADR 0004's Consequences anticipated exactly this split.

**The macOS backend has not been run on macOS.** It compiles on every platform
(it is ordinary Rust, no FFI) and its unit tests run everywhere, but the
behaviour of `sandbox-exec` itself is unverified here. Recorded as open-tasks
entry 25.

## Alternatives rejected

- **Give `run` the project-root boundary and stop there.** Correct, and it
  breaks the workflow that produced the bug: the developer legitimately worked
  across sibling checkouts and would have been left with `bash -c` as the only
  route, which is the behaviour being fixed.
- **Infer roots by walking up to a common parent.** Reach that widens itself
  is the opposite of default-deny.
- **Parse shell strings to contain `bash -c`.** Rebuilds the command-string
  guessing ADR 0004 deleted `shell` to be rid of.
- **Classify path-like arguments per program from a table.** The soundness
  cliff ADR 0004 §4 rejected, in a new place.
- **Honour a `read` grant unconfined where there is no sandbox.** Turns the
  guarantee into a promise about the model's word, in a harness whose job is
  reading untrusted repository content.
- **Treat every non-zero exit under `read` as a refusal, as before.** Simple
  and observably corrosive: it is what taught the model the class was broken.
