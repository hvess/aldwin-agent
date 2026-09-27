# ADR 0013 — Aldwin is a co-author of the commits it makes

**Status:** accepted, 2026-09-27 (the developer's decision, and the
mechanism is theirs too).
**Supersedes:** nothing.
**Affects:** `aldwin-cli` (`main.rs`, `git_shim.rs`, `bootstrap::run` and
its startup notices), `.claude/spec/archive/aldwin-cli.md`,
`.claude/spec/aldwin.md`

## Context

Aldwin commits in a developer's project the way it does anything else that
leaves the conversation: through `run`, whose shell calls `git`. Those
commits carried the developer's name and nothing else, so a history read a
month later could not tell a commit the developer made from one Aldwin made
on their behalf. The repository Aldwin is built in already signs its own
commits with a trailer; a user's project had nothing.

The fact to record is small and has a standard shape: git's
`Co-Authored-By:` trailer, which the forges already read. The question was
only where to put it so that it is always there.

## Decision

**Every commit Aldwin creates with `git commit` carries the trailer
`Co-Authored-By: Aldwin <noreply@aldwin.codes>`.** It is added by a `git`
shim on the `PATH` of every process Aldwin starts.

1. **Aldwin's own binary is the shim.** `main` looks at the name it was
   started under before anything else — before clap, the runtime and the
   TUI. Started as `git`, it finds the real git by walking `PATH` in order,
   skipping every entry whose `git` resolves to a build of Aldwin, and
   `exec`s it with the arguments unchanged — except that when the
   subcommand is `commit`, `--trailer` and the trailer go in right after
   it. The subcommand is found by stepping over git's global options: those
   that take the next word (`-C`, `-c`, `--git-dir`, `--work-tree`,
   `--namespace`, `--config-env`, `--super-prefix`, `--attr-source`), and
   every other word starting with `-`. With no real git behind it the shim
   says so in one sentence and exits 127, as a shell does for a missing
   command.

2. **The shim is installed once, at startup, while `main` is the only
   thread.** A private directory (`0700`, under the temp dir) gets a
   symlink named `git` to the running binary, and goes first on the
   process's own `PATH`. Everything Aldwin starts inherits it: `run`'s
   shell, the language server, each MCP server. The sandbox reads anywhere
   (ADR 0011), so the directory and the real git are reachable from inside
   it; a test commits through the real `run` under Landlock to hold that.
   The directory is removed on a clean exit.

3. **A shim that could not be installed is said once.** Aldwin still
   starts, and the developer hears "Commits made in this session will not
   name Aldwin as a co-author", with the reason, at the top of the session
   beside the unconfined-sandbox notice (ADR 0011 §3) — never silently.

4. **Git decides whether the trailer is already there.** Its default
   `trailer.ifExists` (`addIfDifferentNeighbor`) does not add a trailer the
   message already ends with, compared case-insensitively on the key, so a
   message that names Aldwin itself is not given it twice. Aldwin never
   reads the message's words: it reads only whether a message given with
   `-m` or `--message` is empty (Limits).

5. **Unix only, as the sandbox is.** The shim needs `exec` and a symlink.
   Elsewhere nothing is installed and nothing is said: there is no
   mechanism to have failed.

## Requirement

`git commit --trailer` arrived in **git 2.32** (June 2021). An older git
refuses the option, so shimmed, every `git commit` from inside a session
would fail. Startup asks the git on `PATH` for its version, and before 2.32
it installs no shim and says so once, like any shim that could not be
installed: a commit without the trailer is better than no commit. (First
written as "recorded rather than guarded"; guarding it costs one
`git --version` at startup, and losing commits entirely on an older system
costs a user far more.)

## Limits

- **Only `git commit` is rewritten.** A commit git makes by another path —
  a merge commit, `cherry-pick`, `rebase`, `revert`, `am`, `commit-tree` —
  gets no trailer. So does an alias that expands to `commit` (`git ci`):
  the shim sees `ci`.
- **A git reached without `PATH` is not reached through the shim.** `/usr/bin/git`
  by absolute path, a tool that bundles its own git, or a library such as
  libgit2 all commit without the trailer.
- **An empty message typed into an editor becomes the trailer.** Git
  aborts a commit whose message is empty, but the trailer is added before
  that check. A message given empty on the command line — `-m ""`,
  `--message=` — gets no trailer, so git still aborts (the developer's call,
  2026-09-27); an editor that saves nothing cannot be seen from the shim,
  and makes a commit whose whole message is the trailer.
- **A developer's `trailer.*` config applies.** Git reads it for this
  trailer as for any other; a `trailer.ifExists` of `add` gives a message
  that already names Aldwin a second line.
- **An amend of the developer's commit names Aldwin.** `git commit --amend`
  is a `git commit`; Aldwin did write the new commit.

## Rejected

- **Telling the model to add it, in the system prompt.** The trailer
  would be there when the model remembered, and a fact about every commit
  that depends on the model remembering every time is not a fact. It also
  spends context on every request for something a program can do.
- **Amending new commits after a `run`.** Aldwin would compare `HEAD`
  before and after each run and rewrite what appeared. But a run can make
  a commit and push it in one command, and amending afterwards rewrites a
  commit that is already somewhere else — diverging the developer's branch
  from its remote to add a line to a message.

## Consequences

**The one global Aldwin sets is `PATH`.** quality-gate §5 asks for no global
mutable state; the shim cannot reach every process Aldwin starts any other
way, so `install` sets the process's `PATH` once, before any thread exists,
and nothing changes it afterwards.

**Aldwin's binary has a second identity.** Started as `git`, it is not
Aldwin: it reads only the flags it steps over to find `commit`, and
whether a message given there is empty, and starts no runtime. Anything that runs a
file named `git` which is a symlink to Aldwin gets git.

**Nested sessions are safe.** A session started inside another's `run`
puts its own shim in front of the first. Each shim skips any `git` that
resolves to a binary with its own file name — every build of Aldwin, as
built — not only itself, so two builds cannot hand a commit back and forth;
git's own duplicate check keeps the trailer to one line.
