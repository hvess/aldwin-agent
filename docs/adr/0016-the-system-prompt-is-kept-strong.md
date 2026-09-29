# ADR 0016 — The system prompt is kept strong, and checked after every change

**Status:** accepted, 2026-09-29
**Supersedes:** ADR 0008's Consequences, "Each clause here traces to an
observed failure rather than to a worry, which is the bar for adding
another." ADR 0008's decisions §1–§7 stand.
**Amends:** `aldwin-core`'s `prompt::BASE`, now `crates/core/src/prompt.md`;
the `records` and `review` skills (the code judge's fifth source)
**Affects:** `aldwin-core`, `aldwin-tools` (tool descriptions),
`aldwin-cli` (the session context), `aldwin-review`

## Context

ADR 0008 set a bar for the prompt: a clause is added only when a transcript
shows the failure it prevents, because prompt length is a cost on every
call. The bar kept the prompt short, and it also kept it from teaching
anything no transcript had yet gone wrong on. On 2026-09-29 the developer
read Claude's own consumer system prompt as learning material and chose to
apply its lessons in full: rules that carry their reasons, worked examples
for hard judgments, stated precedence, tool output treated as material
rather than instructions.

Checking the rewritten prompt against the code the same day showed what
the old bar could not catch. The prompt told the model to "stage the edits
again" after a review's comments, when a commented changeset stays staged;
it never said that the calls in one response run at the same time, so an
edit and its check could go together and test the old code; and `explain`
took 0-based positions while returning 1-based ones. None of these was a
failure any transcript had shown yet, and no test or judge read the prompt
against the code.

## Decision

### §1 The prompt teaches, not only patches

A clause may be added when it tells the model how Aldwin works, or how to
do the work well, and not only when a transcript shows its absence. Each
rule carries its reason, a hard judgment gets a worked example (Good, Not,
Why), and a new rule replaces the sentence it overrides rather than
contradicting it from a later section. The developer's call, 2026-09-29:
"everything we can do to improve how the agent performs should be done."

### §2 Replies are short, direct and exact

The prompt's "How you answer" section comes second, after which instructions
win, and governs every reply. Answer first, name the real file, line and
value, use plain words rather than jargon, and never answer with a wall of
text. The developer's call, 2026-09-29.

### §3 The prompt is checked after every feature and fix

`crates/core/src/prompt.md` and the tool descriptions are all the model
knows about how Aldwin works, so they change in the same commit as any
change to what a tool takes or returns, when the review opens, or anything
else the model acts on. The `records` skill says how. The review's code
judge reads them as its fifth source: a prompt stating what the code no
longer does is a major finding, and one silent on something new the model
must act on is minor.

## Consequences

- **The prompt is longer** — about 4,900 words against ADR 0008's 700 —
  and it is sent on every call. It sits in the cached prefix, so the cost
  is mostly the first call of a session. Length is still weighed: a clause
  that teaches nothing the model would otherwise get wrong does not go in.
- **A stale prompt now fails the review**, where before it was invisible
  until a session went wrong.
- **The prompt's tests pin its rules**, not its length: each rule a later
  edit could lose has an assertion in `prompt.rs`.

## Alternatives rejected

- **Keep ADR 0008's bar and add clauses one failure at a time.** Every
  lesson would cost a failed session first, and the bugs above would each
  have had to happen before they could be written down.
- **Check the prompt only when it is edited.** The prompt went stale
  through changes to the code, not to the prompt; the check has to follow
  the code.
