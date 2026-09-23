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

30. **The screenshot harness cannot press `⇧↓` or `⌃↩` except as literal
    Kitty sequences.** `keys.rs` names arrows, Enter, Tab, Esc, Space and
    Backspace; the review's selection and approve keys go through the
    protocol the app negotiates with the terminal. `scene.rs` sends `⌃↩` as
    the raw `\e[13;5u` for the `saved` scene, which works because the app
    pushes the Kitty flag at startup, and does not attempt `⇧↓` at all — so
    `selecting` and `commented` exist only as `render_snapshot.rs` scenes.
    Entry 7's `wtype` is the honest fix.

31. **`working` and `running` are transient and the harness cannot hold
    them.** The fake provider answers at once, so the amber `● Working…` is
    never on screen when a frame is taken. `Canned::SseThenStall` would
    hold a turn open mid-reply; a scene built on it would capture the
    working footer with a half-streamed sentence, which is a real state.
    Not done because the capture's quiet-window rule would then wait forever
    — it needs a per-scene override.

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
    Every MCP call is a `Class::Write` whose result is written to a
    transcript like any other. See entry 13.

## Repo

9. ~~**`aldwin-tools`' LSP test needs `rust-analyzer` on PATH.**~~ Done.

10. ~~**`target/` is tracked.**~~ Done.

11. **No CI runs the test suite.** `.github/workflows/release.yml` builds on a
    tag and is the only workflow. `cargo test` and `cargo clippy` could join
    it; the workspace is clippy-clean.

## Permissions and the review (ADR 0009)

12. ~~**An MCP tool cannot be classified by the developer, so every one is a
    write.**~~ **Moot, 2026-09-23.** Nothing asks, so nothing needs a class
    to decide whether to ask. The class still matters to the sandbox, which
    an MCP call never enters; see 13.

13. **An MCP tool that edits files does so outside the review.** The one
    hole in the edit guarantee, and wider than it was: an MCP call runs in
    its own process over the real tree, and the dispatcher opens the review
    *before* it (ADR 0009 §4) precisely because it will see the disk — but
    what the MCP tool itself writes is never staged and never reviewed.
    Until an MCP tool can be declared edit-shaped (which argument is the
    path, which the content), the claim is worded narrowly: *Aldwin's `edit`
    tool never writes without a review you approved.*

14. **A refused read is reported without naming the path.** `ReadRefused`
    tells the model the call tried to write or connect; it does not say
    *which path*, because the kernel hands the child an ordinary permission
    error. Naming it needs syscall interception on top of Landlock. Worth
    building for the message alone; not needed for the guarantee.

15. ~~**Reads can only be enforced on Linux.**~~ Done (ADR 0007).

16. **Landlock's network control covers TCP only.** UDP and unix sockets are
    outside it. A network namespace would close it completely; not taken
    because it needs uid-map plumbing in `pre_exec`.

17. ~~**A long command elides sooner than it used to.**~~ Moot: there is no
    permission panel for it to elide in. A long `run` target elides in the
    work disclosure's row instead, with the fact kept whole.

24. **Thinking is carried but never drawn.** ADR 0006 puts extended-thinking
    blocks in the transcript and on the wire; the TUI renders none of it. The
    new design specifies no treatment for reasoning text either, so this is
    still a design decision before it is a `LogEntry` variant.

25. **The macOS sandbox backend has never run on macOS.** Unchanged. The
    failure mode is now ADR 0009 §3's: a profile that will not load makes
    `build` fail, the call runs unconfined, and the developer is told once.

26. **`bash -c` is still a hole in argument containment.** Unchanged, and
    worth restating under ADR 0009: with no grant to make, running `bash` is
    no longer "a deliberate act" — it is a call like any other, held only by
    the sandbox when declared a read and by nothing when declared a write. A
    `deny: [bash, sh]` in the global file is the developer's lever, and the
    annotated template could suggest it.

## References

- .claude/adr/0009-the-review-is-the-only-gate.md — the decision most of the 2026-09-23 changes belong to.
- .claude/spec/aldwin-review.md — the loop, and what it replaced.
- .claude/skills/review/SKILL.md — the loop as run, including stage 5's prompt.
- crates/review/baseline.json — the design contradictions entries 1, 27 and 29 are about.
- .claude/design/IMPORT.md — the reference, and how to re-sync it.
