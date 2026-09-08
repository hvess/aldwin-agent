# mjolnir-history

Persisted conversation transcripts: sessions survive process exit, and a past
session can be listed, read back, and resumed.

**Status:** draft — high-level outline only, not approved, nothing implemented.
Blocked on an ADR (see Blocking Decision below).
**Scope:** Session identity, on-disk transcript format, write path, resume/load
path, the `/resume` picker, and retention. Excludes the LLM wire format, the
permission engine, and how a transcript is *rendered* (mjolnir-tui already owns
that and needs no new entry shapes).
**Owner:** Maximilian
**Last Updated:** 2026-09-07

## Blocking Decision

`.claude/spec/mjolnir.md` line 83 states, as a V0 Decision: *"No session
persistence, no first-class history, developer-authored memory in V0. Sessions
are ephemeral."* `mjolnir-permissions.md`'s Out of Scope repeats it ("Session
persistence and prompt history — out of V0 per parent"). This feature reverses
that constraint and introduces a persisted format, which is exactly what
`.claude/adr/` exists for. **Write an ADR before Step 1** (0002 is taken — see
`.claude/adr/0002-markdown-tables-are-drawn.md` — so this is 0003). It has to answer,
at minimum: what changed since V0 to make ephemerality the wrong default,
whether history is on by default or opt-in, and whether "the developer's
understanding is the product" is served or eroded by a transcript that outlives
the session. Until that ADR lands, this spec is a sketch, not a plan.

## Why

Every session currently dies with the process. A crash, an accidental `Ctrl+C`,
or simply closing the terminal loses the reasoning as well as the output — and
in a harness whose premise is that the *discussion* is the artefact, that is the
one thing least affordable to lose. Resume also removes the tax on quitting: if
picking a session back up is cheap, stopping to think is cheap.

## Vocabulary

- **Session:** One process lifetime's conversation, identified by a `SessionId`
  minted at bootstrap. `/clear` ends one and begins another (see Decisions).
- **Transcript:** The on-disk JSONL file for a session — a header line plus one
  line per `LogRecord`.
- **Resume:** Loading a transcript back into `ConversationLog` before the first
  turn, so `messages_from_log()` sees it and the TUI renders it.
- **Sealed:** A session no longer being written to. Sealing is implicit (the
  process exited) — there is no close record to lose.

## Model

- **The record already exists.** `LogRecord` (`crates/core/src/event.rs:120`)
  is already `Serialize`/`Deserialize` and is already the committed,
  streaming-stripped form of the conversation. History is persistence for a
  type that is otherwise complete; it does not introduce a second
  representation of a conversation.
- **Append-only JSONL, one file per session,** at
  `~/.mjolnir/history/<project-slug>/<session-id>.jsonl`. First line is a header
  (`started_at`, `cwd`, provider/model, schema version, `title`); every
  subsequent line is one `LogRecord`. JSONL because writes are appends and a
  torn tail costs one turn, not the session — the reader drops a trailing
  unparseable line and continues.
- **The title is derived, not authored.** First user message, trimmed to one
  line. No LLM call to summarise a session: that would spend the developer's
  tokens on filing.
- **Writes are best-effort.** A failed history write emits `Event::Notice` and
  the turn proceeds. History must never be able to fail a turn.
- **Resume rehydrates two places from one source.** The loaded records go into
  `ConversationLog` (so the model sees them) and through the TUI's existing
  `apply_event` path (so the developer sees them). One read, two consumers, no
  divergence.
- **Nothing re-executes on resume.** Tool calls and results replay as text.
  Permission grants are session-scoped and do not come back; a resumed session
  re-asks. Default-deny is not weakened by resume — this is the constraint most
  at risk here and it is not negotiable.

## Interfaces

- **`SessionId`** — new id type alongside `TurnId`/`StepId` in
  `crates/core/src/types.rs`. Minted once, in mjolnir-cli's bootstrap.
- **`HistoryStore`** — lives in mjolnir-config (it already owns `fsio` and the
  `~/.mjolnir/` layout). Surface: `open(session)`, `append(&LogRecord)`,
  `list(project) -> Vec<SessionSummary>`, `load(session_id) -> Vec<LogRecord>`,
  `prune(policy)`.
- **`ConversationLog`** (`crates/core/src/log.rs`) gains an optional store.
  `append()` fans out to it; `snapshot()`, `len()`, `clear()` are unchanged.
- **Events:** `HistoryLoaded { records }` — the resume counterpart to the
  existing `HistoryCleared`, carrying the records for the TUI to fold in.
- **Commands:** `Resume { session_id }`, reached from `/resume` in mjolnir-cli's
  interceptor (`crates/cli/src/slash.rs`).
- **CLI flags:** `--continue` (most recent session in this project) and
  `--resume [id]` (bare form opens the picker).

## Decisions

- **`/clear` seals and starts a new session; it does not delete.** — Today it
  wipes the log. With history, "forget everything" must stay true for the
  model's context without destroying the record — the developer clears to
  manage tokens, not to shred evidence. A separate `/history forget` handles
  actual deletion, deliberately and by name.

