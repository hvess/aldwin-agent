# mjolnir-screenshot

Screenshot harness — runs the shipped binary in a real terminal, offscreen, at three fixed sizes, and scores the frames against the design system.

**Status:** active — built and in use, with the gaps in the 2026-09-19
"session" entry below. Originally: The capture stack is
proven by probe (see Progress), the acceptance model is decided below, and
every Step has an answer; no open questions remain.
**Scope:** screenshotting and scoring `crates/tui` as rendered by the shipped
`mjolnir` binary. Two deliverables: the crate `crates/screenshot`
(`mjolnir-screenshot`), which captures, gates and scores, and a skill that
drives it — declaring the goal, focus set and scenes, running the loop, and
asking before it deletes a run. Excludes cell-level assertions (`crates/tui/tests/render_snapshot.rs`,
`ui/tests.rs`), the design system itself, and functional testing.
**Owner:** Maximilian
**Last Updated:** 2026-09-19

**Progress (2026-09-19, capture probe):** The stack was built and run end to
end on this machine with no new packages — a headless `sway`
(`WLR_BACKENDS=headless`) running `foot`, captured by `grim` launched inside
that session. The real `target/debug/mjolnir` first-run screen was captured at
exactly 120×36 cells (960×684 px) and driven to the model step by injected
input. Four measurements from the probe are recorded as Pitfalls: the cell
size, the pty resize race, the `grim` display trap, and the compositor's own
error bar. A first pass used `tmux` for sizing and input and is now excluded —
the reason is itself a finding, see Decisions.

**Progress (2026-09-19, three-pass audit):** A consistency, accuracy and
buildability pass over this spec found eleven defects and fixed all of them
before any code was written. Three were structural. The regression gate
described two different mechanisms in two places, one of which silently
required building and capturing the merge-base revision; it is now the
focus-scoped `render.snap` diff. The colour and role-pairing gates were
specified as pixel checks, which antialiasing makes unbuildable and which
carry no notion of a region's role; every gate now reads the cells the app
declares, through the proxy. And nothing pinned non-determinism, though the
regression gate demanded exact equality — hence the quiesce rule and masking.
The rest were smaller: frame counts that assumed a single scene, per-capture
checks living inside a once-per-run preflight, a judgement gate defined as
deterministic, steps ordered before their dependencies, and the scene port
described as a rename when it is real work.

**Progress (2026-09-19, three-pass audit of the code):** clippy (one
deny-level error, 14 warnings) plus an independent review pass found fourteen
defects in `crates/screenshot`; all are fixed, and the crate is clippy-clean
with 21 tests. The rust-skills the other specs' audits used were not installed
in that session, so this was clippy plus a structured read rather than
`m01`–`m15`; that is a weaker instrument and the difference is worth knowing.

Four of the findings were the same *kind* of bug, which is the useful pattern:
**a value collision defeating a lookup.**

* `--tui-bar` and `--tui-line` share `#474251`, so requiring *every* alias of a
  colour to be a ground made `is_only_ground` fail open — a glyph painted in
  the top-bar ground could never be reported, one of the two defects the
  role-pairing gate exists for. It now names the non-ink surfaces explicitly,
  which also keeps the `--tui-scrim`/`--tui-reverse-ink` case it was written
  for.
* The region map picked a band's role alphabetically, labelling the dark scrim
  `reverse-ink`; since `role_pairing` tests that label against the ink ramp, a
  luckier alias would have reported a legitimate band as ink.
* `light = dark.clone()` meant a `.tui-light` role that failed to resolve left
  the *dark* value in the light map — every light-theme result computed
  against the wrong theme, silently. A declared light role whose palette step
  does not exist is now an error; one whose step is an `rgba()` (the diff
  backgrounds are translucent by design) is skipped deliberately.
* The content gate reported a *byte offset* as a grid column. `▌`, `·` and `…`
  are three bytes each and in every frame, so the magenta annotation was drawn
  on a cell that had not failed.

Two were panics waiting on malformed input — `CSI 65535 B` overflowed a `u16`,
and VPA/CHA indexed the grid unclamped — both on the pump thread, where a
panic poisons the parser's lock and takes the capture down behind a misleading
message. The parser observes an app that may be misbehaving; it does not get
to assume otherwise.

And one was a gate that could not fail: **the regression result never reached
the verdict.** `run` printed it and returned zero, and `verdict` computed the
exit condition from per-frame gates and scores alone, so a session that moved
snapshot regions outside its focus set still passed — disarming the one check
standing between the loop and chasing its score by editing something it was
not asked to touch. The outcome is now recorded in `session.json`, the verdict
fails without it, and the report has a section for it.

**Progress (2026-09-19, session, region map and report):** Steps 8–11 are
built, so the harness runs a session rather than a sequence of commands:
`start` writes the contract, `preflight` clears or ends the run, `run`
captures and gates every scene the session names, `verdict` computes the exit
condition, and `report` assembles one self-contained HTML file with the frames
embedded and each violation outlined in magenta on a copy.

