# ADR 0005 — A session outlives its process

**Status:** accepted, 2026-09-20
**Supersedes:** one clause of the V0 Decision in `.claude/spec/aldwin.md` line
83 — "Sessions are ephemeral" — which is amended to point here. (An earlier
draft of `aldwin-history.md` also named a matching line in
`aldwin-permissions.md`'s Out of Scope; that spec was rewritten for ADR 0004
and no longer carries one.)
**Affects:** `aldwin-core` (`RecordSink`, `Command::Resume`,
`Event::HistoryLoaded`), `aldwin-config` (`history.rs`, the `~/.aldwin/`
layout), `aldwin-cli` (`/resume`, `/clear`), `aldwin-tui` (the session
picker), `.claude/spec/aldwin-history.md`

## Context

V0 decided that sessions are ephemeral: no persistence, no first-class history,
developer-authored memory only. That Decision bundled three separate things —
**a transcript on disk**, **cross-session memory**, and **the agent proposing
what to remember** — and rejected all three together. The second and third are
the ones the harness's premise actually argues against: an agent that carries
its own summary of yesterday into today is an agent whose understanding, not
the developer's, is the artefact being accumulated.

The first is not the same claim, and eighteen months of using the thing made
the difference visible. Every session dies with the process. A crash, a
mistaken `Ctrl+C`, a closed terminal, and the *reasoning* goes with the output
— in a harness whose whole premise is that the discussion is the product, that
is the one thing least affordable to lose. Ephemerality also taxes stopping:
if picking a conversation back up is impossible, quitting to think costs the
thinking you already did, and the developer stays in a session they should
have left.

## Decision

**A conversation is written to disk as it happens, and `/resume` picks one back
up.** One session is one append-only JSONL transcript under
`~/.aldwin/history/<project-slug>/<session-id>.jsonl`, mode `0600`. Resuming
replaces the running session's `ConversationLog` and rendered log with the
loaded records and continues writing into that same file.

Four boundaries make this a persistence decision rather than a memory one:

1. **Nothing crosses session boundaries by itself.** Resume is an explicit act
   naming an explicit session. No transcript reaches a *new* session, nothing
   is extracted from one, and the agent never proposes that anything be
   remembered. V0's actual argument is untouched — see Consequences.
2. **The transcript is the conversation, never a summary of it.** Resume
   replays what was said. A summarised resume would be a second, lossier
   conversation the developer never read, which is precisely the failure mode
   V0 was guarding against, arriving through a different door.
3. **Nothing re-executes, and no grant comes back.** Tool calls replay as text.
   Permission grants stay session-scoped and a resumed session re-asks. "The
   developer already approved this" would rebuild a persistent allowlist behind
   ADR 0004's back.
4. **Only `/resume` reaches it.** There is no `--resume` or `--continue` flag,
   so `archive/aldwin-cli.md:209`'s zero-arg Decision stands unreversed. A
   launch-time entry point is a separate question for a separate ADR.

## Consequences

**The V0 Decision is narrowed, not deleted.** `aldwin.md` line 83 keeps its
other two halves: memory is still developer-authored, and Aldwin still does
not propose entries or prompt at end of session. What changes is the first
clause only.

**A transcript is a secrets surface with a disk lifetime.** Tool results carry
file contents and command output — a key read out of a `.env`, a token echoed
by a command. This was a memory-lifetime exposure and is now a disk-lifetime
one. `0600` and a project-scoped directory under `~/.aldwin/` are the floor.
There is no opt-out, by decision (the developer, 2026-09-27): transcripts are
always written.

**Nothing prunes.** Transcripts accumulate until the developer deletes them.
Clearing out `~/.aldwin/history/` is their business, like any other directory
of their own files. A retention policy earns a config domain when it becomes
annoying, not before.

**A resumed transcript renders the conversation, not the session.** Approval
cards and permission prompts do not come back: they exist only as the TUI's
`LogEntry` and never as a `LogRecord`, and persisting them would widen the
record type for a UI concern. The developer sees what was said; the decisions
are re-asked rather than re-displayed.

**A crash resumes to the last finished turn.** A process killed between a tool
call and its result leaves an unmatched `ToolUse` on disk, and a request
carrying that block is rejected outright — so the load truncates after the last
`TurnEnded`. The crash case resume exists for is the case that made the rule
necessary.

## Alternatives considered

**Keep sessions ephemeral and let the developer copy what matters out.** This
is the status quo, and it is what the developer was already doing — which is
the evidence against it. Copying happens after you know something was worth
keeping, and a crash is precisely the case where you did not get the chance.

**Persist, but summarise on resume to bound context cost.** Rejected under
Decision 2. The cost is real — a resumed session's first turn is its largest —
but the honest answer to it is to truncate visibly at a turn boundary and say
so, not to silently hand the developer a conversation they never had.

**Fork a new transcript on resume rather than continuing the old one.** It
preserves "one process, one file" and rules out two processes appending to one
transcript. Rejected because one conversation becoming three files after two
resumes is not the model the developer has of their own work. The concurrency
risk is bounded by `O_APPEND` line atomicity and by nobody having asked for
concurrent sessions in one project; it stops being acceptable the day a
launch-time `--resume` exists.

**A launch-time `--resume` / `--continue`.** The obvious entry point, and it
would have reversed a second stated Decision (zero-arg binary) inside an ADR
about a first. `/resume` is one keystroke later and needs no such reversal.
