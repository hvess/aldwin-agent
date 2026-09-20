# mjolnir-open-tasks

Work that is known, understood and not done. Each entry says what was seen, where the evidence is, and what would close it.

**Status:** active — a ledger, not a spec. Nothing here blocks anything else.
**Scope:** everything outstanding as of 2026-09-19, from the first screenshot
session, the harness's own disclosed gaps, two repo-level findings, and two
scene gaps found by `run-1789850385`'s judges (entries 14 and 15).
**Owner:** Maximilian
**Last Updated:** 2026-09-19

An entry leaves this file by being done, or by being decided against — in
which case the decision goes where it belongs (an ADR, or the spec it
contradicts) and the entry says so before it goes.

## Design conformance — superseded

**Superseded 2026-09-19 by `.claude/spec/mjolnir-design-conformance.md`.**
Entries 1–4 came from run `run-1789824458`, which scored two scenes. The
full-catalogue run `run-1789826989` scored all twelve at three sizes in both
themes — 72 frames, minimum 58 against 90 — and the conformance spec carries
each of these with its measurements across every scene, its class (is the app
wrong, or has the design no answer), and where in `crates/tui` it lives. They
are kept here as a pointer, not restated.

- Entry 1 → conformance spec, Class A item 6 (the uncapped measure). **Open**,
  waiting on a maximum-measure token that the design system does not have.
- Entry 2 → the status row's 2-cell separators. **Fixed 2026-09-19** and
  deleted from that catalogue, which keeps only open work; the pass that
  built it is in its Progress entries.
- Entry 3 → the orphan break band. **Fixed 2026-09-19**, same.
- Entry 4 → Class A item 22 (the missing time row). **Open**, and reclassified
  while it was attempted: there is no clock anywhere in the workspace, so it
  is a data source that does not exist rather than a layout defect.

The original text follows, unchanged.

1. **The measure is uncapped at wide terminals.** `conversation` at 200×50
   scored 74 spatial / 76 component — the only frames under the threshold.
   Assistant prose ran from column 13 to column 195 as a single 183-cell line,
   against the design reference's 104-cell body, leaving the following row a
   45-cell stub. A paragraph reads as a ribbon across the frame. This is what
   the "large = restraint" rubric exists to catch, and nothing else in those
   frames was wrong: no clipping, no colour outside the palette, every anchor
   held.

2. **The status row parts groups by 2 cells where the handoff specifies 6.**
   Seen at medium and large in both scenes (row 34 at 120×36). `--group-gap`
   is 6 cells and parts *unrelated* facts; facts within a group ride the
   tighter ` · ` rhythm. Worth checking which the status row's fields are
   before moving anything — `ui/grid.rs`'s `GROUP_GAP` comment is explicit
   that the distinction has been misread before.

3. **An orphan break band at 80×24.** In `markdown` at small, the user's turn
   has scrolled out of the viewport but its break band remains at row 5, with
   empty ground above it at rows 3–4. It reads as a second bar under the top
   bar. A break that separates nothing should not survive the thing it was
   separating.

4. **No time row under the speaker label.** The judge noted the label column
   carries only the speaker where the session reference shows a time beneath
   it. Possibly deliberate and never built; decide which, then either build it
   or record that the reference is not being followed there and why.

## Harness gaps — disclosed, not hidden

Entries 5–10 are in `.claude/skills/screenshot/SKILL.md` under "What this does
not cover", so a session reports them rather than implying coverage. Listed
here because each is closable work.

Entries 14 and 15 are new on 2026-09-19 and are **not** disclosed in the
skill — both are scenes that cannot exercise what they render, found by
`run-1789850385`'s judges reading the frames as evidence about the app. 14 is
the sharper of the two, because the skill currently claims the coverage it
lacks. They take the next free numbers rather than slotting in above the Repo
entries; numbers here are never reused.

5. **`breakages` cannot see a collision inside the body column, or text
   truncated with a well-formed ellipsis.** The `layout` gate catches content
   pushed into the 3-cell margin, which is the *overflow* symptom, but two runs
   colliding mid-row is invisible. This is the defect class the UI actually
   keeps producing (`6c1ab32`, `1f125d4`), so a clean `breakages` is weaker
   evidence than it looks.

6. **`role pairing` does not check that a label is `--tui-label`.** It catches
   a band painted in an ink role and a glyph painted in a ground rung, and
   stops there, because the design does not enumerate which ink belongs on
   which band. Closing this means getting that mapping from the design system
   rather than inventing it in the harness.

7. **The spatial reference at 120×36 is decided but not built.** The plan is
   token-derived rules on every frame *plus* the rendered handoff frame where
   the design drew that scene — the handoff being the one reference that does
   not come from the app's own output. Today only the tokens half exists, so
   the check is near-circular: `ui/grid.rs` derives its constants from the same
   `cells.css`. Needs a `DesignSync` fetch (the `.dc.html` frames are not in
   `.claude/design/`) and a browser render; measure its pixels against its own
   9×20 cell, never read positions off the prose.