The **region map** is derived, not declared: bands come from runs of rows
sharing a ground — Turn 13's rule that a band's identity *is* its step on the
ladder makes that sound — and columns from the grid's own arithmetic. It
unblocked two gates. `role pairing` is deliberately weak (a band painted in an
ink role, a glyph painted in a ground rung) because the design does not
enumerate which ink belongs on which band; anything more would be invented
here rather than imported. `layout` is the useful half of the missing
breakages work: nothing but a band's ground belongs in the 3-cell margin, and
content in the margin is the visible symptom of the clipping this UI keeps
producing. Both write the map beside the frame, because a map inferred from
the rules the app is meant to follow will mislabel a band drawn in the wrong
place rather than notice it.

Three defects the build produced, all caught by the harness's own checks:

* **`--tui-reverse-ink` and `--tui-scrim` are the same value**, so "is this
  colour a ground" flagged the wordmark. A colour is only a ground when *every*
  role it resolves to is one.
* **A diff row is a band that is not a ground rung** — `--tui-del-row` is a
  field by design — so the band check narrowed to the ink ramp alone.
* **A relative `--run` path made every frame the same wrong screen.** `HOME`
  is resolved by the app against its own working directory, so a relative one
  sent it looking for `~/.mjolnir` inside the project; it found nothing, opened
  first run, and captured that in the wrong theme twelve times over. It looked
  entirely plausible. Seeded paths are canonicalised now.

**Progress (2026-09-19, the whole catalogue):** All twelve scenes run,
including the six tool-driven ones. What a scene reaches is decided by the
**grants it seeds**, not by faking a panel: an allowed tool runs, an unallowed
one asks, and an `edit` always asks with a diff because editing is outside the
permissions model entirely (ADR 0001). That is the property that makes these
scenes worth more than the snapshot's — they arrive at a decision surface the
way a developer does.

Two findings came out of it.

**The colour gate had to learn what dimming is.** `prompt` reported 33 colour
violations and `approval` 53, and every one was correct rendering: the
transcript dims behind an open decision panel, and a blend is by construction
not a palette value. Each flagged colour measured as an *exact* 45% blend of a
role toward `--tui-ground`. The gate now reconstructs the blend — a colour
passes if it is a role, or a role dimmed toward a ground rung — and reports
which role was dimmed rather than staying silent about it. A tolerance would
have been the wrong fix; solving for the blend keeps "dimmed" and
"off-palette" distinguishable.

**`preflight` does not check that the binary is current.** It verifies that
`target/debug/mjolnir` exists, that `render_snapshot` is green and that the
measured cell matches the baseline — not that the binary is newer than the
sources it was built from. On 2026-09-19 a run captured an hour-old binary,
reported "preflight clear", and passed all six gates on frames showing the
*previous* build's panel. Nothing in the harness noticed, and nothing could
have: every gate reads the frames, and the frames were internally consistent.

This is the same failure shape as the stale-snapshot check `preflight`
already guards against — a gate comparing against fiction and reporting
clean — and it wants the same treatment: compare the binary's mtime against
the newest file under `crates/`, and fail the session rather than warn. Until
then: build before every run, and read the first captured frame before
trusting any of them.

**Key encoding is unverified by construction, and `Tab` was how we found
out.** The harness chooses the bytes a key sends, so it cannot vouch that they
are what foot would send. On the permission panel digits resolved and arrows
moved; `Tab` never did, in either the legacy `\t` or the disambiguated
`CSI 9 u` encoding — either a real defect under a terminal or exactly that
blind spot, never distinguished.

ADR 0003 then unbound `Tab` in that panel, so the one scene that exercised it
(`prompt_scoped`) no longer sends it and the symptom is unreachable. **The
gap is not closed, only unobservable**: a clean run still says nothing about
key handling, and the first non-digit, non-arrow binding a panel takes will
need `wtype` (compositor-level key events, one package) before its frames
mean anything.

**Progress (2026-09-19, scenes):** Six of the twelve catalogue scenes run
through the real path — `first_run`, `empty`, `conversation`, `markdown`,
`fenced_diff`, `long`. A scene is now a seeded config, a queue of canned
provider replies and a key script; the binary plays it out over real HTTP,
through the real streaming adapter, core loop and TUI. The provider is
`mjolnir-llm`'s own `test_server`, exposed behind a dev-only `test-server`
feature so it never reaches the shipped binary — with the caveat that Cargo
unifies features, so `cargo build --workspace` does compile it into
`mjolnir-cli`; the release workflow builds `-p mjolnir-cli`, where it stays
off. Scenes are **openai-compatible** by force, not preference: `base_url` is
ignored for the anthropic provider (`client.rs:64`), and pointing the app at a
local fake is nothing but a base_url. A defect living only in the Anthropic
client is therefore invisible here.

The first scripted scene broke quiesce, and the fix is the better design.
Waiting for the *byte stream* to stop never succeeds: an idle app repaints on
every tick and ratatui emits the frame envelope — synchronised-update markers,
cursor hide/show, an SGR reset — even when no cell differs. Quiesce is now
measured on the **rendered state**: the mark moves only when the grid's
fingerprint changes. A scene that legitimately never settles (a running
spinner) is what the baseline file's `masks` are for.

