# aldwin-history

Persisted conversation transcripts: a session survives process exit, and a
past session can be listed and resumed from inside a running one.

**Status:** complete — every step done, 2026-09-20. See Progress.
**Scope:** Session identity, the on-disk transcript, the write path, `/resume`
and its picker, and what a resumed session restores. Excludes the LLM wire
format, the permission engine, retention, and any launch-time entry point.
**Owner:** Maximilian
**Last Updated:** 2026-09-20

## The stale line

`.claude/spec/aldwin.md` line 83 says, as a V0 Decision: *"No session
persistence, no first-class history... Sessions are ephemeral."* It becomes
false the day this ships, so it is edited in the same change (Step 9), and ADR
0005 records why. This is bookkeeping that travels with the code, not a gate in
front of it.

Only the first clause goes. The rest of that Decision — memory is
developer-authored, Aldwin proposes no entries, nothing crosses into a *new*
session — is what this feature is careful not to touch.

## Why

Every session currently dies with the process. A crash, an accidental
`Ctrl+C`, or closing the terminal loses the reasoning as well as the output —
and in a harness whose premise is that the *discussion* is the artefact, that
is the one thing least affordable to lose. Resume also removes the tax on
quitting: if picking a session back up is cheap, stopping to think is cheap.

## Vocabulary

- **Session:** One conversation, identified by a `SessionId` minted at
  bootstrap. `/clear` ends one and begins another.
- **Transcript:** The on-disk JSONL file for a session — a header line plus
  one line per `LogRecord`.
- **Resume:** Loading a transcript into the running session, replacing both
  `ConversationLog` and the TUI's rendered log.
- **Sealed:** A session no longer being written to. Sealing is implicit — the
  process exited. There is no close record to lose.

## Model

- **The record already exists.** `LogRecord` (`crates/core/src/event.rs:122`)
  is already `Serialize`/`Deserialize` and is already the committed,
  streaming-stripped form of the conversation. History persists a type that is
  otherwise complete; it introduces no second representation.

- **Append-only JSONL, one file per session,** at
  `~/.aldwin/history/<project-slug>/<session-id>.jsonl`, mode `0600`. First
  line is a header (`version`, `started_at`, `cwd`, `model`); every line after
  it is one `LogRecord`. JSONL because writes are appends and a torn tail costs
  one turn, not the session.

  The header carries no title. It is derived at listing time instead — at the
  moment a file is opened no user message exists yet, so a header title would
  mean going back to rewrite line one mid-session, which is the one thing an
  append-only file must never do.

- **Every record is written, and the *load* decides what is usable.** Filtering
  on write would be more code than not filtering, and `messages_from_log`
  (`crates/core/src/agent.rs:446`) already knows what to do with the full
  record stream. See the truncation Decision for what load drops.

- **The title is derived, not authored.** First user message, trimmed to one
  line. No LLM call to summarise a session: that spends the developer's tokens
  on filing.

- **Writes are best-effort.** A failed history write emits `Event::Notice` and
  the turn proceeds. History must never be able to fail a turn.

- **Core stays filesystem-free.** `aldwin-core` depends on no fs crate and
  must not start. The writer reaches it as a `RecordSink` trait implemented in
  aldwin-config; resume reaches it as a command *carrying records*, not a
  path. Core never learns where a transcript lives.

- **The TUI stays filesystem-free too.** `aldwin-tui` depends only on core and
  permissions. The session list is handed in at startup as display rows, the
  same way the model catalogue already arrives (`SessionProvider`).

- **Nothing re-executes on resume.** Tool calls and results replay as text.
  Permission grants are session-scoped and do not come back; a resumed session
  re-asks. Default-deny is not weakened by resume — this is the constraint most
  at risk here and it is not negotiable.

## Interfaces

- **`SessionId`** — new id type beside `TurnId`/`StepId` in
  `crates/core/src/types.rs`. Three fields, each answering a different
  collision: epoch seconds (ordering), pid (two Aldwins in one project), and a
  process-local counter (two sessions in one process — see Progress).
- **`RecordSink`** — `trait RecordSink: Send + Sync { fn append(&self, r:
  &LogRecord); }` in aldwin-core. `ConversationLog::with_sink` installs one;
  `append()` fans out to it. `snapshot()`, `len()`, `clear()` unchanged.
- **`HistoryStore`** — aldwin-config (`history.rs`; it owns `fsio` and the
  `~/.aldwin/` layout). `create` / `reopen` / `append`, with free functions
  `list`, `load` and `project_dir` beside it, and `Config::history_dir` for the
  path. It does *not* implement `RecordSink`: a failed write has to reach the
  developer through the session's event channel, and aldwin-config holds no
  tokio dependency. That impl is `aldwin-cli`'s `History`, which also owns the
  swap `/clear` and `/resume` perform — the shape `ClientHandle` already uses
  for `/model`.
- **`Command::Resume { records }`** and **`Event::HistoryLoaded { records }`** —
  the exact shape of the existing `ClearHistory` / `HistoryCleared` pair. The
  interceptor reads the file, core swaps its log, the TUI redraws from the
  event. One read, two consumers, and core touches no disk.
