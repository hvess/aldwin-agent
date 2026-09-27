# ADR 0014 — An MCP server cannot write the workspace

**Status:** accepted, 2026-09-27 (the developer's decision).
**Supersedes:** ADR 0011 §1 for MCP servers only (they are no longer
given the workspace roots), and the one exception ADR 0009 §4 stated —
"what an MCP server writes during its call".
**Affects:** `aldwin-tools` (`mcp/bridge.rs`, `mcp/tool.rs`, `sandbox.rs`),
`aldwin-cli` (`bootstrap::run`), `docs/spec/aldwin-tools.md`,
`docs/spec/aldwin.md`, `README.md`

## Context

ADR 0009 §4 makes the review the only way a change reaches the tree:
`edit` stages, and only an approve writes. An MCP tool runs in its own
process over the real tree, so the dispatcher opens the review before it,
but what the server itself writes was never staged. ADR 0011 bounded that
to the workspace — every process Aldwin starts could write its roots — and
left it open as open-tasks 2.

A first answer recorded the workspace around each call, put back what the
server changed, and staged it. It was built and taken out before it
landed: the put-back was Aldwin's own write, outside the sandbox, and each
review pass found another way a path check in it could be fooled (a
swapped link, an unreadable file, a name that is not UTF-8). The developer
then asked for the put-back to run inside the sandbox; the cost that
remained was a copy of every file in scope before every call, since
putting a file back needs its old contents.

## Decision

**An MCP stdio server is started with no workspace root.** `McpBridge`
passes `sandbox::command` an empty root list, so where the sandbox
confines, the server can write only the incidental paths (ADR 0011 §1):
the devices, `/tmp`, `$TMPDIR`, `~/.cache` and the package stores. It
reads anything, as before, and the review still opens before its call,
since it reads the tree.

A server that tries to write the workspace gets the kernel's refusal
(Landlock, Seatbelt) and reports it as its own error. The model makes the
change with `edit` instead, which is reviewed. `McpBridge` no longer takes
a `Workspace`.

## Limits

- **Where nothing can be confined, nothing changes.** ADR 0011 §3 holds:
  the server runs unconfined and the developer is told once at startup.
- **A workspace inside an incidental path stays writable** — a project
  under `/tmp` or `~/.cache` — because those paths are writable to every
  process Aldwin starts.
- **A server that keeps its own state in the workspace fails** to write
  it. State under `~/.cache` or `/tmp` still works.

## Rejected

- **Record, put back, stage** — the first answer, above. Its put-back was
  a second write path with its own boundary checks, and a correct one
  still copies the tree before every call.
- **An overlay filesystem for each call.** Linux-only, and a mount per
  call; macOS has no equivalent that needs no privileges.

## Consequences

- **Where the sandbox confines, an MCP server is no longer a way around
  the review** (within the Limits above). A command the model runs, and
  the language server behind `explain`, can still write the workspace
  (ADR 0011 §1); that is their contract, not this ADR's.
- **An MCP tool that edits files no longer works** under a confining
  sandbox. The tools that read — search, fetch, look up — are unaffected.
- Open-tasks 2 is closed.
