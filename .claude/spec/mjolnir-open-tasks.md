# mjolnir-open-tasks

Work that is known, understood and not done. Each entry says what was seen, where the evidence is, and what would close it.

**Status:** active — a ledger, not a spec. Nothing here blocks anything else.
**Scope:** everything outstanding as of 2026-09-19, from the first screenshot
session, the harness's own disclosed gaps, and two repo-level findings.
**Owner:** Maximilian
**Last Updated:** 2026-09-19

An entry leaves this file by being done, or by being decided against — in
which case the decision goes where it belongs (an ADR, or the spec it
contradicts) and the entry says so before it goes.

## Design conformance — from the first screenshot session

Run `run-1789824458`, scoring `markdown` and `conversation` at all three sizes
in both themes. Verdict: **below threshold, minimum 74 against 90**. Gates were
clean on all twelve frames; every deduction below is a judgement a
deterministic gate cannot make. The run directory ages out after five more
sessions, so the numbers are recorded here rather than by reference.

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

These are in `.claude/skills/screenshot/SKILL.md` under "What this does not
cover", so a session reports them rather than implying coverage. Listed here
because each is closable work.

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

8. **Tab does not take effect through injected input, and nobody knows why
   yet.** On the permission panel digits resolve and arrows move, but Tab —
   which should widen the grant to the directory — does nothing, in either the
   legacy `\t` or the disambiguated `CSI 9 u` encoding. `app.rs:1236` reads
   correctly and the hint is rendered, so `decision_grant().alternate` is
   `Some`. Two possibilities: a real defect in Tab handling under a terminal,
   or the harness's own blind spot — it chooses the bytes, so it cannot vouch
   that they are what foot would send. **The cheap test is a human pressing Tab
   in a real session.** If it works there, the input path wants `wtype`
   (compositor-level key events, one package) and `prompt_scoped` stops being
   reshaped.

9. **Scenes exercise the OpenAI adapter only.** `base_url` is ignored for the
   `anthropic` provider (`llm/src/client.rs:64`), and pointing the app at a
   local fake is nothing but a `base_url`. A defect living only in the
   Anthropic client is invisible to every screenshot session.

10. **The loop has never iterated.** One session has run: capture → gates →
    judge → report. No session has gone capture → fix → recapture → rescore,
    so the iteration cap, the per-iteration notes and the fix half of the loop
    are untested by use.

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

- .claude/spec/mjolnir-screenshot.md — the harness these gaps belong to.
- .claude/skills/screenshot/SKILL.md — what a session must disclose.
- .claude/design/HANDOFF.md — the reference the conformance entries are measured against.