- **The picker asks; the interceptor decides.** — `/resume` with no argument
  opens a one-stage list over the existing `picker.rs` control, and committing
  submits `/resume <id>` as if typed. Same rule `picker.rs` already documents
  for `/model`: one implementation of what the answer *does*, in the CLI's
  dispatch table.

- **Resume replays the transcript, not a summary.** — A summarised resume is a
  second, lossier conversation the developer never read. If context cost
  becomes the binding constraint, the answer is to truncate visibly at a turn
  boundary and say so in the transcript, not to silently compress.

- **History is per project, keyed by project root.** — Sessions are about a
  codebase. A global timeline across unrelated projects is a search problem
  this spec does not want.

- **No cross-session memory.** — This spec persists transcripts. It does not
  propose entries, extract facts, or carry anything into a *new* session. The
  parent's "developer-authored memory" Decision stands untouched.

## Steps

0. **Write ADR 0003** (see Blocking Decision). Nothing below starts first.

1. Add `SessionId` to `crates/core/src/types.rs`; mint it in mjolnir-cli's
   bootstrap and thread it to the store.

2. Define the header record and JSONL schema (with a `version` field) in
   mjolnir-config; implement `HistoryStore::open`/`append`.
   - Verify: a killed process leaves a readable file whose last line may be
     partial and whose earlier lines all parse.

3. Wire `ConversationLog::append` to fan out to the store. Failure path emits
   `Event::Notice`.
   - Verify: an unwritable history directory does not fail a turn.

4. Implement `list` and `load`, with the trailing-partial-line tolerance.

5. Change `/clear` to seal-and-open rather than wipe-only; keep
   `HistoryCleared` semantics for the in-memory log and the rendered transcript
   exactly as they are.

6. Add `Event::HistoryLoaded` and fold it in `App::apply_event` so a resumed
   transcript renders through the same path as a live one.

7. Add `--continue` / `--resume <id>` to the CLI; load before the first turn.

8. Add the `/resume` picker — one list, rows of `date · title · turns`, over
   `picker.rs`'s existing control. No strokes: bands on the ground ladder, the
   fixed glyph set, lowercase labels.
   - Verify: measured against the design system's list frame, not eyeballed —
     see `.claude/design/IMPORT.md` rule 4.

9. Retention: a `history.yaml` (or a `tui.yaml` section) with a count/age
   policy, `/history prune`, and `/history forget <id>`.

10. Note completion in this spec's Progress; move to `.claude/spec/archive/`
    when every step is done, and amend `mjolnir.md` line 83 and
    `mjolnir-permissions.md`'s Out of Scope line to point at ADR 0003.

## Pitfalls

- **History as a secrets surface.** Tool results carry file contents and
  command output — API keys read out of a `.env`, tokens echoed by a shell
  call. Ephemeral sessions made this a memory-lifetime problem; a transcript
  makes it a disk-lifetime one. Mode `0600`, project-scoped directories, and a
  documented opt-out are the floor, not polish.
- Resume quietly restoring permission grants along with the transcript, on the
  reasoning that "the developer already approved this" — it re-creates a
  persistent allowlist through the back door and breaks the ADR 0001 grant
  model.
- The context cost of resume growing unbounded, so a resumed session's first
  turn is its most expensive. Decide the truncation rule in Step 7, not after
  the first complaint.
- The header's `title` drifting toward an LLM-generated summary because the
  first user message reads badly in a list. Fix the list, not the source.
- Writing a close/seal record and treating its absence as corruption — a killed
  process never writes one. Sealing must be implicit.
- `snapshot()`'s O(n) clone (`log.rs:16`) is fine once per turn but is *not*
  fine as the write path. Append one record; never re-serialise the log.
- Two Mjolnir processes in the same project appending to the same file. Distinct
  `SessionId`s make this a non-issue by construction — keep it that way rather
  than adding a lock.

## Out of Scope

- Cross-session memory, fact extraction, or end-of-session prompts to remember
  anything — parent spec, and deliberately.
- Search over transcripts (full-text, semantic, or otherwise). Listing and
  resuming only.
- Export to any other format, and sharing a transcript off-machine.
- Editing or redacting a past transcript in place beyond whole-session
  `forget`.
- Rendering changes in mjolnir-tui — resume reuses the existing entry shapes,
  and if it needs new ones that is a signal the fold in Step 6 is wrong.
- Syncing history between machines.

## References

- `.claude/spec/mjolnir.md` — parent; line 83 is the Decision this feature
  reverses.
- `.claude/spec/mjolnir-permissions.md` — Out of Scope names session
  persistence as V0-excluded; grants deliberately do not survive resume.
- `.claude/spec/archive/mjolnir-core.md` — `ConversationLog`, `LogRecord`, and
  the event/command surface this plugs into.
- `.claude/spec/archive/mjolnir-config.md` — `~/.mjolnir/` layout and `fsio`,
  where `HistoryStore` belongs.
- `.claude/spec/archive/mjolnir-cli.md` — the slash dispatch table `/resume`
  joins.
- `.claude/spec/mjolnir-tui.md` — `picker.rs`'s "the picker asks, the
  interceptor decides" rule; the 2026-09-06 layout Progress entry.
- `.claude/adr/0001-tool-level-permission-grants.md` — the grant model resume
  must not undermine.
