# 0009 — The review is the only gate

**Status:** accepted, 2026-09-23.
**Supersedes:** ADR 0004 §1–§4, §6 and §8; ADR 0003 in full; the first-run
Decision in `aldwin.md`. **Keeps:** ADR 0004 §5 and §7 (reach and the deny
lock), ADR 0007 in full, ADR 0002, ADR 0005, ADR 0006, ADR 0008.
**Amends:** `CLAUDE.md`'s first, second and fourth non-negotiables, which
now read as §1–§3 below state them.

## Context

The Aldwin Design System replaced the Mjolnir system on 2026-09-23, and its
README opens with a sentence about behaviour, not paint:

> Aldwin reads and runs what it needs without asking, says what it is doing,
> and never edits a file directly. Every edit opens as a full-window review,
> and nothing is saved until you approve.

That is a different product from the one ADR 0004 built. 0004 made every
tool call a question — default-deny, a grant per program and class, an
eight-row prompt, a standing rung chosen on first run — and put `edit`
outside that model with its own per-call diff gate. The design has no
permission frame at all: its one question surface is a `QuestionPanel` the
*agent* asks through, and its one gate is the review.

The developer was asked whether to follow the design fully, follow it but
keep a prompt for write-class runs, or keep ADR 0004 and take only the
design's tone. The answer was the first, and the six decisions below are
what that answer means when built.

## Decisions

### §1 Reads and runs need no grant

A `read`, an `explain`, and a `run` execute when the model calls them. There
is no allow list, no standing rung, and no prompt. The `permissions.yaml`
keys `allow:` and `default:` are still *parsed* — a file written by the
previous first run carries both, and refusing to load it would stop every
existing project from starting — but nothing reads them, they are not
written, and the session says so once at startup. `Locks::stale_keys` is the
whole of that compatibility.

**What holds the line instead is the sandbox.** A `run` still declares its
class, and a call declared `read` still executes with every root read-only
and the network unreachable (ADR 0004 §4, kept). What changed is the answer
to a refused read: it goes back to the model as an error saying the call was
a write and to declare it so, not to the developer as a prompt. There is no
prompt to send it to.

### §2 A deny is still a lock

ADR 0004 §7 stands unchanged. A `deny:` entry in either `permissions.yaml`
— a program, optionally qualified by class — refuses the call outright, the
refusal names the file, and nothing narrower overrides it. `aldwin-permissions`
is now exactly this: a `Locks` type over two deny lists, with `Outcome::Allow`
and `Outcome::Locked` and no third variant.

### §3 Where a read cannot be enforced, it runs unconfined and says so once

0004 §4 said "every call asks" on a platform with no sandbox, and ADR 0007
§6 built that. With no prompt in the product, the fallback is: the call runs
with the tree writable, and the developer is told once per session —
*"Runs are not sandboxed on this system"* — at the first such call rather
than at startup, so a session that never runs anything never hears it.

This is a stated weakening, not an oversight. The alternative — refusing
every read-declared call on macOS-without-Seatbelt or Linux-without-Landlock
— is the flat error ADR 0007 records teaching the model to declare `ls` a
write 69 times. The one-time notice is what makes the weakening visible.

### §4 An edit is staged, and the review is the gate

`edit` writes nothing. It applies its change to a **staged overlay**
(`aldwin_tools::Staging`) and returns *"staged an edit to path"*. `read`
serves the staged content for a staged path, so the model reads back what it
wrote. Every edit of a turn accumulates into one changeset.

**The review opens at the first moment the changeset would be observed on
disk without one** — which is two moments, and `ToolDispatcher` gained a
hook for each:

- `before_step`: a `run`, or an MCP tool (which executes in its own process
  over the real tree), would see the disk. The dispatcher opens the review
  first. If the developer approves, the files are written and the step
  proceeds — so *Check that it works* tests approved code. If they comment,
  the step's calls are answered with the comments as an error result and do
  not run; the model addresses them and calls again. If they discard, the
  calls are answered with that, and the changeset is dropped.
