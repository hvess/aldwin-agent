# aldwin-open-tasks

Work that is known, understood and not done. Each entry says what was seen, where the evidence is, and what would close it.

**Status:** active — a ledger, not a spec. Nothing here blocks anything else.
**Scope:** everything outstanding as of 2026-09-23. The redesign of that
date (ADR 0009) closed or mooted eleven entries and added five (27–31); the
closed ones are kept below, struck through, until the next renumbering.
**Owner:** Maximilian
**Last Updated:** 2026-09-23

An entry leaves this file by being done, or by being decided against — in
which case the decision goes where it belongs (an ADR, or the spec it
contradicts) and the entry says so before it goes.

## Design

1. ~~**The design reference contradicts itself, and the app compensates.**~~
   **Moot, 2026-09-23.** The Mjolnir design system and all seven of its
   contradictions are gone with it. `crates/review/baseline.json` now carries
   three: ADR 0002's table (the new system has no table component either),
   and two that are the design against a *decision* — the frame's command
   list and its `↺ Undo` — see entries 27 and 29.

2a. ~~**The design system upstream is still named Mjolnir.**~~ **Done,
   2026-09-23.** The upstream project is "Aldwin" (`b9de8837-…`); every
   address in `.claude/design/` and the `design-sync` skill names it.

