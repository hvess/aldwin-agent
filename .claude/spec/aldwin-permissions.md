# aldwin-permissions

Default-deny permission engine — a grant is a program and a class, a deny is
a lock, and a read declaration is enforced rather than believed.

**Status:** active
**Scope:** aldwin-permissions crate. Policy engine, entry shape, prompt
round-trip. Excludes TUI rendering, YAML I/O (config), tool implementations
and the sandbox (all aldwin-tools).
**Owner:** Maximilian
**Last Updated:** 2026-09-20

**Progress (2026-09-20, ADR 0004 — the model was reopened from first
principles):** Everything below is new. The previous model — `kind:pattern`
grants, a four-tier prompt, a `shell` tool taking one opaque command string —
is gone, along with ADR 0001 which superseded it and the parts of ADR 0003
that described the option list. Read
`.claude/adr/0004-permissions-are-a-declared-class-an-enforced-sandbox-and-a-lock.md`
first; this spec is its implementation.

The short version of what moved:

- **`shell` is gone.** `run` takes a program and an argument list and
  `execve`s it. There is no interpreter, so no grant can be walked past with
  `&&`.
- **A grant is `program: class`.** The class belongs to the *call* — `git
  status` is a read, `git push` is a write — which is what makes a read/write
  axis sound where ADR 0001 found it unsound.
- **The agent declares the class; the sandbox enforces it.** A read-declared
  call runs with the tree read-only and no TCP. A wrong declaration costs a
  prompt, not a tree.
- **A deny is a lock.** Nothing narrower overrides it, and a locked call draws
  no prompt at all — there is no answer that would change it.
- **Each scope carries a standing rung** (`ask` → `read` → `write`), and the
  narrower file wins outright.
- **`edit` left the model entirely.** Not a grant, not a rung, not a row.

Every persisted grant from the old model is meaningless, and a v1
`permissions.yaml` is moved aside to `permissions.yaml.v1` rather than
reinterpreted — see aldwin-config's `retire_v1_permissions`.

## Why

Every tool call and context-file ingestion runs through this engine. It owns
precedence, the standing rung, and the in-memory session layer. Cross-cutting:
any crate that gates an action calls in rather than reimplementing policy.

One thing it deliberately does **not** own, and the division is the design:
**it never judges what a command does.** It is handed a declared class and
weighs it against the rules. Verifying the declaration is the sandbox's job at
execution time (aldwin-tools). A policy engine that also guessed at a
command's nature would be making the guess the whole model exists to avoid,
and a wrong guess there *runs the command*.

## Vocabulary

- **Program:** the grant key. For `run`, the binary (`git`). For a built-in,
  the tool's own name (`read`, `explain`). For an MCP tool, its namespaced
  name.
- **Class:** what a call does — `read`, `write`, or `edit`. A property of the
  *call*, not the program. `write` covers `read`; nothing covers `edit`.
- **Entry:** one line of an allow or deny list — a program, optionally
  qualified by a class. `git: read`, or bare `curl` for every class.
- **Rung:** a scope's standing answer for any call no entry covers. `ask` →
  `read` → `write`, widening.
- **Scope:** turn, session, project, global. Turn and session never touch
  disk; project and global are `permissions.yaml` files.
- **Lock:** a deny. It cannot be overridden by anything narrower.
- **Declaration:** the class the agent states for a call. An input, never a
  finding.

## Model

- **Default deny.** Every call starts denied. No "obviously safe" carve-out.
- **Resolution order**, and it is the order because of what each step means:
  1. `edit` never resolves here — it always asks, and `record` refuses it at
     every row including the ones that persist nothing.
  2. **Deny, across every scope.** Checked before anything that could allow,
     including a narrower scope. That precedence *is* what distinguishes a
     lock from a pre-answer.
  3. **Allow, across every scope.** Any allow covering the call suffices;
     allows do not compete.
  4. **The standing rung**, narrower file winning outright.
  5. Otherwise, ask.
- **Entries outrank the rung, both ways.** A denied program stays denied under
  `write`; an allowed one runs under `ask`. That falls out of 2 and 3 running
  before 4.
- **Deny is asymmetric with allow on class.** An allow of `read` does not
  cover a write. A deny of `read` *does* cover a write — permitting writing
  while forbidding reading describes no coherent posture.
- **A rung of `None` is not `ask`.** A file that states no rung falls through
  to the wider one; a file that states `ask` overrides it. They behave
  identically at a check and differ in the panel, which shows "not set"
  rather than asserting a rung nobody chose.