- **`/resume`** — aldwin-cli's dispatch table (`crates/cli/src/slash.rs`).
  Bare opens the picker; `/resume <id>` does the work. The session being
  written is excluded from every listing and refused by id
  (`History::resumable`, `History::is_current`).

## Decisions

- **There is no launch-time entry point in V1 — no `--resume`, no
  `--continue`.** `archive/aldwin-cli.md:209` decides "Zero-arg
  binary in V0 — no flags, no subcommands", and `crates/cli/src/main.rs`
  enforces it with a `Cli` struct whose only job is rejecting arguments. A
  flag would reverse a second stated Decision for a convenience `/resume`
  already provides one keystroke later. If the flags are wanted, they are
  their own ADR.

- **Resume continues the resumed session's file; it does not fork.** The
  writer's path swaps to the loaded session and appends there. One conversation
  is one file, which is the model the developer already has — forking would
  leave three files for a conversation resumed twice.

- **`/clear` seals and opens a new session; it does not delete.** "Forget
  everything" must stay true for the model's context without destroying the
  record — the developer clears to manage tokens, not to shred evidence. A new
  `SessionId` and a new file; `HistoryCleared`'s in-memory semantics are
  untouched.

- **Load stops at the last complete turn.** Truncate the loaded records after
  the final `TurnEnded`. A tool call and its result are a pair: a process
  killed between them leaves a `ToolUse` on disk with no `ToolResult`, and a
  request carrying that unmatched block is rejected outright. Dropping back to
  the last finished turn is a filter over the loaded vector, and it covers the
  torn-final-line case for free.

- **A resumed transcript renders the conversation, not the session.** User
  text, assistant text, and tool activity come back. Approval cards and
  permission prompts do not — they exist only as `LogEntry`
  (`crates/tui/src/log.rs:9`), never as `LogRecord`, and persisting them means
  widening the record type for a UI concern. This is a chosen loss, not a
  defect: the developer sees the conversation they had, and the decisions they
  made are re-asked rather than re-displayed. Revisit only if reading a
  resumed session actually feels wrong.

- **The picker asks; the interceptor decides.** Bare `/resume` opens a
  one-stage list and committing submits `/resume <id>` as if typed — the rule
  `picker.rs` already documents for `/model`. One implementation of what the
  answer *does*, in the CLI's dispatch table.

- **History is per project, keyed by project root.** Sessions are about a
  codebase. A global timeline across unrelated projects is a search problem
  this spec does not want.

- **No cross-session memory.** This spec persists transcripts. It does not
  propose entries, extract facts, or carry anything into a *new* session. The
  parent's "developer-authored memory" Decision stands untouched.

## Steps

All done, 2026-09-20.

1. Add `SessionId` to `crates/core/src/types.rs`; add `RecordSink` and
   `ConversationLog::with_sink` beside it.
   - Verify: a log with no sink behaves exactly as today.

2. `HistoryStore` in aldwin-config: header + JSONL append, `0600`, implementing
   `RecordSink`. Mint the `SessionId` in bootstrap and wire the sink through
   `Agent::new`.
   - Verify: a killed process leaves a file whose earlier lines all parse.
   - Verify: an unwritable history directory emits a Notice and does not fail
     a turn.

3. `list` and `load`, with the trailing-partial-line tolerance and the
   last-complete-turn truncation.
   - Verify: a transcript ending in a half-written line, and one ending in a
     `ToolUse` with no result, both load to the last `TurnEnded`.

4. `Command::Resume` / `Event::HistoryLoaded`, mirroring `ClearHistory` /
   `HistoryCleared` — including being discarded with a warning mid-turn, which
   is what the existing pair already does.

5. Fold `HistoryLoaded` in `App::apply_event`: map each record to its
   `LogEntry`. `ToolUse` + `ToolResult` become one completed `ToolActivity`
   entry per step; deltas do not replay, so `AssistantMessage` becomes one
   `AssistantText`.
   - Verify: a resumed log renders identically to the live one for a session
     containing a user message, an assistant message, and one tool call.

6. `/clear` seals and opens a new session file.

7. `/resume <id>` in the interceptor: load, send `Command::Resume`.

8. The `/resume` picker — one list, rows of `date · title · turns`. Reuse
   `ui::decision`'s panel shape as `ui/picker.rs` already does rather than
   generalising `ModelPicker`; the session list arrives at startup beside the
   catalogue. No strokes: bands on the ground ladder, the fixed glyph set,
   lowercase labels. An empty list answers with a Notice rather than an empty
   panel.
   - Verify: measured against the design system's list frame, not eyeballed —
     see `.claude/design/IMPORT.md` rule 4.

9. Write ADR 0005; amend `aldwin.md` line 83 to point at it. Note completion
   in Progress and move this spec to `.claude/spec/archive/`.

## Pitfalls

- **History as a secrets surface.** Tool results carry file contents and
  command output — a key read out of a `.env`, a token echoed by a command.
  Ephemeral sessions made this a memory-lifetime problem; a transcript makes it
  a disk-lifetime one. `0600` and project-scoped directories are the V1 floor,
  and the absence of an opt-out is a known gap, not an oversight.
