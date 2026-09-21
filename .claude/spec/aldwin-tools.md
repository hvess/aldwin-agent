# aldwin-tools

ToolDispatcher impl, built-in tool set, Edit approval gate, MCP bridge via rmcp.

**Status:** active — one known gap, see Progress below
**Scope:** aldwin-tools crate only. Built-in tool implementations, registry, dispatch, Edit approval surface, MCP bridge. Excludes permission policy, agent loop, TUI, config persistence.
**Owner:** Maximilian
**Last Updated:** 2026-09-21

**Progress (2026-09-21, ADR 0007 — `run` joins the model the other tools live in):**
Prompted by a reviewed session transcript, not by a plan. The 2026-09-20 entry
below says reach "is bounded by `paths.rs`'s argument containment". That was
true of `read`, `edit` and `explain` and **false of `run`, which never called
`paths.rs`** — so the boundary held on the tools that show a diff and was
absent from the one that executes programs. In the transcript: `edit` refused
a sibling directory, and `run` wrote a 135-line file there through
`bash -c 'cat > …'` and later `rm -rf`'d two checkouts, with no prompt. 71
`run` calls; 33 were `bash -c`; the single `edit` call failed; the diff gate
fired zero times.

- **`paths::Workspace` replaces `resolve_in_project`.** A canonical, shared root list;
  `roots[0]` is the project root, the rest come from `roots:` in the *project*
  `permissions.yaml`. All four built-ins take a `Workspace`, and
  `builtin_registry` does too. Absolute paths are now accepted when contained
  (a second root is unaddressable otherwise); the double lexical-then-canonical
  check, and so the symlink-escape refusal, is unchanged.
- **`run` contains its arguments.** Checked when absolute or climbing with
  `..`; everything else is relative under an already-contained `cwd`. Globs and
  flags are deliberately left alone (`path_like` and its test say which).
  `bash -c '…'` is the stated hole — open-tasks entry 26.
- **`run` takes `cwd`.** Per call, contained like any path. This is what most
  of those 33 shell calls were standing in for.
- **A timeout keeps the output.** `drain` accumulates into a shared buffer, so
  `ToolError::Timeout` carries `partial`. A 30-minute clone used to report
  only that it was long.
- **A non-zero exit is not a refused read.** `looks_like_denial` needs
  evidence — a permission/read-only message on stderr, or death by signal.
  `grep` exiting 1 used to raise `ReadRefused` and ask about a write nobody
  attempted.
- **`SandboxUnavailable` is a question.** It carries `program`/`args` and the
  dispatcher routes it to `offer_as_write`, as ADR 0004 §4 always specified.
  It was a flat error, which on macOS failed every read-declared call; the
  model declared `read` twice and then declared 69 consecutive calls `write`.
- **The sandbox seam is `command_line` + `install`,** not `engage`. Linux
  returns the program untouched and confines in the child; macOS returns
  `sandbox-exec -p <profile> -- <program>` and installs nothing. The command
  line is asked for *before* stdio and `setsid` are configured because
  `Command` has no getter for either — a backend that rebuilt the command
  would drop the pipes and `run` would panic taking stdout.
- **An audit the same day found eleven defects in the above, all fixed.** The
  ones that change how to read this crate: containment is decided on
  *resolved* paths only (a lexical pre-check against canonical roots refused
  every symlinked prefix, i.e. `/tmp` on macOS) and also on the path as the
  filesystem resolves it (`out/../x` after a symlink); `Workspace` roots are
  shared and replaceable, so `/reload-config` re-reads them; the sandbox's
  incidental paths are legitimate `run` arguments (`sandbox::is_incidental`);
  output is accumulated as bytes; a timeout `killpg`s the group `setsid`
  created; and an unenforceable read consults the engine before it asks. The
  refusal message had named a `--root` flag that never existed.
- **`sandbox/macos.rs` is compiled everywhere and used on macOS.** No FFI, so
  no reason to hide it behind a `cfg` this project's machines never build.
  Never run on a Mac — open-tasks entry 25.

