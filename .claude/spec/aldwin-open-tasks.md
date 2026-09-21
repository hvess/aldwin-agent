# aldwin-open-tasks

Work that is known, understood and not done. Each entry says what was seen, where the evidence is, and what would close it.

**Status:** active — a ledger, not a spec. Nothing here blocks anything else.
**Scope:** everything outstanding as of 2026-09-20. Entries 1–15 are gone:
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
   `crates/review/baseline.json` carries two entries: the closed glyph table
   against the design's own copy, and the absent table component ADR 0002
   works around. Both are bugs in `.claude/design/`, not in the app. Closing
   them means fixing the design upstream and re-syncing — read the
   `design-sync` skill first — after which the entries are deleted.

2a. **The design system upstream is still named Mjolnir, and its frames
   still spell that wordmark.** The project was renamed Mjolnir → Aldwin on
   2026-09-21; `.claude/design/` keeps the old name wherever it is an
   *address* — the project title on `claude.ai/design`, the bound copy's
   `_ds/mjolnir-design-system-4ea574fb-…/` path, `window.MjolnirDesignSystem_4ea574`
   — because renaming those locally would only stop `design-sync` finding
   anything. The consequence that reaches the app is the wordmark: the
   frames render seven letters in a 17-cell field, the app renders six in
   15, and `baseline.json`'s `wordmark-letters-are-the-old-name` records the
   split. Closing it means renaming the project upstream and re-syncing,
   after which that contradiction entry is deleted and the addresses here
   are updated in the same pass.

2. **The design system ships no reference frames.** `.claude/design/` carries
   prose and tokens; the rendered `.dc.html` frames the design was drawn as
   are not imported. Without them, nothing mechanical can check whether a
   band is in the *right place* — stage 3 checks tokens and cells, and
   everything positional falls to stage 5's judgement.

   Feasible here and needs nothing installed: there is no Chromium and no
   snap, but `firefox --headless --screenshot <abs path>
   --window-size=1080,720 file://<abs path>` writes the file. What is owed is
   the fetch, which is a deliberate `DesignSync` call, and a decision about
   what to do with them — a cell-for-cell diff is wrong, because the design's
   frames hold different content; landmark positions are the comparable part.

21. **A panel dims the ink behind it but not the ground.** `ui/mod.rs:296`'s
    `fade_area` composites `cell.fg` against `cell.bg` and leaves `cell.bg`
    alone, so an overlay panel recedes the *text* behind it and none of the
    bands. `HANDOFF.md:280` and `:353` say "the transcript behind dims to
    ~35%", which is the whole surface. Shared by the permission panel and both
    pickers, so it is not any one screen's defect — but it is most visible
    where a panel opens over the resting screen, since the wordmark's
    `--tui-reverse-bg` then stays the brightest field in the frame, above the
    panel that is supposed to be the one live surface. Raised by a stage 5
    judge on the session list, 2026-09-20; reachable identically through bare
    `/model`. Fixing it means fading `cell.bg` toward the ground in the same
    pass, and deciding what that does to the wordmark specifically.

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

15. **Reads can only be enforced on Linux.** `sandbox::availability` reports
    `Unavailable` everywhere else, and `ReadOnly::build` refuses, so a
    read-declared call is refused rather than run unconfined — correct, and
    a worse product on macOS and Windows. macOS has an equivalent primitive
    worth wiring up; Windows effectively does not, and there a `read` rung
    cannot honestly be offered at all. The fallback is currently the same in
    both cases and should probably differ.

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

## References

- .claude/spec/aldwin-review.md — the loop most of these belong to, and what it replaced.
- .claude/skills/review/SKILL.md — the loop as run, including stage 5's prompt.
- crates/review/baseline.json — the design contradictions entry 1 is about.
- .claude/design/IMPORT.md — the reference, and how to re-sync it.
