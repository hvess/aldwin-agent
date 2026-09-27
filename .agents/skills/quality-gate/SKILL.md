---
name: quality-gate
description: The architectural and code-quality bar every change to Aldwin must clear before it is done — idiomatic and canonical over novel, clean, well encapsulated, modular, scalable and hexagonal. Use when designing a change, before calling any Rust change finished, and when reviewing code. Pairs with the rust skill (patterns and commands) and /review (the submission loop).
---

# Quality gate

This project always prioritises **idiomatic and canonical patterns over novel
solutions**. Code here is clean, well encapsulated, modular, scalable and
hexagonal, and it is written to be exemplary: the next change should be able
to copy it as the model for how things are done.

A change passes the gate only when every section below holds. If one does
not, fix the code — do not argue the gate down in a comment.

**Going above and beyond means making it lean and simple.** Adding a layer
of abstraction is easy; finding the design that needs no extra layer is the
hard part, and it is the extra effort this gate asks for. When two solutions
work, the one with fewer concepts, fewer types and fewer indirections is the
exemplary one. An abstraction with one caller and no second in sight is not
scalability, it is complexity; the simplest code that fits the canonical
pattern is what scales.

## 1. Canonical before novel

- Reach for the pattern the Rust Book, std, or the crate's own docs use
  (ratatui, crossterm, reqwest, rmcp, serde, tokio, thiserror). If a
  canonical pattern exists, use it; a clever alternative needs a reason
  written next to it.
- Before writing something new, find how this codebase already does it and
  follow that. Two ways of doing one thing is a defect even when both work.
- Use std and existing workspace dependencies before adding a crate. A new
  dependency is an architectural decision, not an implementation detail.
- No macros, unsafe, or type-level tricks where a plain function, trait or
  enum does the job.

## 2. Hexagonal: the core knows no adapters

The workspace is already shaped as ports and adapters; keep it that way.

| Role | Where | Rule |
|---|---|---|
| **Domain + ports** | `aldwin-core` | Depends on no other workspace crate. Defines the traits the outside world must satisfy — `LlmClient`, `ToolDispatcher`, `RecordSink` — and the types that cross them. |
| **Adapters** | `aldwin-llm`, `aldwin-tools`, `aldwin-tui` | Implement or consume ports. Never depended on by `aldwin-core`. |
| **Shared settings and store** | `aldwin-config` | The settings schema and its YAML files, and the transcript store (`HistoryStore`) that aldwin-cli's `RecordSink` writes through. Not an adapter behind a port: a shared crate any crate but `aldwin-core` may depend on. |
| **Account login** | `aldwin-login` | A provider account's login and the session that keeps it fresh (`Login`, `Session`). A leaf: it depends on nothing in the workspace, and aldwin-llm and aldwin-cli depend on it. Nothing OAuth-shaped crosses its surface. |
| **Composition root** | `aldwin-cli` (`bootstrap.rs`) | The one place concrete adapters are chosen and wired together. |
| **Dev harness** | `aldwin-review` | The `/review` loop. Never in a release build; nothing depends on it. |

Checks:

- **Dependencies point inward.** `aldwin-core` imports nothing from the
  workspace; an adapter may depend on core, never the reverse. Verify with
  `grep aldwin- crates/core/Cargo.toml` — it must be empty of workspace crates.
- **Nothing foreign crosses a port.** Provider wire types stop at
  `LlmClient` (a non-negotiable in AGENTS.md); likewise terminal, filesystem
  and HTTP types stay in the adapter that owns them. A port speaks domain
  types only.
- **New external capability ⇒ new port.** Talking to something new (a
  service, a store, a process) means a trait in core that describes what the
  domain needs, and an adapter that implements it — not a direct call from
  domain code.
- **Wiring happens once.** Constructing concrete adapters anywhere but the
  composition root is a defect.

## 3. Encapsulation

- Private by default. `pub` is a promise to other crates; `pub(crate)` is the
  default for anything shared inside one. Every `pub` item must be needed by
  a caller that exists.
- Struct fields are private unless the type is plain data by design. Enforce
  invariants in constructors and methods, not in callers.