**Progress (2026-09-20, ADR 0004 — `shell` is gone and a sandbox arrived):**
The largest change this crate has had. Read
`.claude/adr/0004-permissions-are-a-declared-class-an-enforced-sandbox-and-a-lock.md`
and the rewritten `aldwin-permissions.md` before anything below; several
sections further down still describe the world this replaced and are marked
where they do.

- **`shell` is replaced by `run`** (`tools/run.rs`). It takes a **program and
  an argument list** and `execve`s them. There is no interpreter, so `&&`,
  `|`, `;` and `$(...)` are ordinary argument characters — the "a glob has no
  concept of a metacharacter" limitation `shell.rs` documented as an accepted
  risk is not mitigated, it is *gone*, because there is no command line to
  chain onto. Pipelines and redirection are lost with it; if they return, they
  return as a list of stages each with its own grant.
- **Every call carries a declared class.** `Tool::permission` replaces
  `permission_target`/`permission_target_is_path` and returns a
  `PermissionRequest { program, class, argv }`. `read`/`explain` are
  structurally `read`; `edit` is `edit` and never reaches the permission path;
  `run` takes the agent's declaration from its input; **every MCP tool is
  `write`** — see `mcp::tool`'s `permission` for why a server's own hint
  cannot be believed here.
- **`sandbox/` is new, and the model rests on it.** A `run` call declared
  `read` executes under a Landlock ruleset: the whole filesystem readable,
  nothing writable but a short incidental list, and — from ABI 4 — no TCP. A
  declaration that turns out to be wrong produces `ToolError::ReadRefused`,
  which the dispatcher turns into the developer-facing question rather than a
  model-facing error. Nothing landed when it is raised, which is what makes
  the re-run after a yes safe.
- **Read is allowed broadly inside the sandbox, on purpose.** A program must
  read its interpreter, libraries and `/etc` to run at all. Reach is bounded
  by `paths.rs`'s argument containment instead, which is why the claim is *no
  tool is pointed outside your project by us* rather than *nothing outside
  your project is touched*.
- **`.git/` is deliberately not on the incidental-write list.** Letting a read
  write anywhere under it would let `git commit` — which touches nothing else
  — succeed while claiming to be a read. `GIT_OPTIONAL_LOCKS=0` is set for
  read-declared calls instead, which is git's own switch for exactly this and
  is what makes `git status` work.
- **Two gaps, stated rather than hidden.** Landlock's network control covers
  TCP only; UDP and unix sockets are outside it. And a denial reaches us as an
  ordinary failure from the child, so the prompt says a read-declared call
  *could not complete with the project read-only* — it does not name the path
  it reached for. Naming it needs syscall interception; the guarantee comes
  from the write being impossible, not from our seeing it.

**Sections below that ADR 0004 invalidated** and that are left in place rather
than silently edited, because the reasoning in them is still worth reading:
the **shell** bullet under Built-ins (line ~160) and the V0-built-in-set
Decision (line ~179) describe the argv-glob model; `Explain`'s reference to
`shell:git diff*` is now `run` with program `git`; and "Sandboxing of shell
execution (seccomp, landlock, containers) — out of V0 per parent" in Out of
Scope is no longer true — it is in, and it is the load-bearing piece.

**Progress (2026-09-19, `explain` surfaced a retriable LSP error as a
failure):** `LspClient::request` now re-sends a request the server answered
with `ContentModified` (-32801), five attempts over ~1.5s, that code and no
other. Nothing else in the crate changes.

**The defect, in a real session:** `ContentModified` means the server threw
its answer away because its view of the content moved underneath it, and the
protocol's intent is that the client quietly asks again. rust-analyzer
returns it throughout startup and reindexing. Nothing in the crate recognised
it — `client.rs` modelled every server error as a flat `Rpc { code, message }`
and handed it up — so asking a code question in the first seconds of a
session produced a tool error that fixed itself if you asked twice. That is
the shape developers report as "it's flaky" and never pin down.