8. ~~**Tab does not take effect through injected input.**~~ **Retired
   2026-09-19 without being answered, because the feature it was about is
   gone.** ADR 0003 moved grant scope onto the option rows and unbound `Tab`
   in the permission panel, so there is no longer a widening keypress to test
   and `prompt_scoped` no longer sends one.

   **What did not go away is the underlying question**, and it is worth
   restating because the evidence for it is now gone from the scenes: the
   harness chooses the bytes a key sends, so it cannot vouch that they are
   what foot would send. Digits and arrows demonstrably worked and `Tab` did
   not, which was either a real defect under a terminal or exactly that blind
   spot. Nothing about key encoding is tested here by construction. If a
   future panel binds a non-digit, non-arrow key, this is the first thing to
   suspect, and the input path wants `wtype` (compositor-level key events,
   one package) before it is trusted.

9. **Scenes exercise the OpenAI adapter only.** `base_url` is ignored for the
   `anthropic` provider (`llm/src/client.rs:64`), and pointing the app at a
   local fake is nothing but a `base_url`. A defect living only in the
   Anthropic client is invisible to every screenshot session.

10. **The loop has never iterated.** Six sessions have run: capture → gates →
    judge → report. No session has gone capture → fix → recapture → rescore,
    so the iteration cap, the per-iteration notes and the fix half of the loop
    are untested by use.

    **Cheaper than it was, as of 2026-09-20.** An iteration pass no longer
    needs a judge at all: `mjolnir-screenshot conformance` is the fix
    loop's feedback, it runs in the 2m31s the capture takes, and
    `run --quiet-ms 150 --theme dark` roughly quarters that again for a pass
    that is not about colour. `crates/tui` now has eleven cited Class A items
    queued (conformance Step 4), which is the first time there has been
    enough work to make an iterating session worth opening.

14. **No scene ever has two prompts pending at once, so
    `decision::queue_note` is unreachable.** `prompt_scoped` issues two tool
    calls in one assistant message and draws two `◐` rows, but its panel is
    **byte-identical** to `prompt_path`'s and `(+N more pending)` appears in
    none of the 72 frames of `run-1789850385`: the second call is logged as
    running before its prompt is queued, so `pending_prompts.len()` is 1 when
    the panel draws. The screenshot skill's "What this does not cover" says
    that scene "covers a *queued* second prompt" — it does not, and the
    disclosure should either be corrected or the scene should press the case
    it claims. Closing it means a scene that reaches two pending prompts,
    which is also what would let a judge see the design question in
    `mjolnir-design-conformance` Class B 9.

15. **`empty`'s `provider` row cannot exercise what it renders.**
    `14d` specifies the fact as `anthropic · sonnet-4.6` — vendor and model —
    and `transcript::intro_content` draws exactly that whenever the provider
    is known. Every scene reaches its fake through a bare `base_url`, so
    `current_provider` is `None` and the row falls back to the model alone. A
    blind judge on `run-1789850385` read that as the app dropping the vendor,
    which is the failure mode this ledger's entry 9 describes from the other
    side: the scenes exercise one configuration and the frames are then read
    as evidence about all of them. Closing it means a scene whose provider is
    a catalogue entry rather than an endpoint.

## Repo

11. **`mjolnir-tools`' LSP test needs `rust-analyzer` on PATH and is not gated
    for it.** `lsp::client::tests::spawns_and_initializes_a_real_language_server`
    fails a clean checkout on a machine without it — confirmed to predate the
    2026-09-19 work. This crate already has the convention for tests that need
    something the build does not provide: `llm/tests/live_lumo.rs` is
    `#[ignore]`d behind an env var. Either gate it the same way, or make
    `rust-analyzer` a documented prerequisite.

12. **`target/` is tracked — 402 files — and the repo had no `.gitignore`
    until 2026-09-19.** The new file ignores `target/screenshot-runs/` only,
    because an ignore rule does nothing about what is already tracked.
    Untracking the rest is a separate decision with a noisy diff; nothing
    depends on it, and build churn shows up in `git status` until it happens.

13. **No CI runs the test suite.** `.github/workflows/release.yml` builds on a
    tag and is the only workflow. Note that the screenshot harness cannot join
    it without a compositor in the runner — but `cargo test` and `cargo clippy`
    could, and the workspace is currently clippy-clean, which is the cheap
    moment to start enforcing it.

## References

- .claude/spec/mjolnir-design-conformance.md — the design gap in full; supersedes entries 1–4.
- .claude/spec/mjolnir-screenshot.md — the harness these gaps belong to.
- .claude/skills/screenshot/SKILL.md — what a session must disclose.
- .claude/design/HANDOFF.md — the reference the conformance entries are measured against.