- **Eight rows.** Four allow tiers and four deny tiers, mirrored. Rows 2–4 and
  6–7 are class-qualified; row 8 (`never allow <program>`) is the whole
  program, everywhere, and is the lock.
- **Context files** keep the two-tier prompt (project / session), path-keyed,
  no content hash. Untouched by ADR 0004.

## Interfaces

- **`check(program, class, argv) -> Outcome`**: `Allow`, `Locked { scope,
  rule }`, or `Ask(PromptPayload)`. `Locked` and `Ask` are different answers:
  a locked call draws no prompt.
- **`record(program, class, choice)`**: writes the row's rule at the row's
  scope. Refuses `edit`.
- **`effective_rung()`**, **`set_rung(scope, rung)`**: the standing answer.
  Not reachable from a prompt — a per-call moment is the wrong place to change
  the standing rule for everything.
- **`effective_view()`**: snapshot with per-entry scope attribution, for the
  panel.
- **Events**: `PromptRequested`, `PermissionsChanged` — through core's stream.
- **Commands**: `PromptResponse` — through core's command channel.

## Decisions

- **Default-deny is the floor; no carve-out.** A "read is always safe"
  exception invites "`ls` is always safe" next. Uniformity of friction is
  structural.

- **A grant is a program and a class, not a command string or a glob.** The
  old unit could not carry a read/write distinction and could be walked past
  by a metacharacter. Program-plus-class is what a developer can actually hold
  in their head, and the sandbox is what bounds it.

- **The declaration is never trusted, and never needs to be.** Enforced, not
  believed: no veto list, no table of read-safe invocations, no trial run. All
  three were considered; each puts our judgement in the path of a decision
  that runs a command.

- **Deny is a lock, not a pre-answer.** A denylist anything can shrug off is
  not a guarantee. The cost is real — undoing one means editing a file, mid-
  task — and accepted, because a denylist that a session can override
  promises more than it delivers.

- **The narrower file wins outright.** Most-restrictive-wins was rejected: it
  makes a single project impossible to open up without loosening every
  project, which inverts how anyone works.

- **`edit` is outside the model.** Not a rung, not a grant, not a row. This is
  what makes the rest safe to coarsen — the one tool whose purpose is
  modifying the tree cannot be granted at all.

- **Every MCP tool is a write, whatever the server says.** An MCP call runs
  inside the server's process, where the sandbox cannot hold a declaration to
  its word. With no enforcement, believing a hint is the trust-the-declaration
  design ADR 0004 rejected, minus the thing that made it safe. Letting the
  developer classify one — with the server's claim shown as a claim — is
  ADR 0004 §4's intent and is not built.

- **No first-run wizard.** First launch writes an annotated, fully-denied
  file. First run asks one access question and writes it as the project's
  rung — the same setting a developer can change later, not a preset that
  expands into grants and vanishes.

## Pitfalls

- A "read is always safe" carve-out arriving as ergonomics.
- The prompt growing a ninth row. Eight is already at the edge of what a
  developer reads under time pressure; the deny half earns its place only
  because a lock must be reachable.
- **Treating the declaration as a finding.** Any code path that lets a
  declared class decide something the sandbox does not then enforce has
  reintroduced self-granting. MCP is the live example and is why it is
  hard-coded to `write`.
- A deny becoming overridable by a narrower scope "for convenience".
- The rung being settable from a prompt.
- An `edit` entry becoming expressible. `Class` deserializes only `read` and
  `write` precisely so a hand-written `edit:` in YAML is a load error rather
  than a rule that silently does nothing.
- The incidental-write allowlist growing. It is the one place our judgement
  re-enters; `.git/` was kept out of it on purpose (see aldwin-tools).
- Session grants leaking to disk via a confused "remember this" path.

## Out of Scope

- On-disk schema and file layout — aldwin-config.
- Prompt rendering and the permissions panel — aldwin-tui.
- The sandbox, `run`, argument containment — aldwin-tools.
- Developer classification of MCP tools — ADR 0004 §4, not built.
- Naming *which* path a refused read reached for — needs syscall
  interception; the guarantee does not depend on it.
- Audit log of grant changes — out of V0.

## References

- `.claude/adr/0004-permissions-are-a-declared-class-an-enforced-sandbox-and-a-lock.md` — the decision this implements.
- `.claude/adr/0003-the-permission-option-row-is-a-sentence.md` — §1 still governs each row's shape.
- `.claude/spec/aldwin-tools.md` — `run`, the sandbox, argument containment.
- `.claude/spec/aldwin.md` — parent; default-deny and friction-as-feature.