**Why it went unseen, which is the more useful half.** Two independent
covers. `real_rust_analyzer_resolves_a_definition` is `#[ignore]`d by default
for good reasons of its own (it waits on indexing; slow and load-sensitive),
so it ran only on request. And `rust-analyzer` was not installed on the
development machine at all, so the *other* LSP test —
`spawns_and_initializes_a_real_language_server` — failed on a missing binary,
which made `cargo test --workspace` permanently red and trained everyone
reading it to skim the failure. A red suite is where a real regression hides:
the missing-binary line and a genuine breakage render identically.

Installing rust-analyzer turned the first test green *and* let the second run
for the first time, which is what exposed this. The tempting fix at that
point — `#[ignore]`-ing the failing test to get a green suite — would have
buried a true signal twice over.

**Scope of the retry, deliberately narrow.** `is_retriable` matches
`ContentModified` alone and is its own function so that widening it is an
edit to a documented rule. Every other `Rpc` code reports something about the
request itself, where re-sending identical bytes can only reproduce the
answer while hiding it behind a delay; `Closed` and `Io` will not heal on the
same connection. Each attempt takes a fresh request id — the abandoned one
has been answered and will never be answered again.

**What is still untested:** the retry *loop*. The policy is pinned by unit
tests and the end-to-end path by `real_rust_analyzer_resolves_a_definition`
(1.7s, stable over three runs, previously failing at 0.6s), but there is no
fake server in this crate, so nothing exercises the loop deterministically
against a scripted `ContentModified`. Building one means a test-only LSP
server binary; it is real coverage the crate does not have.