- Resume quietly restoring permission grants along with the transcript, on the
  reasoning that "the developer already approved this" — it re-creates a
  persistent allowlist through the back door and breaks ADR 0004's grant model.
- `snapshot()`'s O(n) clone (`log.rs:23`) is fine once per turn but is *not*
  fine as the write path. Append one record; never re-serialise the log.
- Writing a close/seal record and treating its absence as corruption — a killed
  process never writes one. Sealing must be implicit.
- The header's `title` drifting toward an LLM-generated summary because the
  first user message reads badly in a list. Fix the list, not the source.
- The context cost of a resumed session's first turn being its largest. V1
  accepts it: there is no truncation beyond the last-complete-turn rule, and a
  session too big to resume is a real answer, not a bug to paper over.
- Two processes resuming the same session and appending to one file. The
  fork-free Decision above accepts this; it is bounded by `O_APPEND` line
  atomicity and by nobody having asked for concurrent sessions. It stops being
  acceptable the moment a launch-time `--resume` exists.

## Out of Scope

- **Retention.** Nothing prunes and there is no `/history forget`. Clearing out
  `~/.aldwin/history/` is the developer's business, the same as any other
  directory of their own files.
- Launch-time flags (see Decisions), and with them any `--continue`.
- Cross-session memory, fact extraction, or end-of-session prompts to remember
  anything — parent spec, and deliberately.
- Search over transcripts. Listing and resuming only.
- Export, sharing off-machine, and syncing between machines.
- Editing or redacting a past transcript.
- Rendering changes in aldwin-tui — resume reuses the existing entry shapes,
  and if it needs new ones that is a signal Step 5's mapping is wrong.

## Progress

- **2026-09-20 — built, all nine steps.** 632 workspace tests pass and clippy
  is clean. Four things are worth reading before changing any of it.

  **`SessionId` needed a third field.** It was epoch seconds plus pid, on the
  reasoning that ids only have to be unique *across* processes. `/clear` seals
  and opens a new session inside one process, so two clears in the same second
  minted the same id — and because transcripts are opened for appending, the
  second conversation was silently appended to the first, in a file `load` then
  resumed as one history. A process-local counter closes it, and
  `HistoryStore::create` now uses `create_new` so a collision is loud rather
  than a merge. Caught by the `/clear` test, not by review.

  **A session listed itself.** `list` reads a directory, and the directory
  contains the transcript the running session is writing — so `/resume` offered
  the session the developer was sitting in, and resuming it would have replaced
  the log with a copy of itself. `History::resumable` and `History::is_current`
  exclude it at both entry points. The bootstrap's first cut avoided this by
  reading the list *before* opening its own file, which worked and was fragile:
  it depended on statement order in one function.

  **`aldwin-config` gained a dependency on `aldwin-core`.** The truncation
  rule and the turn count are `LogRecord` semantics, and splitting them from
  the file format would have put half the transcript's meaning in aldwin-cli.
  Acyclic — core depends on no workspace crate — but it is a new edge and worth
  knowing about. `aldwin-tui` did *not* gain one: it holds aldwin-config as a
  dev-dependency only, which is why the session list is handed to it as display
  rows rather than read.

  **The picker is a second control, not a generalised one.** `ResumePicker`
  sits beside `ModelPicker` rather than `ModelPicker` being made generic over
  its stages, and both draw through one `ui/picker.rs`. The shape is shared;
  the state is not. Generalising a two-stage picker to serve a one-stage list
  was more indirection than either screen wanted.

- **2026-09-20 — cut to a V1.** The previous draft carried ten steps including
  `--continue`/`--resume` flags, a `history.yaml` retention policy with
  `/history prune` and `/history forget`, and a documented opt-out. Three
  things came out of that. The flags contradicted
  `archive/aldwin-cli.md:209`'s zero-arg Decision, which the draft did not
  notice — dropping them removes the contradiction rather than needing a second
  reversal to resolve it.
  Retention became a named gap rather than a config domain. And the
  approval-card loss, which the old Step 6's "one read, two consumers" claim
  hid, is now a stated Decision: the TUI's `LogEntry` is a different vocabulary
  from `LogRecord` and the gap between them is chosen, not discovered. What was
  added is the last-complete-turn truncation rule, which removes the unmatched
  tool-use failure the old draft would have hit on the first `kill -9`.

- **2026-09-24 — the swap moved into core.** `/clear` and `/resume` moved the
  writer as they passed the interceptor, guarded by a "turn in flight" flag
  that is false between a review's follow-up turns; a command landing there
  moved the writer, core then refused it, and the rest of the conversation
  went into another session's file. `RecordSink` now has `cleared` and
  `resumed(&SessionId)`, which `ConversationLog` calls when core acts, and
  `Command::Resume` carries the id. The interceptor's flag stays as the early
  word only. Two read/write defects closed with it — see aldwin-config.md's
  same-day entry: one `write` per line, and a non-UTF-8 torn line costs that
  line alone.
