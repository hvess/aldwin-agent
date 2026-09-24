# aldwin-permissions

The lock — a `deny:` entry refuses a program outright and nothing narrower
overrides it — and the reach a `roots:` entry declares. That is all this
crate holds since ADR 0009.

**Status:** closed — the crate is deleted, 2026-09-24. See the closing note.
**Scope:** aldwin-permissions crate. The deny lock and the compatibility
read of the keys nothing consults any more. Excludes the sandbox, the
staging area and the review (all aldwin-tools), YAML I/O (config) and
rendering (tui).
**Owner:** Maximilian
**Last Updated:** 2026-09-24

**Closing note (2026-09-24, ADR 0011 — the workspace is the only
boundary):** The deny lock was the last thing this crate held, and ADR 0011
removed it: a lock keyed on a command's first word is one the command can
rename its way past, and with `run` now a shell command there is not even a
first word to key it on. Every process Aldwin starts runs in a sandbox that
can write only inside the workspace, and that — with every tool resolving
its paths through `Workspace` — is the whole boundary. The crate is
deleted. `deny:`, `allow:` and `default:` are still parsed so old files
load, and reported once at startup (`Config::stale_permissions` in
aldwin-config, wired in aldwin-cli's bootstrap). `roots:` is unchanged. What
follows is the crate as it stood under ADR 0009, kept for its reasoning.

**Progress (2026-09-23, ADR 0009 — the review is the only gate):** The
engine of ADR 0004 is gone. Reads and runs need no grant and never ask; the
sandbox holds a read declaration to its word and a refused read goes back to
the model as an error; an edit is staged and reviewed whole at the end of
the turn or before any run that would observe it. What was left of the
permission model after that is the one clause no answer at a prompt could
ever have lifted — **a deny is a lock** (ADR 0004 §7) — and this crate is
now exactly that.

- **`Engine` became `Locks`.** `check(program, class)` answers
  `Outcome::Allow` or `Outcome::Locked { scope, rule }` and nothing else.
  There is no session layer, no `record`, no `set_rung`, no
  `effective_view`, and no `prompt.rs`: `Choice`, `PromptPayload`,
  `PromptResponse` and `ContextFileTier` are deleted, not moved.
- **`permissions.yaml` still parses `allow:` and `default:`.** A file the
  previous first run wrote carries both, and `deny_unknown_fields` would
  otherwise stop every existing project from starting. `Locks::stale_keys`
  reports which scope carries one; aldwin-cli says so once at startup. They
  are never written (`skip_serializing_if`) and never read. A later format
  version drops them.
- **Context files are not this crate's business any more.** `CLAUDE.md`
  and `AGENTS.md` are read into the context because reading is a read;
  `context_files.yaml` remains a config domain nothing consults.
- **`roots:` is unchanged** (ADR 0007): reach, not a grant, project scope
  only, applied by aldwin-cli at startup and on `/reload-config`.

The spec that described ADR 0004's engine — precedence, the eight-row
prompt, the standing rung, the session layer — is in git under this file's
previous revision, and ADR 0004 itself records the model. Neither is needed
to work on this crate now, which is 100 lines and its tests.

## Interfaces

```rust
pub struct Locks { /* Config */ }
impl Locks {
    pub fn new(config: Config) -> Self;
    pub fn check(&self, program: &str, declared: Class) -> Outcome;
    pub fn all(&self) -> Vec<(LockScope, GrantEntry)>;
    pub fn stale_keys(&self) -> Vec<LockScope>;
}
pub enum Outcome { Allow, Locked { scope: LockScope, rule: GrantEntry } }
pub enum LockScope { Project, Global }   // where_it_lives() names the file
```

`Class` and `GrantEntry` are re-exported from aldwin-config. A deny of class
`C` blocks every call at or above `C`; a bare program blocks every class.
The nearest scope's lock is the one named, because it is the one the
developer can most easily change.

## Decisions

- **Two answers, no third.** "Ask" left with the prompt. A call either runs
  or is locked, and a locked call's refusal names the file.
- **Stale keys are reported, not honoured and not rejected.** Honouring
  `allow:` would be a grant nothing asks for; rejecting the file would
  strand every project the old model touched.
- **No session deny.** There is no prompt at which one could be made, and a
  lock that is not in a file is not a lock the developer can find.

## Pitfalls

- Reintroducing an allow list "just for one program" reintroduces the
  question of who answers when it is missing. The answer under ADR 0009 is
  the sandbox and the review, and an allow list has no role in either.
- `Class::Edit` still exists in aldwin-config and still cannot be
  deserialised. It is not a class a deny can name, and it is not consulted
  here: an edit reaches the review, never this crate.

## References

- `.claude/adr/0009-the-review-is-the-only-gate.md` — why this crate is a lock.
- `.claude/adr/0004-permissions-are-a-declared-class-an-enforced-sandbox-and-a-lock.md` — §5 and §7, the two clauses that stand.
- `.claude/adr/0007-reach-is-a-workspace-and-every-tool-honours-it.md` — `roots:`.