**Progress (2026-08-29):** All four V0 built-ins (Read, Edit, shell,
Explain) and the MCP bridge are implemented and tested — `053792a`,
`bcf6209`, `6eb4f6d`, audit-fixed in `bb8acff`. Explain's LSP support is
V0-scoped to Rust only (rust-analyzer), matching the crate's own
LSP-scope-creep Pitfall rather than a gap. Not yet built: the MCP
first-invocation edit-shape follow-up ("MCP Edit-Shape Detection" — see
aldwin-permissions.md's matching gap, which this depends on). Every MCP
tool currently registers with `edit_class: false` and never graduates to
Edit's binary approval gate. Keep this spec active until that's built.

**Progress (2026-08-30, concurrency/path-safety audit-fix):** A rust-skills
audit (unsafe-checker, m06-error-handling, m07-concurrency, m12-lifecycle,
m15-anti-pattern) plus a follow-up 3-pass verification found and fixed
three real defects, all confirmed reachable, not theoretical:
(1) `LspClient::ensure_open` (`lsp/client.rs`) checked-then-inserted into
its `opened` set across an `.await` using a `std::sync::Mutex`, so two tool
calls dispatched concurrently on the same file (e.g. `definition` +
`hover` in one step) could both see "not yet opened" and both send
`didOpen` — an LSP protocol violation; fixed by switching `opened` to a
`tokio::sync::Mutex` held across the notify. (2) `paths::resolve_in_project`
only checked containment lexically, so a symlink planted inside
`project_root` pointing outside it (legal in a git repo) passed the check
while the real I/O followed it out — fixed by re-checking containment
against the canonicalized, symlink-resolved form (see
`canonicalize_existing_prefix`), with new regression tests for both an
existing and a not-yet-existing (Edit-new-file) escape target. (3)
`EditTool::call` (`tools/edit.rs`) computed its diff from content read
before the approval wait, then wrote that stale content unconditionally
after approval — a real TOCTOU, since dispatch runs multiple tool calls
concurrently within a step; fixed by re-reading and rejecting
(`ToolError::ConcurrentModification`) if the file changed during the wait.
Also added the missing `// SAFETY:` comment on `shell.rs`'s `pre_exec`
unsafe block (the call itself was already sound). A fourth suspected
finding — MCP servers cold-starting concurrently and racing `bridge.rs`'s
`running` guard — was investigated and refuted: `register_mcp_tools`
already warms every configured server sequentially at startup, so the race
needs two servers to both fail at boot and be retried concurrently later;
not a practical defect.

**Progress (2026-09-02, path-like permission targets):** Part of the
directory-scope prompt option (see aldwin-permissions.md's matching
Progress note). `Tool` gained a defaulted `permission_target_is_path`
method (default `false`, the conservative "no directory offer" choice) —
`ReadTool` always returns `true`, `ExplainTool` returns `true` only for
its position-based ops (a `path` arg present; `workspace_symbols`' search
string is not a path). `shell`/MCP tools take the default. `Dispatcher::
check` reads it alongside `permission_target` and threads it into
`Engine::check_tool`'s new `path_like` param; `prompt_and_record` now
persists whatever `pattern` came back on the developer's `PromptResponse::
Tool` (the TUI's own choice — exact target or a broadened directory glob)
instead of always re-using the original `target`. No change to Edit's
approval gate (`gate.rs`) or its own future-driven path — `edit_class`
tools never call `permission_target`/`permission_target_is_path` at all,
so this is invisible to Edit by construction, matching aldwin's
non-negotiable Edit-is-never-allowlistable constraint.

**Progress (2026-09-03, a grant made mid-step now applies to the rest of
the step):** Developer report — "directory permissions don't appear to
count properly when commands are queued (approving a directory in the first
request doesn't automatically approve the next request in the same
directory)." The path-like directory scope above worked exactly as designed
and still failed here, for a reason upstream of it: aldwin-core dispatches
a step's tool calls concurrently (`dispatch_tools`' `future::join_all`), so
every call in the step reached `Engine::check_tool` before the developer had
answered anything, and each independently got `PromptRequired` back. The
answer to the first prompt — even a `<dir>/**` grant plainly covering the
rest — could not affect calls whose outcome was already decided, so the
developer was asked again for every queued call in the directory they had
just approved.

Fixed in `Dispatcher` with a `prompt_gate` (`tokio::sync::Mutex`) and a
second check: `check` still runs the fast, uncontended check first, and only
a call that comes back `PromptRequired` takes the gate — then re-checks
under it, because by the time it acquires the gate the grant made in answer
to an earlier prompt has been recorded. A call now covered proceeds
silently; only a genuinely still-uncovered one prompts. This also makes
one-prompt-at-a-time real rather than incidental (the TUI only ever makes
the front of its queue interactive anyway). Pinned by
`a_directory_grant_answered_for_one_queued_call_covers_the_others`, which is
bounded by a timeout on purpose: the pre-fix failure is a *hang* (a second
prompt raised that nothing answers), not a wrong value, and was confirmed to
fail that way with the fix reverted before being accepted as a regression
test. Edit's approval gate is untouched — `edit_class` calls never enter
`check` at all.

## Why

Owns every concrete tool Aldwin can dispatch — the V0 built-ins (Read, Diff, Explain, Edit, shell) and the MCP bridge that maps remote tools onto the same dispatch surface. Implements core's ToolDispatcher trait. Hosts the Edit approval gate as structural friction the developer cannot configure away. Other crates supply policy and protocol; this crate supplies behaviour.

## Vocabulary

- **Tool:** A registered (name, input schema, edit_class, dispatch fn) tuple. Built-in tools register at startup; MCP tools register lazily on first server enumeration.
- **Registry:** In-process map of name → tool. Single source for both core's ToolDispatcher impl and TUI listing.
- **Edit Class:** Boolean flag set at registration for built-ins, or at first-invocation prompt for MCP tools. When set, the permission engine refuses to attach anything other than the per-call binary approval gate to invocations. For MCP, the marking includes the path-arg and content-arg mapping needed to render the diff at call time.
- **Dispatch:** Resolve name → tool, check permissions, run the future. Concurrent across calls within a step (the core awaits join). Each tool owns its own cancellation behaviour.
- **Approval Gate:** The binary prompt round-trip Edit waits on inside its own future. Distinct from the four-tier permission prompt — approval is per-call, never persisted, never allowlistable.
- **MCP Bridge:** Subprocess host for MCP servers via rmcp. Each remote tool surfaces as a registry entry with a dispatch fn that proxies to the server; edit_class is decided at first invocation via the permission engine's edit-shape follow-up, not at registration.

## Design

- **Registration:** Built-ins register at crate init with a static descriptor (name, JSON schema, edit_class, async fn). MCP tools register after the bridge enumerates a server. Duplicate names are rejected; MCP-supplied names that collide with built-ins are namespaced `<server>:<name>`.
- **Dispatch Flow:** Core calls dispatcher with (name, assembled input). The dispatcher resolves the tool, calls permissions.check, and either runs the future, returns a structured Denied, or — on PromptRequired — emits PromptRequested via the core event stream, awaits PromptResponse on the core command channel, calls back to record the decision, and then continues. Errors are structured, not panics — the model sees them and adapts.
- **Builtin Tools:**
  - **Read** — Read a file from disk. Path-globbed via permissions.
  - **Explain** — LSP-backed code intelligence. Single tool with an `op` enum (definition, references, hover, implementations, workspace_symbols). Output is structured location and signature data only — no prose summaries. LSP servers spawn lazily per-project per-language, persist for session, shut down at process exit. Diffs between refs/paths are handled via `shell:git diff*`; `shell:diff*` is the fallback when the working directory is not a git repository.
  - **Edit** — Propose a single edit (path, before, after). Always per-call approval; the gate lives inside the tool's future. Approval emits ToolApprovalGranted; denial returns structured Denied to the model.
  - **shell** — Run a command. Argv pattern is permission-keyed (e.g. `shell:cargo test*`). Piped output (not PTY), project-root cwd (no per-call override), inherited env, 120s timeout (overridable per call), output capped at 50KB with a truncation marker.
- **Edit Approval:** Edit's future emits ToolApprovalRequested via the core, then awaits the matching ApproveTool/DenyTool command. Approval state is per-invocation; no persistence, no allowlisting. The diff is rendered from the (before, after) the tool already assembled — TUI owns formatting.
- **MCP Lifecycle:** Servers are spawned from config entries on first use of any of their tools. The bridge enumerates tools and routes invocations through rmcp. Spawn failures and protocol errors surface as structured tool errors, not crashes. Servers shut down on process exit; per-session reconnect is out of V0.
- **MCP Edit-Shape Detection:** At first invocation of an MCP tool, the permission engine's four-tier prompt grows a one-time follow-up: "does this tool modify a file? If yes, which arg is the path, which is the new content?" If the developer marks it edit-shaped and supplies the arg mapping, subsequent calls route through Edit's per-call binary approval gate — the bridge reads the file at call time to render the diff against the supplied content. Tools whose shape cannot be pre-diffed (apply_patch, edit_at_line, bulk_replace and the like) fall back to the standard prompt with no diff gate; the developer's choice there is accept-as-is or deny. Marking persists with the permission grant at the chosen scope.
- **Cancellation:** Pure-Rust tools cancel at await points. Shell sends SIGKILL to the process group. MCP invocations drop the response future and best-effort signal the server. Cancellation is advisory from core's perspective — the dispatcher promises to release the slot, not that the OS-level work stopped.

## Interfaces

- **Tool Dispatcher Impl:** Implements core's ToolDispatcher trait. Single entry point: dispatch(name, input, cancellation token) → future of Result<ToolOutput, ToolError>.
- **Registry View:** Read-only listing of registered tools (name, source: builtin|mcp, edit_class, schema). TUI status bar consumes this to display the names of tools currently running within a step.
- **Events:**
  - ToolApprovalRequested — Edit only; carries assembled diff payload
  - McpServerStateChanged — spawn / ready / errored / exited
- **Commands:**
  - ApproveTool — response to ToolApprovalRequested
  - DenyTool — response to ToolApprovalRequested

## Decisions

- **V0 built-in set is exactly Read, Explain, Edit, shell. Nothing else.** — Parent decision. Additional capabilities arrive via MCP, not built-ins. The minimal set is the contract; expanding it would normalise built-in growth as the escape valve. Diff is not a tool — `shell:git diff*` covers the common case, `shell:diff*` is the non-repo fallback, and the internal diff-rendering primitive Edit uses is an implementation detail of the approval surface.

- **Explain is LSP-backed code intelligence; output is structured, factual, concise.** — LSP gives the model navigation affordances (definition, references, hover, implementations, workspace symbols) without paying the token cost of reading every potential caller. Output is structured location and signature data only — no prose. Prose summaries would duplicate what Read + model reasoning already covers.

- **Edit approval gate lives inside the tool's future, not in the dispatcher.** — Keeps the dispatcher uniform — every tool is a future of a Result. Edit's friction is structural to the tool, not a side path. Matches core's "approval-gated tools handle their gate inside the future" decision.

- **edit_class is registration-time and immutable; never derived from tool name.** — Inherited from aldwin-permissions. Naming-based enforcement is escapable by renaming; flag on the descriptor is not.

- **MCP tools become edit-shaped via a first-invocation follow-up, never via upfront config.** — Edit friction must extend to MCP tools that modify files, but the MCP protocol does not tell Aldwin which tools those are. Marking at first invocation puts the question at the moment the developer is already paying attention to the call — matches the parent decision against first-run wizards ("understanding develops by encounter"). Tools whose shape cannot be pre-diffed (arbitrary patch / partial edit / mutation by query) cannot inherit the Edit gate and fall back to the standard prompt; that limitation is structural to the diff-rendering contract, not a setting.

- **MCP name collisions namespace under `<server>:<name>`; built-ins win unprefixed.** — Built-ins are the stable surface; remote tools must not silently shadow them. Prefixing is explicit and survives server churn.

- **Tool errors are structured and fed back to the model; transport errors do not retry here.** — Tool-level failure is signal for the model. Transient retry policy belongs to aldwin-llm for upstream calls, not to the tool layer.

- **Each tool owns its cancellation; the dispatcher only promises to release the slot.** — Shell needs SIGKILL on the process group; pure-Rust tools want await-point abort; MCP wants the response future dropped. A single cancellation primitive at the dispatcher would have to lie about at least one of these.

## Pitfalls

- LSP integration may outgrow this crate — server lifecycle, JSON-RPC client, capability negotiation, and per-language config (rust-analyzer, sourcekit-lsp, kotlin-lsp) are real scope. Split into aldwin-lsp if it eats more than ~25% of this crate's surface.
- Edit-shape marking for MCP tools drifting back into upfront config (e.g. a UI flow that asks at server registration rather than at first call) — defeats the encounter-driven design and re-creates the wizard the parent spec rejected.
- MCP edit-shape arg mapping going stale if a server changes its tool schema between sessions — detect schema-hash mismatch on the marked tool and re-prompt, do not silently reuse the old mapping.
- Approval state for Edit accidentally caching across calls "for ergonomics" — the gate is per-invocation by construction; any cache is a bypass.
- Concurrent tool calls in a step racing on shared resources (same file edited twice, same process group signalled twice) — dispatcher is concurrent; tools must be reentrant or self-serialise.
- rmcp version drift silently changing the wire shape under us — pin a known-good version and surface protocol mismatches as structured tool errors, not panics.
- A built-in growing a fifth member under the banner of "small obvious addition" — every addition is permanent surface. Channel it through MCP first.

## Out of Scope

- Permission policy, scope precedence, allowlist storage — aldwin-permissions.
- Agent loop, append-only log, turn/step semantics — aldwin-core.
- TUI rendering of diffs, tool listings, approval dialogs — aldwin-tui.
- Config file format and persistence of MCP server entries — aldwin-config.
- Anthropic wire format, SSE, retry — aldwin-llm.
- Session persistence of approval history — out of V0 per parent.
- Sandboxing of shell execution (seccomp, landlock, containers) — out of V0 per parent.
- Streaming tool outputs (partial deltas during execution) — V0 returns terminal results only.
- Web-search, fetch, or any network tool as a built-in — channel via MCP.

## References

- .claude/spec/aldwin.md — parent; built-in surface, MCP-as-extension, Edit-as-structural-friction.
- .claude/spec/aldwin-core.md — ToolDispatcher trait, ToolApprovalRequested / ApproveTool placeholders.
- .claude/spec/aldwin-permissions.md — check() interface, edit_class enforcement, MCP gating.
- https://github.com/modelcontextprotocol/rust-sdk — rmcp.
- https://modelcontextprotocol.io/specification — MCP protocol surface.