2. ~~**The design system ships no reference frames.**~~ **Done, and the
   frame is committed now**: `.claude/design/frames/Aldwin Agent TUI.dc.html`
   (71 KB, token-authored, the only place the brand mark's cells exist). The
   automatic half is still owed: nothing compares a rendered design frame to
   a rendered app frame on a schedule. Landmark positions are the comparable
   part, and the frame being token-authored makes those readable from the
   markup.

21. ~~**A panel dims the ink behind it but not the ground.**~~ Moot since the
    repaint; the scrim itself is gone with the permission panel.

22. ~~**Two designed screens have no implementation at all.**~~ **Done,
    2026-09-23.** The command menu (`ui::question::draw_commands`) and the
    full-window review (`ui::review`, `crate::review`) are built against the
    new design's frames `F` and `G`–`J`.

23. ~~**The top bar draws two facts where the design draws four.**~~
    **Done, 2026-09-23.** There is no top bar. The context bar in the footer
    reads the last step's prompt size over the model's context window
    (`StatusInfo::context_percent`, `Model::context` in the catalogue); the
    launch card carries version, project, branch and model. There is no
    cost figure and the design draws none.

27. **No undo, and no `/changes`.** Frame `J` offers `↺ Undo` after a save
    and frame `F` lists `/changes` ("everything changed since you started")
    and `/undo`. The developer decided against both for this pass
    (2026-09-23): ADR 0009 §4 makes undo unnecessary for safety — nothing is
    written before an approve — and a change ledger is a feature, not a
    guard. `baseline.json` records the frame's side. Closing it means a
    per-review record of `(path, before, after)` kept after the write, a
    footer key that restores it, and a `/changes` list over the session's
    reviews.

28. **`explain` reads the disk, not the staging overlay.** `read` serves a
    staged file's staged content, so the model reads back what it wrote; the
    LSP behind `explain` reads the file as written, so a symbol lookup after
    an edit sees the old code. Not a safety hole — nothing is written — but
    a correctness one for the model. Closing it means either opening the
    review before `explain` too (which would make every post-edit lookup a
    review) or feeding the server `didOpen`/`didChange` notifications from
    the overlay, which is the right fix and the larger one.

29. **A saved review cannot be reopened.** Frame `J` draws `›` at the end of
    the `✓ Saved 3 files` row and the README says "one line you can open
    again". The changeset is dropped at the write, so there is nothing to
    reopen; the row is drawn without the `›`. Closing it shares a record
    with entry 27.

30. **The screenshot harness cannot drag the mouse, and presses `⌃↩` only
    as a literal Kitty sequence.** `keys.rs` names arrows, Enter, Tab, Esc,
    Space and Backspace. `scene.rs` sends `⌃↩` as the raw `\e[13;5u` for
    the `saved` scene, which works because the app pushes the Kitty flag at
    startup. A review selection is a mouse drag (ADR 0010), which the key
    grammar has no way to send — so `selecting` and `commented` exist only
    as `render_snapshot.rs` scenes, seeded with `Review::select`. An SGR
    mouse sequence written to the pty is the honest fix, and needs no
    compositor.

31. **`working` and `running` are transient and the harness cannot hold
    them.** The fake provider answers at once, so the amber `● Working…` is
    never on screen when a frame is taken. `Canned::SseThenStall` would
    hold a turn open mid-reply; a scene built on it would capture the
    working footer with a half-streamed sentence, which is a real state.
    Not done because the capture's quiet-window rule would then wait forever
    — it needs a per-scene override.

32. **A failed turn's kind reaches the TUI as a string.**
    `TurnEndReason::Error(String)` carries `LlmError`'s `Display`, and
    `log::failure_sentence` reads its prefixes (`network error:`,
    `provider error 429:`, …) to choose a sentence you can act on. A new
    `LlmError` variant, or a reworded `#[error]`, falls through to the plain
    fallback without failing any test. The fix is a typed kind beside the
    message in `TurnEndReason::Error`; that changes `LogRecord::TurnEnded`,
    which is written to disk (ADR 0005), so it needs its own ADR.

## Review loop

3. **`cargo fmt` is not in stage 1.** The codebase's aligned struct fields and
   grouped imports need `struct_field_align_threshold` and `group_imports`,
   both nightly-only, and the workspace pins no nightly. Revisit when either
   option stabilises or the project pins a nightly.

4. ~~**No scene reaches two pending prompts.**~~ **Moot, 2026-09-23.** There
   are no prompts. Two `ask` calls in one step would queue as two
   `QuestionAsked` events, and the TUI shows the first; the second replaces
   it when answered. A scene for that is worth having and is not written.

5. **Every scene reaches its fake provider through a bare `base_url`**, so
   `current_provider` is `None`, the launch card's model has no catalogue
   row, and the context bar has no window to divide by — it reads `0%` in
   every capture. Closing it means a scene whose provider is a catalogue
   entry, or a `context` field on the fake's `provider.yaml`.

6. **Scenes exercise the OpenAI adapter only.** `base_url` is ignored for the
   `anthropic` provider, so a defect living only in the Anthropic client is
   invisible to every review.

7. **Key encoding is untested by construction.** The harness chooses the bytes
   a key sends, so it cannot vouch that they are what foot would send. See
   entry 30 for where that now bites.

8. **The loop has never iterated.** No session has gone review → fix →
   re-review → rescore, so the five-iteration cap and stage 5's re-run are
   untested by use.

## History (ADR 0005)

18. ~~**No review scene reaches the `/resume` picker.**~~ **Done 2026-09-20**,
    and re-scripted 2026-09-23: the scene now opens the menu with `/` and
    picks `resume`, which is how the question is reached in the product.

19. **There is no opt-out, and nothing prunes.** Both are deliberate V1 gaps
    named in ADR 0005's Consequences. Transcripts accumulate under
    `~/.aldwin/history/` at mode `0600` until the developer deletes them.

20. **An MCP tool's results land in the transcript with no classification.**
    Every MCP call's result is written to a transcript like any other; there
    is no class to mark one by since ADR 0011. See entry 13.

## Repo

9. ~~**`aldwin-tools`' LSP test needs `rust-analyzer` on PATH.**~~ Done.

10. ~~**`target/` is tracked.**~~ Done.

11. **No CI runs the test suite.** `.github/workflows/release.yml` builds on a
    tag and is the only workflow. `cargo test` and `cargo clippy` could join
    it; the workspace is clippy-clean.

## Permissions and the review (ADR 0009, ADR 0011)

12. ~~**An MCP tool cannot be classified by the developer, so every one is a
    write.**~~ **Moot, 2026-09-23.** Nothing asks, so nothing needs a class
    to decide whether to ask — and since ADR 0011 there is no class at all:
    an MCP server runs in the same write-confining sandbox as `run`.

13. **An MCP tool that edits files does so outside the review.** The one
    hole in the edit guarantee: an MCP call runs in its own process over the
    real tree, and the dispatcher opens the review *before* it (ADR 0009 §4)
    precisely because it will see the disk — but what the MCP tool itself
    writes is never staged and never reviewed. Since ADR 0011 it can write
    only inside the workspace, which bounds the hole without closing it.
    Until an MCP tool can be declared edit-shaped (which argument is the
    path, which the content), the claim is worded narrowly: *Aldwin's `edit`
    tool never writes without a review you approved.*

14. ~~**A refused read is reported without naming the path.**~~ **Moot,
    2026-09-24 (ADR 0011).** There is no read declaration to refuse. A
    write outside the workspace fails with the program's own permission
    error, which is the program's to word — most name the path.

15. ~~**Reads can only be enforced on Linux.**~~ Done (ADR 0007).

16. ~~**Landlock's network control covers TCP only.**~~ **Moot, 2026-09-24
    (ADR 0011).** The sandbox no longer restricts the network at all, and
    ADR 0011 states that as a non-goal rather than a gap.

17. ~~**A long command elides sooner than it used to.**~~ Moot: there is no
    permission panel for it to elide in. A long `run` target elides in the
    work disclosure's row instead, with the fact kept whole.

24. **Thinking is carried but never drawn.** ADR 0006 puts extended-thinking
    blocks in the transcript and on the wire; the TUI renders none of it. The
    new design specifies no treatment for reasoning text either, so this is
    still a design decision before it is a `LogEntry` variant.

25. **The macOS sandbox backend has never run on macOS.** Unchanged, and
    it now carries every process rather than read-declared ones (ADR 0011).
    Without `/usr/bin/sandbox-exec` the session says once that commands can
    write outside the workspace; a profile `sandbox-exec` refuses to load
    fails each command with its own message, which is loud rather than
    silent but is not yet a sentence of ours.

26. ~~**`bash -c` is still a hole in argument containment.**~~ **Done,
    2026-09-24 (ADR 0011).** There is no argument containment to have a hole
    in: `run` *is* `sh -c`, and the sandbox contains what a command does
    rather than reading what it says. `deny:` is gone with it.

33. **A command cannot write a package manager's store outside the
    workspace.** ADR 0011's incidental list is the devices, the temp
    directories and `~/.cache`, so `cargo` fetching a new dependency into
    `~/.cargo`, `npm install`/`npx` into `~/.npm`, and `go` into `~/go` fail
    with a permission error; building with dependencies already fetched
    works (`cargo check` of this repository, checked under the sandbox on
    2026-09-24). An `npx`-launched MCP server that has not been cached yet
    fails to start the same way. Closing it is a decision, not a fix: which
    stores join the incidental list — each is a directory anything a command
    writes persists in — or a `writable:` key beside `roots:` that widens
    writes without widening what the tools may be pointed at.

## References

- .claude/adr/0009-the-review-is-the-only-gate.md — the decision most of the 2026-09-23 changes belong to.
- .claude/spec/aldwin-review.md — the loop, and what it replaced.
- .claude/skills/review/SKILL.md — the loop as run, including stage 5's prompt.
- crates/review/baseline.json — the design contradictions entries 1, 27 and 29 are about.
- .claude/design/IMPORT.md — the reference, and how to re-sync it.