- Make invalid states unrepresentable: an enum over a flag-and-option pair,
  a newtype over a bare `String`/`u64` that carries meaning (as `SessionId`
  does).
- A module exposes behaviour, not its internals. If a caller needs to know
  how something works to use it, the boundary is in the wrong place.

## 4. Modularity

- One responsibility per module and per crate; a file that needs "and" to
  describe it is two files.
- Functions do one thing at one level of abstraction. Long functions are
  split along their natural steps, each named for what it achieves.
- Behaviour that varies goes behind a trait; data that varies goes in an
  enum. Do not branch on a type tag in several places.
- No cycles between modules, and no reaching into a sibling module's
  internals via `super::super::`.

## 5. Scalability

Scaling here means the *next* feature is cheap to add, not that this one
handles every case.

- Adding a new tool, provider, slash command or screen should mean adding a
  new implementation of an existing trait or a new enum variant — not editing
  a dozen match arms scattered across crates. If it would, fix the seam
  first, in its own change.
- Registries and dispatch tables over hard-coded chains of `if`.
- No global mutable state; pass what a component needs. Shared state is
  explicit (`Arc`, channels) and owned by the composition root.
- Async code never blocks the runtime; blocking work goes through
  `spawn_blocking` or stays out of async paths.

## 6. Clean code

- Names say what a thing is or does in the domain's words, not how it is
  implemented. No `data`, `info`, `manager`, `helper`, `utils`.
- Errors are typed with `thiserror`, one error enum per crate boundary, and
  every variant is one a caller could act on or report. No `unwrap`/`expect`
  outside tests except where an invariant makes failure impossible — and then
  the `expect` message states that invariant.
- No dead code, commented-out code, or `TODO` without an entry in
  `docs/spec/aldwin-open-tasks.md`.
- Comments follow `.agents/skills/comments/SKILL.md`: only what the code
  cannot say — a reason, an invariant, a warning, a hidden dependency, a
  pointer — lean, and written for an LLM reader.

## 7. Consistency

- The new code reads as if the same author wrote the whole crate: same
  naming, same error shape, same module layout, same formatting.
- Rust conventions from the rust skill apply without exception.
- A change that introduces a better pattern either migrates the existing
  uses too, or does not introduce it. Inconsistency is not paid for later.

## 8. Tests

- Behaviour is tested at the port: domain logic is exercised through its
  traits with in-memory adapters, not through real I/O.
- Each adapter has its own tests against the real thing it adapts, using
  `tempfile` for the filesystem.
- A bug fix lands with the test that would have caught it.

### Fixing a bug

1. **Fail first.** Run the new test against the old code — revert the fix,
   or copy the old function back — and see it fail for the reported reason.
   A test that passes either way pins nothing.
2. **Test the siblings.** Name the other ways the same input can arrive —
   another platform or launcher (`sandbox-exec` starts, then fails), a
   malformed value (a `.git` file without `gitdir:`), the all-empty case —
   and cover each one the fix treats differently.
3. **Keep what the old code did.** Read what the code being removed or
   simplified did besides the bug — an error it reported, a fallback, a
   note it cleared — and keep it, or say in the commit why it goes.
4. **Change the fact everywhere.** Grep the old behaviour's wording across
   `crates/`, `docs/`, `.agents/` and `AGENTS.md`: the system prompt, tool descriptions, doc
   comments, specs and ADRs that state it (records skill).

## 9. Records

The ledger, the specs, the ADRs and `baseline.json` change in the same
commit as the code they describe, and a decision against fixing something
is the developer's call. `.agents/skills/records/SKILL.md` says how.

## Running the gate

1. While designing: check the plan against sections 1, 2 and 5 before
   writing code. Structural mistakes are cheapest here.
2. Before calling the change done: walk every section against the diff
   (`git diff`) — for a fix, the four steps of §8's *Fixing a bug*, and the
   records skill's checks — then run the rust skill's required checks.
3. Before committing: run `/review`. Its stage 6 judges the diff against
   this gate, and an agent's commit is refused without a passing review.

Report the result as a short list — one line per section, **pass** or the
specific file and line that fails it. A gate with a failing line is not
passed.