The same scene also found a product defect, which is the first thing this
harness has caught that the 297 TUI tests do not: **`markdown.rs:419` treats
`_` as an italic delimiter with no word-boundary rule**, so
`ANTHROPIC_API_KEY` renders as `ANTHROPICAPIKEY`. CommonMark and GFM both
forbid intraword `_` emphasis precisely so `snake_case` survives. Unfixed; it
is a TUI bug, not a harness one.

**Progress (2026-09-19, five gates of six):** Step 9 is built except role
pairing. `colour`, three `breakages` checks (unpainted cells, glyphs outside
the closed table, a wide glyph clipped at the row's edge) and `content` read
the declared cells; `regression` diffs `render.snap` against the merge-base
and reports which sections moved outside the focus set. All six `first_run`
frames come back clean, which is itself the first evidence that the palette
parsing and the app agree: every declared colour resolves to a design role in
both themes.

The design system is **read, not restated** — `design.rs` parses
`tokens/palette.css` and `tokens/semantic.css` (resolving role → ramp step →
value, per theme) and the glyph table out of `HANDOFF.md`, so a re-sync moves
the gates with it. That immediately exposed a distinction the prose leaves
implicit: the closed table covers **marks**, while the design's own screen
descriptions use `·`, `…`, `⏎`, `↑↓` and `→` freely in key hints and
typography. Those are a cited exception in the baseline file rather than a
widened table, and every run prints which exceptions it leaned on.

`content` also caught itself. Its first version flagged the wordmark: `M J O
L N I R` is letter-spaced, so it contains a literal `"I "`. The bare pronoun
now needs a lowercase word after it, and the honest fix is noted in the code —
prose is a region, and a wordmark is not.

**Role pairing is the one gate still unbuilt, and it is blocked on the region
map**: a cell's *role* is not recoverable from its colour, and the map that
would say which cells are labels, body or chrome is the same structure the
focus set needs. It is the next piece of work.

**Progress (2026-09-19, the proxy):** Step 5 is built, and it replaced two
guesses rather than only adding input. Cell measurement now reads the pty foot
sizes instead of running `stty` in a shell; capture waits for the app to go
quiet instead of a settle interval; and the app is started on a pty *already*
at the target size, so foot's 80×24-then-resize race cannot reach it at all.
The parser is hand-rolled (`vt.rs`, 8 tests) because `vte` is not available
offline, and it is checked against the picture on every capture
(`proxy::verify_against_pixels`): a cell it calls "space on ground X" must be
a flat block of X in the PNG, or the run is refused. That check earned itself
immediately. The first keyed capture failed with "parser and frame disagree at
cell (13,29)", which was not a parser bug — `wait_quiet` returns instantly for
an app that has been idle since startup, so the keystroke was sent and the
frame taken before the app had read it. The parser had the next screen, the
PNG had the previous one, and nothing but the cross-check would have noticed.
`send_key` now waits for the app to react *before* waiting for it to settle.
A full six-frame run of `first_run` driven by `Down,Down,Enter` cross-checks
1674–9742 cells per frame with no disagreement.

**Progress (2026-09-19, Steps 1–4 built):** `crates/screenshot` exists and
captures. Working end to end: the run-root ignore, compositor lifecycle with
`sway -C` validation, cell measurement, and capture of the `first_run` scene
at all three sizes in both themes — six frames, each asserted to be exactly
`cols·cell_w × rows·cell_h`. The skill that drives it is
`.claude/skills/screenshot/`. Two corrections the build forced. **The cell is
8×18, not the 8×19 the probe recorded**: the probe inherited the developer's
`foot.ini`, and pinning `--config=/dev/null` changes the answer — the Pitfall
now carries both numbers, because the gap between them is the point. And
**seeding `~/.mjolnir` by hand does not work**: `init_global_if_empty` treats
a directory missing any of permissions.yaml, mcp.yaml or tui.yaml as
half-deleted and refuses to start, so the scene materialises global config
through `mjolnir_config::Config` — the product's own writer — and patches only
the theme. Not built: the pty proxy (Step 5), so there is no declared cell
grid and therefore no gates; the fixture endpoint (Step 6), so ten of the
eleven catalogue scenes refuse to run rather than capture something else under
the right name; quiesce, which needs the proxy and is a `--settle-ms` interval
until then; and the report generator.

## Why

Every automated check on the TUI asserts `App`'s output through
`ratatui::backend::TestBackend` — 110 row-level tests in `ui/tests.rs`, plus
whole-buffer snapshots across four sizes and two themes. They are fast and
precise, and they share one blind spot: they read the cell grid the app
*intended*, never the bytes it emitted or what a terminal does with them.

That is where the last two releases' bugs came from. `40cb6b1` — Shift+Enter
gated behind a capability query that fails silently — is terminal capability
negotiation. `029984d` — the input drain polling crossterm with a no-op waker
— is the executor, and its regression guard (`tests/input_wakeup.rs`) is a
grep over source, because the crate cannot observe the defect at all. Both
were found by the developer using the app.

So the existing tests stop a fixed bug returning but cannot find a new one.
This spec adds the missing layer — shipped binary, real emulator, real pixels
— and scores the result against the design system instead of against the last
run.

## Vocabulary

- **Scene** — a named state the binary is put into before capture: a seeded
  `HOME`/`.mjolnir` config plus a scripted conversation, driven by keystrokes
  where needed. Scenes live in a catalogue; a session names the ones its goal
  needs.
- **Size** — one of the three geometries below. Each scene a session names is
  captured at all three, in both themes: **six frames per scene**. Theme is
  global config (`config.global_tui().theme`, read once by `Theme::from_config`
  — `cli/src/bootstrap.rs:220`), so the two themes are two seeded configs, not
  two renders of one capture.
- **Focus set** — the regions a session's goal is allowed to change.
  Everything else in the frame is out of focus and must not move.
- **Gate** — a zero-tolerance check. Gates are not scored, cannot be averaged
  away, and block the exit. Five are deterministic; `component exists` is a
  judgement, and the only thing it may do is stop the loop for a human.
- **Score** — the judged pair (spatial, component fidelity), 0–100 per frame.
  The loop's output is a score, the gate results, and the frames behind both.

## Model

```
headless sway (no GPU, nothing on the developer's desktop)
  └── foot   (pad 0, pinned font, size and TERM)
        └── mjolnir   (the shipped binary, isolated HOME)
grim, run inside that session → PNG
```

The output mode is computed as `cols × cell_w` by `rows × cell_h`, from a cell
size measured at startup. Capture waits until the pty reports the target size.

| | cells | pixels @ 8×18 | what it checks |
| --- | --- | --- | --- |
| small | 80 × 24 | 640 × 432 | **survival** — nothing clipped or overlapping, panel and composer both usable |
| medium | 120 × 36 | 960 × 648 | **conformance** — the design system's own frame, checked against it |
| large | 200 × 50 | 1600 × 900 | **restraint** — anchors hold, measure stays capped, nothing stretches |

## Decisions

- **The subject is the shipped binary in a real terminal, not `App` in a `TestBackend`.** — Only this can see capability negotiation, wide glyphs, wrapping, unset colours against a real palette, and malformed escapes that a `TestBackend` accepts.

- **foot is the only terminal, and the harness pins its config.** — One emulator, so scores compare across runs. Explicit pad, font, size and `TERM`, never `~/.config/foot/foot.ini`: a developer's local settings must not change a test result. foot earns the slot by being native Wayland, fully configurable from argv, and having `--pty`.

- **Cell size is measured at startup, never hardcoded.** — A wrong cell gives a frame that is wrong in every row and looks right; see Pitfalls.

- **tmux is excluded.** — It letterboxed silently in the probe (a 120×36 session against a 120×34 client, and the capture still looked correct), and its `TERM=tmux-256color` and re-emission hide the capability layer `40cb6b1` broke. Its three jobs are replaced: sizing by foot, input and cell capture by Step 5.

- **Three fixed sizes, each checking something different.** — 120×36 is not a free choice: it is the frame `.claude/design/` is authored at (`tokens/cells.css`, 9×20px) and the only size with a reference to check against. The other two are checked on rules derived from it.

- **Small is 80×24 for vertical pressure, not narrow width.** — At 24 rows `COMPOSER_MAX_ROWS = 10` takes 42% of the frame and `clamp_panel` in `ui/decision.rs` starts discarding rows. Most interesting failures are vertical; width mostly re-tests wrapping.

- **No standard size sits near where the option-detail column drops.** — That threshold is computed from actual label and detail widths (`decision.rs:335`), not fixed, and lands in the mid-50s for today's lists. A size on that boundary would flip with a one-word copy change and make the score noisy rather than wrong.

- **52×20 stays out.** — It keeps its cheap place in `render_snapshot.rs`. Scoring a deliberately degenerate layout measures how gracefully the UI fails, which is a different question.

- **Gates and scores are different kinds of thing and are never averaged together.** — A frame with truncated text is not 80% unbroken. Fold one binary failure into a percentage and a real break hides behind four good numbers — and silent clipping is this UI's recurring defect (`6c1ab32`, `1f125d4`).

- **Regression is the `render_snapshot.rs` diff, scoped to the focus set — not a second capture of the baseline revision.** — Regression is a comparison between two revisions, not a property of one frame, and `render.snap` is already a cell-by-cell text record that can be diffed by region. So the gate is: every region the `.snap` diff touches falls inside the focus set. It costs no second build and no second capture set, and it is immune to the timing volatility a live capture has, because `TestBackend` renders statically. The cost is stated plainly: it is an `App`-level check, so a regression that appears *only* through a real terminal is not what this gate catches. The two harnesses compose — the snapshot proves nothing outside the focus set moved, the screenshots judge whether what moved inside it is right.

- **A component the design system has no counterpart for halts the loop.** — The design is imported, not invented, so such a thing is not 60% correct. It is design debt needing a deliberate decision; ADR 0002 is what handling one looks like.

- **The threshold is the minimum across frames, never the mean.** — Three sizes in two themes is six frames, and a mean of 90 hides one frame at 40.

- **The exit condition is a conjunction, not a number.** — It is also what keeps self-scoring honest: an agent that both fixes and judges can rationalise a score, but the gates are deterministic and it cannot exit on the score alone.

- **Input goes through a pty proxy, not a virtual keyboard.** — The proxy is the only option that also yields the cell grid, which four of the six gates and one of the two scores read, and it needs nothing installed. The cost is recorded rather than argued away: the harness chooses the bytes, so foot's own key encoding is never exercised and a `40cb6b1`-class defect stays invisible to it. `wtype` remains the answer if that class recurs, and the two compose. What the proxy does *not* fake is the terminal: the app's capability queries are forwarded to foot and foot's replies come back, so only synthesised keypresses are the harness's invention.

- **The parser is checked against the picture on every capture.** — A subtly wrong terminal parser fails the same way a wrong cell size does: it yields a grid that looks entirely plausible, and every gate then reports confidently about cells the app never drew. So each frame must reconcile — a cell the parser calls "space on ground X" is a flat block of X in the PNG — and a run that cannot reconcile them is refused rather than scored. Cheap, since the capture produces both artefacts anyway.

- **The contrast gate checks role pairing, not a ratio.** — The design system states no ratios: its palette is OKLCH-generated, with lightness assigned per role rather than solved per pair, so there is no number to check against. A ratio floor would also flag the deliberately dim metadata tiers as failures. The defect this gate exists for — the light wordmark rendering a rung too dark — is a wrong-rung bug, which pairing catches exactly.

- **The loop caps at five iterations.** — An iteration is a full capture set, a judge pass and a fix; five rounds that have not reached 90 usually mean a problem the loop cannot grind out. The cap is a stop, not a verdict: the report says it capped and at what score.

- **The regression baseline is the merge-base with `main`.** — It stays fixed across a session however many commits the loop makes, and it answers the question actually being asked — what did this piece of work change — rather than what the last commit changed.

- **The harness is a workspace crate, `crates/screenshot` (`mjolnir-screenshot`).** — `CLAUDE.md` says all code here is Rust, and the pty proxy, the cell parsing and the image work are all comfortable in it. It is dev-only: `release.yml` builds `-p mjolnir-cli`, so nothing needs excluding, and nothing in the shipped binary may depend on it.

- **Scenes are a named catalogue; a session declares which ones its goal needs.** — The catalogue is seeded from the eleven `render_snapshot.rs` already defines (`empty`, `conversation`, `markdown`, `fenced_diff`, `tools`, `approval`, `approval_large`, `prompt`, `prompt_path`, `prompt_scoped`, `long`), so both harnesses speak one vocabulary and a scene is written once rather than re-derived per session. A run captures only what its goal touches: six frames per scene is real cost, and a standing full-suite run would spend most of it on states the change cannot reach.

- **Known deviations live in a baseline file, and every gate consults it before reporting a violation.** — Two frames are correct today in ways a gate will flag: ADR 0002's markdown table draws box-drawing glyphs the closed table forbids, and the light theme's wordmark renders a rung dark because the design system's own `--tui-reverse-bg` disagrees with its frame (`mjolnir-tui.md`, 2026-09-07: do not fix it in `palette.rs`). Without the file the first run scores the design system's bugs against us. Each entry names the gate it exempts, where it applies, and the ADR or spec entry that authorised it — an entry with no authority is not an exception, it is an unfixed bug — and the report lists every exception it applied, so a carve-out stays visible instead of settling into a file nobody reopens.

- **Spatial accuracy is checked against the tokens on every frame, and against the rendered handoff at 120×36 where the design drew that scene.** — The tokens alone would be near-circular: `ui/grid.rs` derives its constants from the same `cells.css`, so a token-only check largely confirms the renderer used its own numbers. It still catches a row that ignores the grid, which is why it is the floor; the handoff render is what makes at least one reference independent of the code. The two reference frames and the 33 screenshots are upstream, not in `.claude/design/` — fetching them needs `DesignSync` and the `design-sync` skill's traps.

- **The judged pair is scored by a separate agent, blind to the code.** — It sees the frames, the criteria, the focus set and the design reference; not the diff, not the source, not its own earlier scores. An agent that both fixes and judges converges on the scorer rather than on the design, and one that knows the intent behind a change is biased toward seeing that intent met. Blinding it to earlier scores also stops the number anchoring and drifting upward across iterations.

- **Frames are scored against criteria, not diffed pixel for pixel.** — Font rasterization, fontconfig and the foot version move pixels without moving the design. Exact equality keeps its home in `render_snapshot.rs`: that proves *unchanged*, this judges *correct*.

## Acceptance model

A session declares a **goal** and a **focus set** before anything runs — e.g.
"multi-line support in the composer: the input field meets the multi-line
design, regresses nothing else, and survives unexpected input". The goal names
what changed; the focus set names the regions allowed to change.

Three tiers run in that order and fail differently.

### 1. Preflight — once per run; any failure quits the skill

- **The measured cell matches the recorded one**, and the compositor came up with nothing on the output but foot.
- **The baseline is valid** — `render_snapshot.rs` is green at the merge-base, so a `.snap` diff is a claim about this change and not about pre-existing drift.
- **The goal, focus set and scenes are declared**, and the focus set resolves to regions.

**Capture invariants** are the same severity but cannot be checked once: every
single capture must come back at exactly `cols·cell_w × rows·cell_h`, from a
pty that reported the target size first. A capture that misses either is not
scored and not retried — it aborts the run like a preflight failure.

Both exist for one reason, and the 34-row frame in Pitfalls is it: a bad
capture does not score badly, it scores confidently and wrongly, and every
number downstream inherits that.

### 2. Gates — every frame, every iteration; zero tolerance

| gate | measured on | checks |
| --- | --- | --- |
| breakages | declared cells | truncated or clipped text, colliding runs, glyphs outside the closed vocabulary, cells the app never painted, misaligned table rows |
| colour | declared cells | every foreground and ground the app declares belongs to the theme's palette |
| role pairing | declared cells + region map | a cell's declared foreground and ground are the pair the design specifies for its region's role |
| regression | `render.snap` diff vs merge-base | every region the diff touches falls inside the focus set |
| content | declared cells | the mechanical part of Content Fundamentals — third-person "The agent", lowercase labels |
| component exists | judgement | nothing rendered that the design system has no counterpart for |

**"Declared cells" means the app's own output, not the picture.** The pty
proxy carries every escape sequence mjolnir emits, so the harness knows each
cell's character and its declared foreground and background exactly. Reading
colour off the PNG instead cannot work: font rasterization antialiases, so
every glyph edge is a blend that belongs to no palette, and a pixel carries no
notion of which run of text is a label rather than body. Role comes from the
region map the focus set already requires. The pixels stay worth capturing —
they are what a human reads in the report, and the one check that the terminal
actually painted what was declared. That check has one defect of its own to
catch: **font-fallback tofu**, where the declared cell is a perfectly correct
glyph and the font has no such glyph to draw. It is invisible in the cells by
definition, so it is the single breakage read from the image.

Gates do not halt scoring. They are computed alongside it and reported
together, so one iteration shows the whole picture instead of sending a fix
loop from one symptom to the next. They do block the exit.

`component exists` is the one gate that halts the loop rather than failing it:
it is a decision for a human, not a fix for the loop to invent.

### Capturing a stable frame

A capture is taken only once the app has gone quiet — nothing emitted for a
set interval, and no turn active. The spinner stops when idle, which removes
most of the volatility on its own; `render_snapshot.rs` never had this problem
because `TestBackend` renders statically and never sees a spinner.

Whatever is still volatile at rest — a counter, an elapsed time — is declared
in the baseline file and masked before any comparison. Nothing in the shipped
binary changes to make this work: a freeze hook would be test-only surface in
the product, which this spec does not take.

### Baseline file

A file checked in with the harness, recording what is correct by decision:
the known deviations, each with the gate it exempts, the scene or region it
covers, and its authority — plus the cells masked as volatile. Gates consult it before reporting a violation, and
the report lists which exceptions were applied.

It is not the regression baseline. The regression gate diffs `render.snap`
against the merge-base; this file says which *design* deviations are accepted,
and which cells are volatile enough to mask (below). Different questions, on a
different clock — the design system's, not the code's.

### 3. Score — the judged pair

| criterion | measured on | checks |
| --- | --- | --- |
| spatial | cells vs the reference frame | rows and columns sit where the handoff puts them |
| component fidelity | judgement | what is in focus matches its referenced design |

Scored 0–100 per frame by a **separate agent, blind to the code**: it is given
the frames, the criteria, the focus set and the design reference, and nothing
else — no diff, no source, no earlier scores. The threshold is **90, taken as
the minimum across every frame in the run** — six per scene, and a session may
name several.

Its feedback is therefore in visual terms ("the option list sits one cell left
of the body column"), and translating that back into code is the fixer's job,
not the judge's.

The reference it is given for spatial accuracy is the token-derived grid on
every frame — `cells.css`: margin 3, label column 8, gutter 2, so body text
lands on cell 13 — plus, at 120×36 and only for scenes the design system drew,
the rendered handoff frame. Rendering it means dividing its pixel positions by
its own 9×20 cell, never reading positions off the prose; `CLAUDE.md` records
what skipping that step cost last time.

### Exit

    preflight clean ∧ no gate violation ∧ min score ≥ 90 ∧ iterations ≤ 5

A report is written on every exit, including a cap-out, which it must say
plainly.

### Report

Each invocation gets its own directory — `target/screenshot-runs/<run-id>/` —
holding that session's frames, gate output and report. It is working space: a
run that fails, caps out or is abandoned keeps its directory as the evidence
for the next attempt.

Only the newest few, though. Deleting on acceptance is not a retention policy:
it says nothing about the runs nobody accepts, and those are the common case
while something is being built — this harness's own development left 70
directories and 24MB in an afternoon. Evidence is useful while it is recent, so
`start` prunes all but the last five and says how many it dropped, and `clean`
does the same on demand.

Acceptance is an explicit human action after reading the report — the skill
asks, and deletes the directory only on a yes. Nothing is removed on the
skill's own judgement, including after a passing run: a 90 the human has not
looked at is not an accepted 90.

The report is a **single self-contained HTML file** with its frames embedded,
so it outlives that directory — a human who wants to keep or send one still
can after the run space is gone.

It opens with a **verdict**: passed, failed at a named gate, or capped at N
iterations with a minimum score of M. Everything below is evidence for that
line, in the three tiers in the order they ran.

1. **Preflight** — what was checked, and that it cleared. Short by
   construction: a run that produced a report cleared it.
2. **Gates** — each gate, its result, and every violation with its cell
   coordinates.
3. **Score** — spatial and component fidelity per frame, the minimum across
   frames, and the threshold it is held to.

Then the **final frames** — six per scene the session named, three sizes ×
two themes — with violations marked on the image. A cell maps to a pixel rect, so a gate's coordinates can
be drawn; leaving them undrawn makes a human hunt for what the harness already
found. A frame with nothing to mark is shown clean.

The report also carries the goal and focus set the run was held to, the
baseline commit, and what changed on each iteration.

It is a tool artifact, not a Mjolnir surface: plain, legible HTML. **It does
not follow the design system.** That grammar belongs to the TUI, and dressing
the reference up in the thing it is judging would make it harder to trust, not
easier.

## Steps

1. Run-root ignore — this repo has no `.gitignore`. Add one covering `target/screenshot-runs/` before anything writes there. Ignoring the run root is all this spec needs: an ignore rule does not untrack what is already tracked, so `target/`'s 402 tracked files stay a separate question.

2. Compositor lifecycle — start a headless sway with a harness-owned config and socket, check it came up clean, tear it down on exit.

3. Cell measurement — launch foot once, read the pty size back after it settles, derive `cell_w`/`cell_h`. Fail loudly if it disagrees with the recorded value rather than silently resizing the frame.

4. Capture — for (scene, size, theme): set the output mode, launch foot with the pinned config and an isolated `HOME`, wait for quiesce, capture with `grim`, and enforce the capture invariants. Keep the declared cell grid from the proxy alongside the PNG: the gates read the first, the report shows the second.

5. Input and cell capture — a **pty proxy**. The harness owns two ptys: foot is attached to one with `foot --pty` (confirmed working against a pty owned by another process, which also sets the window size on it), mjolnir runs on the other, and the harness pumps bytes between them. That gives keystroke injection, a verbatim copy of everything the app draws — the cell grid the gates read — and one place to propagate window size and `SIGWINCH` from foot's pty to the app's.

6. Scenes — a **fixture LLM endpoint**. A scene is a seeded `HOME`/`.mjolnir` config plus a scripted conversation the real binary plays out against a local fake provider, driven where needed by keystrokes through the proxy. `.mjolnir/provider.yaml`'s `base_url` is written per run, since the server binds an ephemeral port; the theme is seeded in global config.
   - Why: it adds no test-only surface to the shipped binary, and a scene exercises the whole real path — core, tools, permissions, TUI — rather than a seeded render. A `--scene` flag or a second binary would be faster and could reach any state directly, but both test rendering instead of behaviour, which is the blind spot this spec exists to close.
   - `llm/src/test_server.rs` is a **model, not a dependency**: it is `#[cfg(test)] mod` inside `mjolnir-llm` (`lib.rs:14`), so it is unreachable from outside the crate. Either expose it or write the harness's own; it queues canned responses per connection, which is the right shape.

7. Scene catalogue — port the `render_snapshot.rs` scene names onto the fixture endpoint. This is **not** a rename: those scenes are built by pushing `LogEntry` values straight into `App`, and reaching the same state through a conversation is real work. Done: `first_run`, `empty`, `conversation`, `markdown`, `fenced_diff`, `long`. Left: the tool-driven ones — `tools`, `approval`, `approval_large`, `prompt`, `prompt_path`, `prompt_scoped` — each of which needs a tool-call delta and a particular permission state, and `approval` in particular needs an edit whose diff the gate will hold open. A scene that cannot be reached is reshaped or dropped **with a note in the catalogue** — the two harnesses share one vocabulary or the shared names become a trap.

8. Preflight and capture invariants — the run-level checks as a single pass that either clears the run or stops it with a named fix, and the per-capture invariants enforced on every frame.

9. Gates — the six checks, five reading the declared cells from the proxy and one a judgement, each consulting the baseline file and reporting surviving violations with their cell coordinates so a fix has somewhere to start. Seed the file with the two known deviations. Palette, glyph vocabulary and the ADR 0002 exemption come from `.claude/design/`, not from constants restated here.
   - `role pairing` needs a **region map** — which cells are a label, body, chrome or a panel — because a role is not recoverable from a colour. That map is the same structure the focus set needs, so building one serves both.

10. Scoring and report — the judged pair per frame from the blind judge, the minimum-across-frames threshold, and the report described above: run directory, self-contained HTML, verdict first, three tiers, annotated frames. Written on every exit, not only a successful one.

11. Loop driver — the skill: declare goal, focus set and scenes; iterate capture → gates → score → fix; stop on the exit condition or at five iterations, then report and ask before deleting the run directory.

## Pitfalls

- **The same font at the same size measures two different cells.** The probe measured 8×19 for JetBrains Mono at size 10; the harness, which passes `--config=/dev/null`, measures **8×18** — the probe had inherited the developer's own `foot.ini`. Earlier still it assumed 8×18 against that contaminated 8×19 and got a window holding 34 rows instead of 36, which looked entirely correct: colours right, geometry wrong, no error anywhere. Two lessons, not one: measure the cell, and measure it under the config the harness pins, or the number is about someone's desktop.
- **foot starts the pty at 80×24 and resizes after the window is configured.** Waiting on a sleep rather than on the pty's reported size captures pre-resize frames intermittently.
- **`grim` run outside the session captures the developer's real screen**, silently and successfully. Launch it via `swaymsg exec`.
- **The compositor must be silent.** One invalid line in a sway config painted a red error bar across the top of every frame.
- Anything that re-emits the app's output (tmux, asciinema) drifting back in for convenience — it removes the thing under test.
- The scoring turning into a pixel diff because a diff is easier to compute than a judgement; it then reports noise on every font update and gets ignored.
- A catalogue scene quietly reshaped to whatever the fixture endpoint could reach, so two harnesses use one name for two states. Reshape deliberately and write it down, or drop the scene.
- The catalogue and `render_snapshot.rs` drifting apart, so a scene name means two different states. They are one vocabulary or they are a trap.
- A session naming too few scenes, so the regression gate has nothing to say about the regions a change was most likely to disturb.
- The baseline file growing into a suppression dump. The citation requirement is what holds it back: an exception that cannot name the decision behind it is a bug being silenced.
- A gate decided from pixels. Antialiasing makes that unbuildable for colour and meaningless for role; gates read the declared cells, and the PNG is evidence for the human.
- The quiesce interval tuned until captures stop flapping, rather than the volatile cell being found and masked. That buys a green run by making the harness slower and no more honest.
- Masking growing to cover a cell that is volatile because of a bug.
- Reading positions off the handoff's prose instead of measuring its HTML, or forgetting its cell is 9×20 while ours is 8×18 — the comparison is between *cell coordinates*, never pixels.
- A design re-sync moving the handoff without the spatial reference being refetched, so the judge scores against a frame the design system has already replaced.
- The judge acquiring context it should not have — the diff, the source, its own earlier scores — because it would make its feedback more actionable. That is the whole defence, traded away for convenience.
- The loop optimising the scorer instead of the design. The gates are the defence, so the judged surface stays small: moving a criterion from gate to score is a loss, not a simplification.
- Chasing the threshold by editing outside the focus set — the regression gate is what catches it, which is why it is a gate and not a score.
- A mean creeping back in because it is easier to report than a minimum.
- Running without an iteration cap, or exiting on the cap without saying so in the report.
- A stale `.snap`, so the regression gate compares against drift rather than against this change — preflight checks it is green at the merge-base for exactly this reason.
- Reading the regression gate as terminal-level proof. It is an `App`-level diff; what it asserts is that no *region* outside the focus set moved.
- Accepting a report deletes its run directory. Anything wanted beyond the single run — a score trend across changes, say — has to be taken out before acceptance, or it is gone.
- The run root not being ignored by git — every session then dirties the tree. As of 2026-09-19 this repo has no `.gitignore` at all and tracks 402 files under `target/`; Step 1 is what closes this, and it must land before the first capture.
- The proxy failing to propagate window size and `SIGWINCH` from foot's pty to the app's — the app then lays out for a size the frame is not.
- Treating a green run as evidence about key handling. The proxy tests the bytes the harness chose; what foot would actually send for that key is untested by construction.
- The harness inheriting the developer's environment — foot config, fontconfig, `TERM`, `HOME` — so a run means something different on each machine.

## Out of Scope

- Cell-level assertions on `App` — `render_snapshot.rs` and `ui/tests.rs`. This does not replace them.
- `crates/tui/examples/{preview,snapshot}.rs`. They stay; `snapshot.rs` is still the right tool for comparing a design against the handoff, but it renders `App`, so it shares the blind spot this spec closes.
- Functional testing — a separate loop, deliberately not mixed in.
- CI. This needs a compositor and will not run in a plain container. No CI runs the suite today, so it is a cost to take later.

## References

- .claude/spec/mjolnir-tui.md — the surface under test; read the 2026-09-06 Progress entry before touching layout.
- .claude/design/ — what the criteria come from; `IMPORT.md` first, then `tokens/cells.css` for the grid. Local copy is prose and tokens only: the `.dc.html` frames and the 33 screenshots are upstream, reachable with `DesignSync` after reading the `design-sync` skill.
- .claude/adr/0002-markdown-tables-are-drawn.md — the one stroked thing a "no strokes" check must exempt.
- crates/tui/tests/render_snapshot.rs — the exact-equality check this sits above.
- crates/tui/tests/input_wakeup.rs — why a defect class can be invisible to every test in the crate.
- https://codeberg.org/dnkl/foot — foot; `man foot` for `--pty` and `-o`.
- https://github.com/atx/wtype — wtype; not used, kept as the answer if key-encoding coverage is ever needed.