- `turn_ending`: the model stopped calling tools. Whatever is staged is
  reviewed. Approve writes; discard drops; a comment becomes the next turn's
  user message — `Agent::run` starts it without the developer typing, and
  announces it first with `Event::FollowUp` so the TUI can echo what the
  model is about to be asked (a typed message it echoes itself). A discard
  at the turn's end starts a turn the same way, because the model must
  learn its edits are gone before it acts as if they landed; its text is
  written in the developer's voice ("I discarded the staged changes…")
  since that is how the echo and the transcript show it. A review nobody
  answers — the session ending under it — writes nothing, runs nothing,
  and starts no turn (`Reviewed::Gone`); a *cancel* never reaches the
  dispatcher, because the agent drops the review future instead.

Nothing is ever written before an approve. That is literally true, not
true-after-an-undo, which is why undo is open-tasks 27 rather than a
requirement of this ADR. A file that changed on disk between staging and
approve is not overwritten (`Staging::write_all` re-checks `before`), and
the developer is told which.

`ReviewDecision`, `ReviewComment`, `Changeset` and `ReviewOutcome` are
core wire types, and `ReviewRequested`/`ReviewClosed`/`Command::ReviewDecision`
replace `ToolApprovalRequested`/`ApproveTool`/`DenyTool` in the persisted
event vocabulary. `LogRecord` is untouched: a review is not part of the
conversation the model sees.

### §5 A failure is a sentence

The design has no `✗`, no `!`, and "green and red appear only in diffs". A
turn that errors, a cancelled turn, a provider retry and a failed run are
each a sentence in `label` with the detail one disclosure below in `label2`.
No status hue, no status glyph. `LogEntry::Failure` is that row.

### §6 There is no first run

"No setup: every launch opens straight to the field." The three-step wizard
is gone with the access rung it existed to ask. Provider and model are
answered where they are needed: with nothing configured the launch card reads
`Model  not set`, the first message is held while two `QuestionPanel`s ask
provider then model, and `/model provider/model` writes the global file
because there is no other default for every other directory to inherit. A
session with no provider starts on an `Unconfigured` client that answers
with the one true thing.

`CLAUDE.md` and `AGENTS.md` at the project root are read into the context
without asking — reading is a read (§1). The stdin prompt that asked about
each one is gone.

### §7 Two tools carry structure to the screen

`plan` declares and advances the plan as outcomes in plain words; the TUI
draws the latest list as `PlanStep` rows. `ask` poses one question with a
line of why and a short list of answers; the tool appends *Chat about this*
if the model left it out, because that row is the developer's way out of a
question that was wrongly framed. The answer returns as the tool result —
the option's text, or what was typed. Both are outside the lock; neither
runs anything.

## Consequences

- `aldwin-permissions` shrank from an engine to a lock. `Choice`,
  `PromptPayload`, `PromptResponse`, `ContextFileTier`, `Rung`'s use, the
  session layer and `record` are gone.
- `aldwin-tools` gained `Staging`, `PlanTool` and `AskTool`; lost `gate.rs`,
  the permission-prompt path and the read-refused re-run. `Tool::permission`
  returns `Option` — `None` is outside the lock.
- `aldwin-core` gained `before_step`/`turn_ending`, three round-trip event
  pairs and `PlanUpdated`; lost the approval pair. `Agent::run` can start a
  turn from a review comment.
- `aldwin-tui` was rewritten against the new design: no label column, no
  first run, no permission panel, no syntax highlighting; a review screen, a
  question panel, a launch card and a context bar. See `aldwin-tui.md`'s
  2026-09-23 entry.
- `aldwin-cli` lost the wizard and the stdin prompt; `/quit` joins `/exit`;
  `/model` works from nothing.
- **Commands the design draws and the product does not:** `/changes`,
  `/undo` and `/settings` are in the frame and not in the menu, by the
  developer's decision; `baseline.json` records both halves.

## What this does not decide

- Undo (open-tasks 27). §4 makes it unnecessary for safety; it may still be
  wanted for convenience.
- Reopening a saved review's diff from its row (`›` in frame `J`).
- `explain` over staged edits: the LSP reads the disk, so a symbol lookup
  after an edit sees the old code. Recorded as open-tasks 28.
