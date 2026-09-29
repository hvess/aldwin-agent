---
name: comments
description: How to write code comments in Aldwin — lean, concise, and written for the reader most likely to read them, an LLM working on the code. Says what a comment is for, what an LLM needs from one and what it does not, and when to write none. Use whenever writing, editing or reviewing a comment or doc comment in any file (Rust, shell, TOML, scripts).
---

# Comments

The code in this repository is read mostly by LLMs, so comments are written
for an LLM. An LLM reads the code itself well: syntax, types, control flow,
names. It cannot see what is not in its context, and it believes what a
comment tells it. So a comment carries only what the code cannot, and it
must be true.

## What an LLM needs from a comment

Write a comment only to give one of these. Each is something an LLM reading
the code in front of it would otherwise get wrong.

1. **The reason, when the code alone suggests a different choice.**
   `// Loop, not iterator: the body awaits.`
2. **An invariant the types do not enforce.** What must stay true, and what
   relies on it.
   `/// Never empty: \`open\` refuses an empty changeset; \`file()\` indexes it.`
3. **A warning against a plausible wrong change.** LLMs "improve" code. If
   the obvious refactor breaks something, say so and say what.
   `// Do not follow links here: this write runs outside the sandbox.`
4. **A dependency the reader cannot see.** Code elsewhere, another process,
   a file format, or an external tool that relies on this exact behaviour.
   `// Parsed by .githooks/pre-commit: keep the variable name.`
5. **A pointer to the authority.** The ADR, spec section, or test that owns
   the rule, so the reader can open it instead of guessing.
   `// ADR 0011 §3: tell the developer once, never silently.`
6. **A fact about a value that its type does not carry.** Units, ranges,
   encoding, ordering, who owns it.
   `/// Milliseconds since the turn started.`

## What an LLM does not need

Delete or do not write:

- **What the code does.** The reader can read it. `// increment the counter`
- **History.** How it used to work, when it changed, the bug that led here,
  who decided. That is git's and the ADRs' job. A dated sentence
  ("until 2026-09-20…", "the first version…") goes.
- **Argument and rhetoric.** Persuasion, metaphors, asides, emphasis for
  effect, "this is the whole point". State the fact once.
- **Jargon and invented shorthand** the reader would have to decode. Use the
  names in the code, the ADR numbers, the file paths.
- **Anything a better name would say.** Rename instead.
- **A copy of something written elsewhere.** Point to it.
- **Commented-out code**, and a **`TODO`** without an open-tasks entry
  (quality-gate §6).

## How to write one

- **Default to one line.** Add a line only when it carries a separate fact
  from the list above.
- **Plain, declarative, present tense.** State the rule: "Must", "Never",
  "Always", "Only". No "we", no "you", no hedging.
- **Use exact anchors:** identifiers, paths, `ADR 0013`,
  `aldwin-review.md Decision 15`, test names. An LLM can search for these.
  **Every anchor must resolve:** grep the test name, path or section before
  writing it; "the spec" is not an anchor, `docs/spec/aldwin-login.md` is.
- **Claim no more than holds.** "Every", "never", "all", "unreachable" and
  "only" must be true across the whole scope; otherwise name the scope
  ("the plain spellings", "except a damaged transcript").
- **Put it where the fact applies**, on the line or item it is about, not
  in a summary at the top of the file.
- **Keep it true.** Changing code means changing or deleting its comment in
  the same edit. A stale comment misleads an LLM more than a missing one.
  After changing a function, re-read: its item doc; its `# Errors` and
  `# Panics` (a condition added, removed or still reachable); the module
  doc; docs on callers that describe it; its tests' names and docs; and any
  comment calling a path "unreachable" or a case "impossible".

## Doc comments (`///`, `//!`)

The workspace lints require docs on public items (`missing_docs`) and
`# Errors` / `# Panics` sections (`clippy::missing_errors_doc`,
`clippy::missing_panics_doc`). Keep them to the contract:

- **Item doc:** one sentence saying what it is or returns. Add the invariant,
  hazard or pointer from the list above only if there is one.
- **Field and variant docs:** what the value means, in a phrase, when the
  name does not already say it all; the lint still needs a doc, so write the
  shortest true one.
- **`# Errors` / `# Panics`:** the conditions, as a short list or one
  sentence. Not why they are errors.
- **No `# Examples`:** the code is the example (`rust` skill).
- **Module doc (`//!`):** one or two lines — the module's single
  responsibility, and the one invariant or pointer a reader must know before
  editing it.

## Tests

A test's name states the behaviour. Add a comment only when the name cannot
carry what the test guards against — then one line: the failure it pins.
`/// Regression: a damaged record was reported as missing.`

## Other files

The same rules hold for shell, TOML, YAML and script comments: only what the
file cannot say itself, one line by default, exact anchors.

## Checking a comment

For each comment, in order:

1. Does it say what the code already says? Delete it.
2. Is it history, argument, or a copy of something elsewhere? Delete it, or
   replace it with a pointer.
3. Does it state a reason, invariant, warning, hidden dependency, pointer or
   value fact that the reader needs? Keep that fact, in as few words as
   carry it.
4. Is it still true of the code beside it? If not, fix it or delete it.
