# ADR 0008 — Discussion-first is about intent, not grammar

**Status:** accepted, 2026-09-21. Superseded in part by 0016 (the bar
for adding a prompt clause, in Consequences).
**Amends:** the "Discussion-first" non-negotiable in `AGENTS.md`;
`aldwin-core`'s `prompt::BASE`
**Affects:** `aldwin-core`

## Context

`prompt::BASE` said:

> Your resting state is discussion: read, explain, analyse, surface tradeoffs.
> You act only on explicit instruction ("apply this", "go ahead", "do it").
> Questions, hypotheticals, and exploratory language get analysis only.

The intent is right and is the product. The implementation made **grammatical
mood** the trigger, with three phrases as the password. A session transcript
from 2026-09-21 shows the cost, and in each case the agent was *complying*:

- The developer, four turns into an approved plan, said "For our purposes, we
  don't need to use proton-authenticator". The agent explained the
  implications, offered two options, and asked. Told "proton-calendar is also
  out", it offered **the same two options again** and asked again. Two turns,
  no work, on a decision the developer had already made twice.
- "What I am thinking is that we can create a script that…" — exploratory
  language. The turn produced no text and no tool call. The developer typed
  "Continue".
- "Do we need the script if this is already a single line?" — a question,
  therefore analysis only. The agent opened "Honestly — no, not strictly",
  filed most of the work it had just built and tested under "not actually
  necessary", and offered to delete it. The developer deleted it. The session's
  entire deliverable was talked away by its author under one question.

A separate failure in the same transcript has no rule against it at all. Asked
three times to show a file, the agent ran `cat`, received the complete 4,039
characters each time, and replied with a description of the contents — *"that's
the full, current contents … 135 lines, read directly from disk just now"* —
without pasting them. The developer: *"You never showed me."* … *"You never
printed anything out, I don't get why."* Tool results go to the model, not to
the developer, and nothing said so.

## Decision

Discussion-first is kept. A question about how something works still gets an
answer rather than a change; `edit` still shows a diff and waits. What is
removed is the idea that a sentence must be an imperative to count.

### 1. Intent, not mood

Act when the developer's intent is clear. A constraint they state is an
instruction: "this repo is out of scope" means take it out of scope.
Describing what they want built is asking for it to be built. Once a plan is
agreed, carry out the whole of it without re-confirming each step.

### 2. Never offer the same choice twice

If an option has been put to them and they answered the substance, act on it.
Where one obvious default exists, take it and say so in a line.

### 3. Ask before expensive work, not after cheap work

The same transcript asked permission for two trivial reversible edits and
asked nothing before a 45-minute clone of seven repositories, two of which the
developer discarded two turns later. The question budget was spent in exactly
the wrong place.

### 4. Tool results are not visible to the developer

Stated explicitly, because nothing stated it. When asked to see something, put
its content in the reply — do not describe it or vouch for it.

### 5. Stand behind finished work

A question is not disapproval. Give the real tradeoff, including what dropping
the work would cost. Changing your mind needs a reason you can name.

### 6. Declare reads as reads

`write` is not the cautious choice; it is a wider grant that spends a prompt.
A refused read declaration is a question about one call, not a verdict on the
class. (ADR 0007 §6 and §8 are what make this advice followable rather than
a trap.)

### 7. Read the error before retrying

An unchanged call fails the same way; a search that timed out gets narrowed,
not repeated.

## Consequences

**The non-negotiable in `AGENTS.md` is reworded**, from "Action only on
explicit developer signal" to intent-based. It is still a constraint and still
non-negotiable; what changed is what counts as the signal.

**The structural protection is unchanged and is where it always was.** `edit`
shows a diff and waits, and is not grantable. Conversational gating was never
the safety mechanism — ADR 0004 put that in the permission model and the diff
gate — so relaxing it costs no guarantee. ADR 0007 §2 closes the route by
which work was escaping the diff gate entirely, which is the change that makes
this one safe.

**The prompt is longer**, and prompt length is a real cost on every call. Each
clause here traces to an observed failure rather than to a worry, which is the
bar for adding another.

**This will sometimes act where the old text would have asked.** That is the
point, and the failure mode is bounded: anything that writes a file shows a
diff first, anything that runs a program is contained by ADR 0007 and gated by
ADR 0004.

## Alternatives rejected

- **Leave the prompt and fix the harness only.** The three worst turns in the
  transcript were prompt compliance, not harness defects. Nothing in the
  harness would have changed them.
- **Delete discussion-first entirely.** It is the product's identity and the
  reason the diff gate is tolerable. The pathology was the literalism, not the
  stance.
- **Enumerate more trigger phrases.** More passwords for the same lock.
- **Handle "show me the file" in the TUI by rendering `run` output to the
  developer directly.** Worth doing and does not replace this: it fixes one
  shape of the failure, and the general rule is that the model must know what
  the developer can and cannot see.
