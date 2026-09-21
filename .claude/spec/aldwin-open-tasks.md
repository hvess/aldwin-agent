# aldwin-open-tasks

Work that is known, understood and not done. Each entry says what was seen, where the evidence is, and what would close it.

**Status:** active — a ledger, not a spec. Nothing here blocks anything else.
**Scope:** everything outstanding as of 2026-09-21 (entries 24–26 added that day, with 14 half-closed and 15 closed). Entries 1–15 are gone:
they belonged to the screenshot harness and its conformance catalogue, both
deleted when the review loop replaced them — see
`.claude/spec/aldwin-review.md`'s Progress entry. New numbering starts at 1.
**Owner:** Maximilian
**Last Updated:** 2026-09-21

An entry leaves this file by being done, or by being decided against — in
which case the decision goes where it belongs (an ADR, or the spec it
contradicts) and the entry says so before it goes.

## Design

1. **The design reference contradicts itself, and the app compensates.**
   `crates/review/baseline.json` carries seven entries. Five are the design
   against itself; **two are the design against an ADR this project has
   already shipped**, which is a different thing and the reason to read them
   before touching anything visual:
   `permission-frame-predates-adr-0004` (frame `3a` draws the four-option
   `cargo *` list ADR 0004 deleted) and
   `access-scale-is-four-points-in-the-frame-and-three-in-the-readme` (frame
   `1c` restores an `all` rung reading "nothing asks", which ADR 0004 §3 makes
   impossible). Neither is a gap in the app. Closing them means fixing the
   design upstream and re-syncing — read the `design-sync` skill first — after
   which the entries are deleted.

   The 2026-09-21 repaint retired three older entries outright and shrank the
   glyph exception list from 18 marks to 11 (ADR 0002's box-drawing set alone),
   because the marks the old table omitted are simply in the new frames.

2a. **The design system upstream is still named Mjolnir.** The project was
   renamed Mjolnir → Aldwin on 2026-09-21; `.claude/design/` keeps the old
   name wherever it is an *address* — the project title on `claude.ai/design`,
   the bound copy's `_ds/mjolnir-design-system-4ea574fb-…/` path,
   `window.MjolnirDesignSystem_4ea574` — because renaming those locally would
   only stop `design-sync` finding anything. The frames themselves are
   rebranded: they write `Aldwin` in the top bar and `aldwin` as the speaker
   label, and the wordmark that used to spell the old name is gone from the
   design altogether, which retired the `wordmark-letters-are-the-old-name`
   contradiction. What is left is addresses, and closing it means renaming the
   project upstream and updating them here in the same pass.

2. **~~The design system ships no reference frames.~~ Done, 2026-09-21.**
   `Aldwin Agent TUI.dc.html` was fetched and rendered during the repaint
   import. The method is in `.claude/design/IMPORT.md`: extract one frame by
   `id`, wrap it in the local `tokens/*.css` plus stubs for the four web-only
   tokens (`--font-mono`, `--font-ui`, `--radius-frame`, `--shadow-lg`), and
   `firefox --headless --screenshot <abs> --window-size=1080,720 file://<abs>`.
   JetBrains Mono is installed, so the 9px advance is faithful and the render
   is measurable.

   It earned its keep immediately: reading `3a` suggested a tone change, and
   *rendering* it showed the frame had regressed to the pre-ADR-0004 permission
   model. The frames are not committed — they are large, and re-fetching is one
   `DesignSync` call.

   What is still owed is the automatic half: nothing compares a rendered design
   frame to a rendered app frame on a schedule. A cell-for-cell diff remains
   wrong (the frames hold different content); landmark positions are the
   comparable part, and the frames being token-authored now makes those
   readable from the markup without rendering at all.

21. **~~A panel dims the ink behind it but not the ground.~~ Moot,
    2026-09-21.** `fade_area` is deleted. The design replaced the whole
    treatment — "a recolour, never alpha" — so a scrimmed transcript is now
    ink remapped to the three `--tui-scrim-*` roles with the bands left alone,
    which is what the reference draws in `3a` and `3b`. The half of this entry
    that read as a defect (the wordmark staying the brightest field in the
    frame, above the panel meant to be the one live surface) went with the
    wordmark.

22. **Two designed screens have no implementation at all.** The repaint's
    frame file draws eleven screens; the app builds nine of them. The two
    missing are not regressions — they were never built — but the design now
    specifies them in full, so they are known, understood and undone:

    - **`3b` commands** — typing `/` lifts an 11-row panel off the composer:
      a title row carrying the filter and an `n of m` count, a 48-cell list
      (`--pane-commands-w`) on `--tui-recess`, and an explain pane beside it
      on `--tui-bar` showing the highlighted command's prose and two gauge
      rows. The app has slash commands (`cli::slash`) and a panel control
      that already draws this shape (`ui::picker`, `ui::decision`), so this
      is mostly wiring a third list into an existing one.
    - **`3c` review** — the only screen that takes the whole frame: a
      33-cell file pane (`--pane-files-w`) on `--tui-recess`, a hunk pane
      beside it, per-hunk `✓`/`▌`/`○` state, a `1 of 3 accepted` gauge, and a
      footer offering only the keys that apply. It needs state nothing
      tracks yet — a file list with per-hunk accept/reject — which is why
      this is the larger of the two by a wide margin.

    Both are drawn in `Aldwin Agent TUI.dc.html`; render them per entry 2
    before starting. Note `3c`'s diff rows use the **6-cell** gutter
    (`--gutter-line-no`, against the inline diff's 5) and hang their trailing
    note on `--diff-code-col` — the app's `diff.rs` is built around the
    inline geometry and would need both.

23. **The top bar draws two facts where the design draws four.** Every frame
    right-flushes `model · gauge · cost`; the app draws the model and the
    build version. A context gauge needs token accounting the session does
    not keep (`UsageStats` exists on a core event but nothing accumulates it
    into `StatusInfo`) and a cost needs per-model pricing, so neither is
    fabricated — `chrome::draw_top_bar` says so in place. The gauge is the
    nearer of the two: the roles are already carried (`gauge_fill`,
    `gauge_fill_hot` at 80%, `gauge_track`) and `1d` shows the empty state,
    so what is owed is the accumulation, not the drawing.

## Review loop

3. **`cargo fmt` is not in stage 1.** The codebase's aligned struct fields and
   grouped imports need `struct_field_align_threshold` and `group_imports`,
   both nightly-only, and the workspace pins no nightly. Under stable rustfmt
   the check wants to reformat 500 files and flatten the alignment. Revisit
   when either option stabilises or the project pins a nightly.

4. **No scene reaches two pending prompts.** `prompt_scoped` issues two tool
   calls and draws two `◐` rows, but the second is logged as running before
   its prompt is queued, so `decision::queue_note`'s `(+N more pending)` row
   renders in no frame the loop has ever captured.

5. **Every scene reaches its fake provider through a bare `base_url`**, so
   `current_provider` is `None` and any row that would name the vendor falls
   back to the model alone. A judge reads that as the app dropping the
   vendor. Closing it means a scene whose provider is a catalogue entry.

6. **Scenes exercise the OpenAI adapter only.** `base_url` is ignored for the
   `anthropic` provider (`llm/src/client.rs:64`), so a defect living only in
   the Anthropic client is invisible to every review.

7. **Key encoding is untested by construction.** The harness chooses the bytes
   a key sends, so it cannot vouch that they are what foot would send. If a
   panel ever binds a non-digit, non-arrow key, suspect this first; the input
   path wants `wtype` before it is trusted.

8. **The loop has never iterated.** No session has gone review → fix →
   re-review → rescore, so the five-iteration cap and stage 5's re-run are
   untested by use.

## History (ADR 0005)

18. ~~**No review scene reaches the `/resume` picker.**~~ **Done 2026-09-20.**
    `scene.rs` gained a `resume` scene: `Script::history` seeds past sessions
    through the product's own `HistoryStore` (same rule as the global config —
    a copy of the JSONL format here would drift), and the scene types
    `/resume`. Two sessions with different turn counts, so the list is a list
    and both `1 turn` and `4 turns` are exercised. It earned its keep
    immediately — stage 5 scored 75 on the first pass, against a panel every
    unit test was happy with. One wrinkle: the row's date is rendered in local
    time, so the frame is stable per machine but not across timezones.

19. **There is no opt-out, and nothing prunes.** Both are deliberate V1 gaps
    named in ADR 0005's Consequences, recorded here so they are found by
    someone looking for work rather than by someone surprised. Transcripts
    accumulate under `~/.aldwin/history/` at mode `0600` until the developer
    deletes them. The opt-out is the more pressing of the two: a developer
    working in a tree whose tool results carry secrets currently has no way to
    say "not this project" short of not running Aldwin in it.

20. **An MCP tool's results land in the transcript with no classification.**
    Consequence of entry 12 rather than of ADR 0005, but history is what gives
    it a disk lifetime: every MCP call is a `Class::Write` whose result is
    written to a transcript like any other. Whatever entry 12 settles about
    classifying MCP tools should say whether a class also decides what is
    recorded.

## Repo

9. ~~**`aldwin-tools`' LSP test needs `rust-analyzer` on PATH and is not
   gated for it.**~~ **Done 2026-09-20.** `#[ignore]`d with a reason, the
   same convention `llm/tests/live_lumo.rs` uses for its live-API tests, so
   the review loop's stage 2 no longer reports a missing dependency as a
   broken workspace. Run it with `cargo test -p aldwin-tools -- --ignored`.

10. ~~**`target/` is tracked — 402 files — and the repo had no `.gitignore`
    until 2026-09-19.**~~ **Done.** The rule is `/target`, which covers the
    whole directory rather than the one subdirectory the first version
    named, and `git ls-files target` now returns nothing. Build output no
    longer shows up in `git status`.

11. **No CI runs the test suite.** `.github/workflows/release.yml` builds on a
    tag and is the only workflow. Note that the screenshot harness cannot join
    it without a compositor in the runner — but `cargo test` and `cargo clippy`
    could, and the workspace is currently clippy-clean, which is the cheap
    moment to start enforcing it.

## Permissions (ADR 0004)

12. **An MCP tool cannot be classified by the developer, so every one is a
    write.** `mcp::tool`'s `permission` hard-codes `Class::Write` regardless of
    what the server advertises, because an MCP call runs inside the server's
    process where the sandbox cannot hold a read declaration to its word.
    ADR 0004 §4 intends the developer to classify each tool at first
    encounter, with the server's own claim shown as a claim. Closing it needs
    a place to persist that classification (a new config domain, or a third
    list in `permissions.yaml`) and a prompt shape that asks the question once
    rather than per call.

13. **An MCP tool that edits files does so without a diff.** Consequence of
    12, and the one hole in the edit guarantee. `Class::Edit` exists and
    `edit` holds it; an MCP tool could too, but only once the developer has
    said which of its arguments is the path and which is the new content —
    without that mapping there is nothing to render a diff from. Until it is
    built, the claim is worded narrowly and deliberately: *Aldwin's `edit`
    tool never lands without a diff you accepted.* The design system's readme
    still says "no write lands without a diff the user has accepted", which
    overstates it and wants rewording upstream (see entry 1 — it goes in the
    same re-sync).

14. **A refused read is reported as a refusal, not as a named write.**
    *Half closed 2026-09-21 (ADR 0007 §8): the **false positive** half is
    gone.* `ReadRefused` used to be raised on any non-zero exit under a read
    declaration, so `grep`'s "nothing matched" (exit 1, silent) asked the
    developer to allow a write that had never been attempted — and in the
    observed session that is what taught the model to stop declaring reads
    at all. It now needs evidence: a permission/read-only message on stderr,
    or death by signal. What remains is the original entry, below.

    `sandbox` returns `ToolError::ReadRefused` when a read-declared call fails
    under the read-only ruleset, and the prompt says the call could not
    complete — it does not say *it tried to write `.git/config`*. The kernel
    hands the child an ordinary permission error; naming the path needs
    syscall interception (seccomp user-notification, or ptrace) on top of
    Landlock. Worth building for the message alone. **Not needed for the
    guarantee**, which comes from the write being impossible rather than from
    our seeing it — and a false positive here is bounded: a read that failed
    for an unrelated reason offers to re-run as a write, which then fails
    again with its own error in view.

15. ~~**Reads can only be enforced on Linux.**~~ **Done 2026-09-21** (ADR
    0007 §6–§7). Two separate defects were hiding here. The smaller: macOS
    now enforces, via Seatbelt through `sandbox-exec` — see `sandbox/macos.rs`
    for why it confines by rewriting the command line rather than acting in
    the forked child. The larger, and the one that was actually costing
    sessions: `ReadOnly::build` refusing produced a **flat error**, not the
    question ADR 0004 §4 specifies ("where a `read` grant cannot be honoured,
    every call asks"). On macOS that meant every read-declared call failed, so
    the model declared `read` twice, saw both fail, and spent the next 69
    calls declaring `ls`, `grep` and `cat` as writes. `SandboxUnavailable` now
    routes to the same prompt as `ReadRefused`. Windows still cannot offer the
    rung and now says so in those words.

16. **Landlock's network control covers TCP only.** UDP and unix sockets are
    outside it, so a read-declared call cannot open a TCP connection but could
    still send a UDP datagram. A network namespace would close it completely
    and was verified to work unprivileged on this machine
    (`unshare -Urn`); it was not taken in this pass because it needs uid-map
    plumbing in `pre_exec`, which is a larger and more failure-prone change
    than the two syscalls `engage` currently makes.

17. **A long command elides sooner than it used to.** The options list is
    eight rows where it was five, the design fixes the panel at
    `--panel-permission-h`, so a long argument's tail is the first thing to
    go — announced by the panel's own marker, never at the cost of an option
    row. Pinned by `a_long_permission_prompt_wraps_in_the_panel_instead_of_
    being_clipped`. If it bites in use, the fix is a scrollable command block
    rather than a taller panel, which the design's band height forbids.

24. **Thinking is carried but never drawn.** ADR 0006 puts extended-thinking
    blocks in the transcript and on the wire; the TUI renders none of it —
    `replay` drops the records and `ThinkingDelta` only keeps the existing
    indicator alive. That is deliberate rather than unfinished: a treatment
    for reasoning text is a design decision, the design system specifies
    none, and CLAUDE.md forbids inventing one locally. Closing it means a
    frame upstream (the `--tui-scrim-*` roles are the obvious candidate,
    since a scrimmed transcript is already ink remapped to them), a re-sync,
    and then a `LogEntry` variant. Until then a developer can see *that* the
    agent thought, never what it thought — which is a real gap in a harness
    whose product is understanding.

25. **The macOS sandbox backend has never run on macOS.** `sandbox/macos.rs`
    is ordinary Rust — a generated SBPL profile and a command-line rewrite,
    no FFI — so it compiles on every platform and its unit tests run
    everywhere (they assert rule order, which is last-match-wins and the easy
    thing to get backwards, and that an unquotable path is skipped rather than
    truncating the profile). None of that exercises `sandbox-exec` itself.
    One macOS-specific trap is already handled on reasoning alone and is the
    first thing to confirm: Seatbelt matches resolved paths, so the `/tmp` and
    `$TMPDIR` exemptions are emitted in canonical form as well.
    What is owed is the Linux backend's own test shape run on a Mac: a
    read-declared call that tries to write, asserting nothing landed. The
    failure mode meanwhile is benign — a profile that will not load makes
    `build` fail, which is entry 15's question, not an unconfined run.

26. **`bash -c` is still a hole in argument containment.** ADR 0007 §2
    contains path-like *arguments*, which is sound for every program whose
    argv means what it looks like. A path inside a string — `bash -c 'cd
    /elsewhere && …'` — is one argument that neither starts with `/` nor
    climbs, and is invisible to the check. (A value glued to a short flag,
    `-C/elsewhere`, *was* a second hole of the same kind; that one is closed.)
    A smaller asterisk of the same family: an absolute argument whose first
    component does not exist is taken for a pattern, not a path, so
    `mkdir -p /brand-new-top-level/x` is not refused — it needs write access
    to `/` to do anything. Granting a shell was already
    granting arbitrary execution (ADR 0004 §1), so this widens nothing that
    was previously closed; it is recorded because the *claim* now reads "every
    tool honours the workspace" and this is the asterisk. The real close is
    ADR 0004 §1's deferred work — pipelines as structured stages, each a
    program with its own grant — which removes the reason to grant a shell.

## References

- .claude/spec/aldwin-review.md — the loop most of these belong to, and what it replaced.
- .claude/skills/review/SKILL.md — the loop as run, including stage 5's prompt.
- crates/review/baseline.json — the design contradictions entry 1 is about.
- .claude/design/IMPORT.md — the reference, and how to re-sync it.
