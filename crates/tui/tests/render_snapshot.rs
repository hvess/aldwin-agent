//! Full-fidelity render snapshots — the automated form of the
//! "render it before trusting your reading of it" discipline in this
//! project's `CLAUDE.md`.
//!
//! Every scene below is drawn against `ratatui::backend::TestBackend` at
//! four frame sizes in both themes, and the *entire* resulting buffer —
//! every cell's symbol, foreground, background and modifiers — is
//! serialized to `tests/snapshots/render.snap`. The unit tests in
//! `ui.rs` assert facts about individual rows; this asserts the whole
//! frame, colors included, which is what makes a layout refactor
//! provably output-preserving rather than merely test-passing.
//!
//! Build identity — the release version, the commit, the working directory
//! — is pinned per scene rather than inherited from the build (see
//! [`fixed_identity`]). The first cut of this file did inherit it, which
//! meant the snapshot encoded who generated it: it broke on the next
//! commit, on a dirty tree, and on any checkout at a different path.
//!
//! Regenerate deliberately, after eyeballing the diff:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p mjolnir-tui --test render_snapshot
//! ```

use std::fmt::Write as _;
use std::sync::Arc;

use mjolnir_config::Config;
use mjolnir_permissions::{Class, Engine, PromptPayload};
use mjolnir_tui::{App, LogEntry, ModelChoice, ProviderChoice, Theme, ToolActivityEntry, ToolActivityStatus, TurnEndReasonKind};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

const SNAPSHOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots/render.snap");

/// The three sizes the review loop uses, and the same three
/// `crates/review`'s `Size` enum captures — small for vertical pressure,
/// the design system's own 120×36 frame, and a maximized terminal whose job
/// is to catch a layout that sprawls rather than one that clips.
///
/// They match on purpose. Stage 4 baselines these frames and stage 5 looks
/// at pictures of the same ones, so a judge's finding and a snapshot diff
/// name the same frame.
const SIZES: [(u16, u16); 3] = [(80, 24), (120, 36), (200, 50)];

const SCENES: [&str; 11] = [
    "empty",
    "conversation",
    "markdown",
    "fenced_diff",
    "tools",
    "approval",
    "approval_large",
    "prompt",
    "prompt_path",
    "prompt_scoped",
    "long",
];

/// The provider list `5d` shows, in the reference's own order and copy.
/// Pinned here rather than read from config for the same reason the build
/// identity is: a snapshot that inherits its content breaks on the next
/// change to something it is not testing.
fn first_run_state() -> mjolnir_tui::__PreviewFirstRun {
    let providers: Vec<ProviderChoice> = [
        ("anthropic", "claude models · ANTHROPIC_API_KEY", ["opus-4.6", "sonnet-4.6", "haiku-4.6"]),
        ("google", "gemini models · GOOGLE_API_KEY", ["gemini-3-pro", "gemini-3-flash", "gemini-3-lite"]),
        ("openai", "gpt models · OPENAI_API_KEY", ["gpt-6", "o5", "gpt-6-mini"]),
        ("mistral", "mistral models · MISTRAL_API_KEY", ["large-3", "codestral-2", "small-3"]),
    ]
    .into_iter()
    .map(|(id, purpose, models)| {
        let models = models
            .into_iter()
            .zip(["deepest reasoning · 200k", "balanced · 200k", "fast, cheap · 200k"])
            .map(|(id, purpose)| ModelChoice::new(id, purpose))
            .collect();
        ProviderChoice::new(id, purpose, models)
    })
    .collect();
    mjolnir_tui::__PreviewFirstRun::new(providers, 3, true, true)
}

#[test]
fn every_scene_renders_exactly_as_recorded() {
    let mut out = String::new();
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                let _ = writeln!(out, "=== {theme:?} {scene_name} {width}x{height}");
                out.push_str(&serialize(&buffer));
            }
        }
        // First run is the design system's `5d` and the one screen the app
        // draws outside `App` — it runs its own terminal loop rather than
        // being a mode, which is why it needs the preview draw rather than
        // going through `scene` above. Baselined here so every screen the
        // design specifies has one, and so stage 4 and stage 5 look at the
        // same twelve scenes.
        for (width, height) in SIZES {
            let state = first_run_state();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
            terminal.draw(|f| mjolnir_tui::__preview_draw_first_run(f, &state, theme)).expect("draw");
            let _ = writeln!(out, "=== {theme:?} first_run {width}x{height}");
            out.push_str(&serialize(terminal.backend().buffer()));
        }
    }

    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(SNAPSHOT, &out).expect("write snapshot");
        return;
    }

    let expected = std::fs::read_to_string(SNAPSHOT).unwrap_or_else(|e| {
        panic!("missing snapshot {SNAPSHOT} ({e}) — regenerate with UPDATE_SNAPSHOTS=1");
    });
    if expected != out {
        let actual_path = std::env::temp_dir().join("mjolnir-render.actual.snap");
        let _ = std::fs::write(&actual_path, &out);
        panic!("{}\n\nfull output written to {}", first_difference(&expected, &out), actual_path.display());
    }
}

/// The permission panel is the band height the design states.
///
/// `cells.css`'s `--panel-permission-h` is 18 rows and `HANDOFF.md:271`
/// restates it. A token, so there is nothing to interpret — which is exactly
/// why this belongs here rather than in a judge's prompt. Two stage 5 judges
/// found it independently on the same run; a finding produced twice is a
/// finding that should stop costing a model's attention.
///
/// **Every permission panel.** The app draws one component and titles both
/// payloads `permission`; the token names that band. Scoping this to the
/// tool prompt for one iteration left `approval` at 12 rows while
/// `approval_large` reached 18 by accident of content, which a stage 5 judge
/// caught — the same panel at two heights depending on its payload is what
/// the token exists to prevent.
///
/// **Not at 80x24.** Eighteen rows plus a 3-row top bar plus the five rows
/// `decision::max_height` reserves for the conversation is 26, and the frame
/// is 24. The design specifies one frame and it is not that one.
#[test]
fn the_permission_panel_is_the_band_height_the_design_states() {
    const PROMPTS: [&str; 5] = ["prompt", "prompt_path", "prompt_scoped", "approval", "approval_large"];
    let expected = mjolnir_tui::__design_panel_rows();
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in PROMPTS {
            for (width, height) in SIZES.iter().filter(|(_, h)| *h >= 36) {
                let (width, height) = (*width, *height);
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                // The panel runs from its title row to the bottom of the
                // frame. The title row is found by its content rather than
                // its tone: `permission` on the 3-cell margin is what
                // `HANDOFF.md:273` specifies and what no other row carries.
                let first = (0..height)
                    .find(|y| row_text(&buffer, *y, width).trim_start().starts_with("permission"))
                    .unwrap_or_else(|| panic!("{theme:?} {scene_name} {width}x{height}: no panel title row"));
                let rows = height - first;
                assert_eq!(
                    rows as usize, expected,
                    "{theme:?} {scene_name} {width}x{height}: the panel runs rows {first}..{} — {rows} rows, not the {expected} \
                     that cells.css --panel-permission-h states",
                    height - 1
                );
            }
        }
    }
}

fn row_text(buffer: &Buffer, y: u16, width: u16) -> String {
    (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect()
}

/// Which rows of a scene the **app** owns, as opposed to echoing from
/// outside it.
///
/// This boundary is the whole subtlety in the two checks below. The closed
/// glyph table and the no-contractions rule govern Mjolnir's own copy —
/// labels, hints, panel sentences, the status row. They do not govern a
/// model's reply, which the transcript renders verbatim and which may
/// legitimately contain an em dash, a contraction, or any Unicode at all. A
/// global buffer-level check would fail on ordinary use, which is a check
/// that is wrong rather than strict.
///
/// The buffer cannot tell the two apart — by the time text is cells, its
/// provenance is gone. So the split is made by scene and band:
///
/// * `first_run`, `empty` and the `prompt*` family render no model text, so
///   the whole frame is the app's. (A `prompt` panel quotes a path, which is
///   ASCII and carries neither a mark nor a contraction.)
/// * `approval*` quote a diff — file content, not the app's — so only their
///   chrome bars are checked.
/// * Every other scene renders a transcript, so likewise.
///
/// What this misses, stated rather than implied: app copy drawn *inside* a
/// transcript body on a prose scene. There is little of it — the tool line's
/// name column and the `N more lines` marker — and it is covered wherever
/// the same code draws into a panel instead.
fn app_owned_rows(scene_name: &str, height: u16) -> Vec<u16> {
    const WHOLE_FRAME: [&str; 5] = ["first_run", "empty", "prompt", "prompt_path", "prompt_scoped"];
    if WHOLE_FRAME.contains(&scene_name) {
        return (0..height).collect();
    }
    let top = 0..3u16;
    let bottom = height.saturating_sub(5)..height;
    top.chain(bottom).collect()
}

/// Every cell the frame paints carries a colour from the design system.
///
/// Stronger than its neighbour below, which only asserts the colour is not
/// the terminal's own default. This asserts *membership*: the value is one
/// of the forty-two roles `tokens.rs` carries, or one of those dimmed
/// toward a ground, which is what the app does to a transcript behind an
/// open panel.
///
/// `tokens.rs` is generated from `.claude/design/tokens/`, so this is
/// conformance to the design rather than to a copy of it. The check used to
/// run against a real terminal through the review harness — a compositor, a
/// subprocess and 2m45s, and not hermetic. A `TestBackend` buffer holds the
/// same declared cells, so it runs here in milliseconds instead.
#[test]
fn every_cell_carries_a_colour_from_the_design_system() {
    for theme in [Theme::Dark, Theme::Light] {
        let palette = mjolnir_tui::__design_palette(theme);
        // The dimmed transcript behind an open panel: every ink the app has,
        // composited over every ground it has. Enumerated rather than
        // solved for — the blend is a known function of two known sets.
        let mut allowed: Vec<ratatui::style::Color> = palette.to_vec();
        for ink in palette {
            for ground in palette {
                allowed.push(mjolnir_tui::__design_fade(*ink, *ground));
            }
        }
        allowed.sort_by_key(|c| format!("{c:?}"));
        allowed.dedup();

        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                for y in 0..height {
                    for x in 0..width {
                        let cell = &buffer[(x, y)];
                        for (which, colour) in [("foreground", cell.fg), ("background", cell.bg)] {
                            // Whitespace paints no ink, so a colourless
                            // foreground on a blank cell is an idiom, not a
                            // defect — the same carve-out the neighbour below
                            // makes, and for the same reason.
                            if which == "foreground" && cell.symbol().trim().is_empty() {
                                continue;
                            }
                            assert!(
                                allowed.contains(&colour),
                                "{theme:?} {scene_name} {width}x{height} at ({x},{y}): {which} {colour:?} is not a design token, \
                                 nor a token dimmed toward a ground"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Every glyph comes from the design system's closed table.
///
/// The table is `HANDOFF.md`'s, parsed into `tokens.rs` by the generator.
/// `MARKS_BY_EXCEPTION` is the set a recorded design contradiction licenses
/// on top of it — the `·` the design's own copy mandates but its table
/// omits, and ADR 0002's box-drawing set. Both are bugs upstream, and both
/// leave `crates/review/baseline.json` when the design is fixed.
#[test]
fn every_glyph_comes_from_the_closed_table() {
    let (marks, by_exception) = mjolnir_tui::__design_glyphs();
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                for y in app_owned_rows(scene_name, height) {
                    for x in 0..width {
                        for ch in buffer[(x, y)].symbol().chars() {
                            if ch.is_ascii() || marks.contains(&ch) || by_exception.contains(&ch) {
                                continue;
                            }
                            panic!(
                                "{theme:?} {scene_name} {width}x{height} at ({x},{y}): {ch:?} is not in the design \
                                 system's glyph table and no recorded contradiction licenses it"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// The mechanical half of the Content Fundamentals: the agent is written
/// about in the third person, and the design system's copy uses no
/// contractions.
///
/// Found a real defect when it first ran against every frame: the permission
/// panel's elision row read `1 more line not shown; deciding doesn't require
/// scrolling them` — a contraction, and a plural pronoun for a count of one,
/// on every 80×24 frame.
///
/// The bare pronoun needs more care than the contractions do, and the first
/// version of this proved it by firing on the wordmark: `M J O L N I R` is
/// letter-spaced, so it contains a literal `"I "`. Requiring a lowercase word
/// after it separates `I can` from `I R`.
#[test]
fn rendered_copy_is_third_person_and_uses_no_contractions() {
    const FIRST_PERSON: [&str; 6] = ["I'm", "I'll", "I've", "we ", "We ", "our "];
    const ENCLITICS: [&str; 6] = ["n't", "'re", "'ll", "'ve", "'s ", "'d "];

    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                for y in app_owned_rows(scene_name, height) {
                    let row: String = (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect();
                    let bytes = row.as_bytes();
                    let starts_word = |at: usize| at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
                    let where_ = format!("{theme:?} {scene_name} {width}x{height} row {y}");

                    for form in FIRST_PERSON {
                        for (at, _) in row.match_indices(form) {
                            assert!(!starts_word(at), "{where_}: first person {form:?} in {:?}", row.trim());
                        }
                    }
                    for (at, w) in bytes.windows(3).enumerate() {
                        let bare_i = starts_word(at) && w[0] == b'I' && w[1] == b' ' && w[2].is_ascii_lowercase();
                        assert!(!bare_i, "{where_}: first person \"I\" in {:?}", row.trim());
                    }
                    for form in ENCLITICS {
                        for (at, _) in row.match_indices(form) {
                            let inside_word = at > 0 && bytes[at - 1].is_ascii_alphanumeric();
                            assert!(!inside_word, "{where_}: contraction {form:?} in {:?}", row.trim());
                        }
                    }
                }
            }
        }
    }
}

/// Every cell the frame paints must carry palette colours, not the
/// terminal's own defaults — the invariant behind `palette.rs` existing at
/// all, asserted here rather than left to a reader spotting a bare
/// `Span::raw` in review.
///
/// `Color::Reset` means "whatever this terminal paints by default", so a
/// cell carrying one is outside the design system: it renders near-black on
/// a light-profile terminal and near-white on a dark one, and no palette
/// change can move it. Two separate rules, because the two channels fail
/// differently:
///
/// * **Background** — never `Reset` anywhere, blank cells included. A
///   `Reset` background is a hole in the opaque canvas `ui::draw` paints
///   first, showing the developer's terminal through the frame.
/// * **Foreground** — never `Reset` on a cell that actually carries a
///   glyph. Whitespace is exempt: `grid`/`row` build margins and gutters
///   from bare `Span::raw`, which paints no ink, so constraining those
///   would forbid an idiom that is genuinely colourless.
///
/// Found one real violation when written: the `, ` between tool names in
/// `chrome::draw_status_line` was a bare `Span::raw`, the single cell in
/// the whole 315k-cell corpus painting a visible glyph in the terminal's
/// foreground rather than a token.
#[test]
fn every_painted_cell_uses_a_palette_colour_never_the_terminals_own() {
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                let where_ = |x: u16, y: u16| format!("{theme:?} {scene_name} {width}x{height} at ({x},{y})");
                for y in 0..height {
                    for x in 0..width {
                        let cell = &buffer[(x, y)];
                        assert_ne!(
                            cell.bg,
                            ratatui::style::Color::Reset,
                            "{}: background is Color::Reset — the terminal's own background shows through the frame here",
                            where_(x, y)
                        );
                        if cell.symbol().trim().is_empty() {
                            continue;
                        }
                        assert_ne!(
                            cell.fg,
                            ratatui::style::Color::Reset,
                            "{}: glyph {:?} is painted in Color::Reset — the terminal's own foreground, not a palette token \
                             (a bare `Span::raw`/`Style::default()` carrying visible text)",
                            where_(x, y),
                            cell.symbol()
                        );
                    }
                }
            }
        }
    }
}

/// A human-readable pointer at the first differing line, so a failure says
/// *what* moved rather than only *that* something did.
fn first_difference(expected: &str, actual: &str) -> String {
    let mut section = "<start>";
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e.starts_with("=== ") {
            section = e;
        }
        if e != a {
            return format!("render changed in {section} (line {}):\n  expected: {e}\n  actual:   {a}", i + 1);
        }
    }
    format!("render changed in length: expected {} lines, got {}", expected.lines().count(), actual.lines().count())
}

/// Replaces the three facts that vary with the build and the machine — the
/// release version, the commit, and the working directory — with fixed
/// stand-ins, so the snapshot records layout and colour rather than the
/// identity of whoever regenerated it. Chosen to be representative widths:
/// a three-part version, an 8-character short hash, and the design
/// system's own example path.
fn fixed_identity(mut app: App) -> App {
    app.status.version = "0.0.0".into();
    app.status.commit = "0badc0de".into();
    app.status.cwd = Some("~/src/gateway".into());
    app
}

fn engine() -> Arc<Engine> {
    // Leaked rather than held in a `TempDir` guard: the engine only reads
    // the (empty) config it was opened with, and every scene wants the same
    // default-deny state, so keeping the directory alive for the whole test
    // is simpler than threading a guard through each scene.
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
    Arc::new(Engine::new(config))
}

fn render(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| mjolnir_tui::__preview_draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// One row per line: the row's symbols verbatim, then a run-length-encoded
/// style track (`fg/bg/modifier-bits×count`). Symbols stay readable so a
/// layout diff is legible at a glance; the style track makes a pure color
/// regression just as loud as a moved character.
fn serialize(buffer: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        let mut symbols = String::new();
        let mut styles: Vec<(String, usize)> = Vec::new();
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            symbols.push_str(cell.symbol());
            let key = format!("{:?}/{:?}/{}", cell.fg, cell.bg, cell.modifier.bits());
            match styles.last_mut() {
                Some((last, count)) if *last == key => *count += 1,
                _ => styles.push((key, 1)),
            }
        }
        let track = styles.iter().map(|(key, count)| format!("{key}x{count}")).collect::<Vec<_>>().join(" ");
        let _ = writeln!(out, "{y:>3}|{symbols}|{track}");
    }
    out
}

fn scene(name: &str, app: &mut App) {
    match name {
        "empty" => {}
        "conversation" => conversation(app),
        "markdown" => markdown(app),
        "fenced_diff" => fenced_diff(app),
        "tools" => tools(app),
        "approval" => approval(app, SMALL_DIFF),
        "approval_large" => approval_large(app),
        "prompt" => prompt(app),
        "prompt_path" => prompt_path(app),
        "prompt_scoped" => prompt_scoped(app),
        "long" => long(app),
        other => panic!("unknown scene {other:?}"),
    }
}

const SMALL_DIFF: &str = "--- a/src/page.rs\n+++ b/src/page.rs\n@@\n fn page(items: &[Item], size: usize, n: usize) -> &[Item] {\n     let start = n * size;\n-    let end = start + size;\n+    let end = (start + size).min(items.len());\n     &items[start..end]\n }\n";

fn conversation(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "Can you refactor the retry logic in llm/src/client.rs to use exponential backoff?".into() });
    app.log.push(LogEntry::AssistantText {
        text: "Sure — here's the plan:\n\n1. Add a `backoff_ms` helper\n2. Wire it into the retry loop\n3. Cap at **5** attempts\n\n```rust\nfn backoff_ms(attempt: u32) -> u64 {\n    100 * 2u64.pow(attempt)\n}\n```\n\nThat gives `100ms, 200ms, 400ms, ...`. Want me to apply it?".into(),
    });
    app.log.push(LogEntry::Notice { message: "context file AGENTS.md injected (project scope)".into() });
    app.status.turn = Some(3);
    app.status.step = Some(2);
    app.input = "/theme light and then keep going".into();
    app.cursor = app.input.chars().count();
}

/// Exercises every inline and block markdown branch in one entry — the
/// wrapper, the heading/bullet/ordered/blockquote/rule prefixes, and the
/// nested inline delimiters.
fn markdown(app: &mut App) {
    app.log.push(LogEntry::AssistantText {
        text: "# Heading one\n### Heading three\n\nProse with **bold**, *italic*, _also italic_, `inline code`, ~~struck~~ and a [link](https://example.com/very/long/path) in it, long enough that it has to wrap across more than one row on any reasonable frame width.\n\n> A blockquote line\n\n- first bullet\n- second bullet with enough words that it wraps too\n\n1. ordered one\n2) ordered two\n\n---\n\nsupercalifragilisticexpialidociousandthensomemoretomakeitunbreakablyloooooong"
            .into(),
    });
}

fn fenced_diff(app: &mut App) {
    app.log.push(LogEntry::AssistantText { text: format!("Here is the change:\n\n```diff\n{SMALL_DIFF}```\n\nApply it?") });
}

fn tools(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "grep for TODO across the repo and summarize".into() });
    app.log.push(LogEntry::ToolActivity {
        step_id: mjolnir_core::StepId(1),
        calls:   vec![
            ToolActivityEntry { call_id: "c1".into(), name: "shell".into(), status: ToolActivityStatus::Completed { is_error: false, summary: "42 matches across 17 files".into() } },
            ToolActivityEntry { call_id: "c2".into(), name: "read".into(), status: ToolActivityStatus::Running },
            ToolActivityEntry { call_id: "c4".into(), name: "".into(), status: ToolActivityStatus::Completed { is_error: true, summary: "exit status 1".into() } },
        ],
    });
    app.turn_active = true;
    app.status.running_tools =
        vec![mjolnir_tui::__PreviewRunningTool { call_id: "c2".into(), name: "read".into() }, mjolnir_tui::__PreviewRunningTool { call_id: "c3".into(), name: "shell".into() }];
    app.status.turn = Some(4);
    app.status.step = Some(1);
}

fn approval(app: &mut App, diff: &str) {
    app.log.push(LogEntry::UserMessage { text: "fix the off-by-one in the pagination helper".into() });
    app.log.push(LogEntry::ApprovalCard { call_id: "call-1".into(), diff: diff.into(), resolution: None });
    app.pending_approvals.push_back(mjolnir_tui::__PreviewPendingApproval { call_id: "call-1".into(), diff: diff.into() });
    app.decision_selected = 1;
}

/// A diff far too tall for any frame — drives `clamp_panel`'s truncation
/// path and the elided-context marker, neither of which the small diff
/// reaches.
fn approval_large(app: &mut App) {
    let mut diff = String::from("--- a/src/big.rs\n+++ b/src/big.rs\n@@\n");
    for i in 0..40 {
        diff.push_str(&format!("+    let value_{i} = compute_something_reasonably_long({i});\n"));
    }
    for i in 0..10 {
        diff.push_str(&format!(" // untouched context line {i}\n"));
    }
    diff.push_str("-    let gone = 1;\n");
    approval(app, &diff);
}

fn prompt(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "run the test suite".into() });
    let payload = PromptPayload::Tool { program: "cargo".into(), argv: vec!["test".into(), "--workspace".into()], declared: Class::Write };
    app.log.push(LogEntry::PermissionPrompt { call_id: "call-2".into(), payload: payload.clone(), resolution: None });
    app.pending_prompts.push_back(mjolnir_tui::__PreviewPendingPrompt { call_id: "call-2".into(), payload });
}

fn prompt_path(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "what does the dispatcher do on a deny-by-absence?".into() });
    let payload = PromptPayload::Tool { program: "read".into(), argv: vec!["./crates/tools/src/dispatcher.rs".into()], declared: Class::Read };
    app.log.push(LogEntry::PermissionPrompt { call_id: "call-3".into(), payload: payload.clone(), resolution: None });
    app.pending_prompts.push_back(mjolnir_tui::__PreviewPendingPrompt { call_id: "call-3".into(), payload });
}

/// The same path-like prompt after Tab has widened the grant to the whole
/// directory, with a second request queued behind it — covers the
/// alternate-scope hint line and the "(+N more pending)" note.
fn prompt_scoped(app: &mut App) {
    prompt_path(app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.decision_selected = 3;
    let queued = PromptPayload::Tool { program: "read".into(), argv: vec!["./crates/core/src/lib.rs".into()], declared: Class::Read };
    app.pending_prompts.push_back(mjolnir_tui::__PreviewPendingPrompt { call_id: "call-4".into(), payload: queued });
}

fn long(app: &mut App) {
    for i in 0..8 {
        app.log.push(LogEntry::UserMessage { text: format!("message {i}") });
        app.log.push(LogEntry::AssistantText { text: format!("reply {i} with a bit more text to see wrapping behavior across the pane width") });
    }
    app.log.push(LogEntry::TurnEnded { reason: TurnEndReasonKind::EndTurn });
    app.log.push(LogEntry::TurnEnded { reason: TurnEndReasonKind::Cancelled });
    app.log.push(LogEntry::RetryAttempt { info: mjolnir_core::RetryInfo { provider: "anthropic".into(), status: Some(529), message: "overloaded, retrying".into(), attempt: 1 } });
    app.log.push(LogEntry::Error { message: "provider returned 529 overloaded".into() });
    app.log.push(LogEntry::ApprovalCard { call_id: "done-1".into(), diff: SMALL_DIFF.into(), resolution: Some(true) });
    app.log.push(LogEntry::PermissionPrompt {
        call_id:    "done-2".into(),
        payload:    PromptPayload::Tool { program: "ls".into(), argv: vec!["-la".into()], declared: Class::Write },
        resolution: Some(mjolnir_tui::PromptResolution { allowed: true, label: "allowed once".into() }),
    });
    app.status.turn = Some(9);
    app.status.step = Some(3);
}

/// Every box a frame opens, it must close.
///
/// The defect this guards: the decision panel used to be trimmed to fit by
/// a blind row budget, which could cut a diff box in half — leaving a
/// `┌───┐` on screen with no `└───┘` under it and no indication anything
/// had been dropped. Boxes are now sized against their budget before they
/// are drawn, so the count of top and bottom edges always matches.
///
/// Scoped to the scenes whose only box is the decision panel's: the panel
/// is a fixed band and cannot scroll, so a half-drawn box there is always a
/// defect. In the conversation log a box legitimately straddles the
/// viewport edge — a `└` with its `┌` scrolled off the top is what
/// scrolling looks like, not a bug.
#[test]
fn no_frame_leaves_a_bordered_box_unclosed() {
    const PANEL_SCENES: [&str; 5] = ["approval", "approval_large", "prompt", "prompt_path", "prompt_scoped"];
    for theme in [Theme::Dark, Theme::Light] {
        for scene_name in PANEL_SCENES {
            for (width, height) in SIZES {
                let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
                scene(scene_name, &mut app);
                let buffer = render(&mut app, width, height);
                let (mut opened, mut closed) = (0, 0);
                for y in 0..height {
                    for x in 0..width {
                        match buffer[(x, y)].symbol() {
                            "┌" => opened += 1,
                            "└" => closed += 1,
                            _ => {}
                        }
                    }
                }
                assert_eq!(opened, closed, "{theme:?} {scene_name} {width}x{height} left {opened} box(es) open but closed {closed}");
            }
        }
    }
}

// ── Grid conformance ────────────────────────────────────────────────────
//
// The snapshot above proves a render is *unchanged*. These prove it is
// *correct* — that every scene sits on the design system's grid rather than
// merely on the grid it sat on yesterday. Without them a wrong column is
// preserved as faithfully as a right one, which is how the label column
// stayed at 12 cells for as long as it did.
//
// Measured off the rendered buffer, never off the constants, per this
// project's "render it before trusting your reading of it" rule: reading
// `MARGIN_X` back out of the code and asserting it equals itself proves
// nothing about what a developer sees.

/// The design system's horizontal landmarks, in cells
/// (`tokens/cells.css`). Restated here on purpose: a test that imported
/// them from the code under test could only ever agree with it.
const MARGIN: usize = 3;
const LABEL_COL: usize = 8;
const LABEL_GUTTER: usize = 2;
const BODY_COL: usize = MARGIN + LABEL_COL + LABEL_GUTTER; // cell 13

/// Cell index of the first non-blank glyph on a row, or `None` if blank.
///
/// Counts *cells*, not bytes — `▌` is three bytes, and measuring columns
/// with `str::find` reports everything past a mark two cells right of where
/// it is.
fn first_glyph(buffer: &Buffer, y: u16) -> Option<usize> {
    (0..buffer.area.width).find(|x| buffer[(*x, y)].symbol().trim() != " " && !buffer[(*x, y)].symbol().trim().is_empty()).map(|x| x as usize)
}

fn last_glyph(buffer: &Buffer, y: u16) -> Option<usize> {
    (0..buffer.area.width).rev().find(|x| !buffer[(*x, y)].symbol().trim().is_empty()).map(|x| x as usize)
}

/// Every scene, both themes, at the design's own 120×36 frame.
fn every_scene(mut f: impl FnMut(&str, Theme, &Buffer)) {
    for theme in [Theme::Dark, Theme::Light] {
        for name in SCENES {
            let mut app = fixed_identity(App::new("claude-sonnet-5".into(), engine()).with_theme(theme));
            scene(name, &mut app);
            let buffer = render(&mut app, 120, 36);
            f(name, theme, &buffer);
        }
    }
}

/// Nothing is drawn inside the 3-cell left margin, and nothing inside the
/// 3-cell right margin.
///
/// The one deliberate exception is a selectable option row, which the
/// design runs flush to the frame's own left edge so its `▌` mark lands in
/// cell 0 ("Four option rows, flush to the frame's left edge"). That is the
/// *only* row type allowed to start before the margin, so the exception is
/// spelled as "cell 0 and the glyph is a mark" rather than as "anything
/// before cell 3".
#[test]
fn every_scene_respects_the_three_cell_margins() {
    every_scene(|name, theme, buffer| {
        for y in 0..buffer.area.height {
            let Some(first) = first_glyph(buffer, y) else { continue };
            let is_option_row = first == 0 && buffer[(0, y)].symbol() == "▌";
            assert!(
                first >= MARGIN || is_option_row,
                "{theme:?}/{name} row {y}: content starts in cell {first}, inside the 3-cell margin, and is not a flush option row"
            );
            let last = last_glyph(buffer, y).unwrap();
            let right_edge = buffer.area.width as usize - 1;
            assert!(
                last <= right_edge - MARGIN,
                "{theme:?}/{name} row {y}: content reaches cell {last}, inside the 3-cell right margin (frame ends at {right_edge})"
            );
        }
    });
}

/// The identity bar puts the harness name on the margin and the working
/// directory on the body column — the same cell 13 a transcript turn's
/// content starts on.
///
/// This one was wrong in every scene until it was measured. The bar used
/// `--group-gap`'s six cells between the name and the directory, which put
/// the directory on cell 16 — a position no token in `cells.css` names.
/// Six cells part two *unrelated* groups (`5b`'s `review changes` / `3
/// files`, the footer's key hints); the reference's own `4a`, `5a`, `5c`
/// and `5d` bars all put the cwd three cells after a seven-letter name,
/// which is the body column exactly. The prose in `HANDOFF.md` says the
/// six-cell gap "survives only between the brand and everything else",
/// which is the stale statement — the frame is the authority on positions,
/// per this project's own "measure the handoff HTML" rule.
#[test]
fn the_identity_bar_puts_the_working_directory_on_the_body_column() {
    let mut seen = 0;
    every_scene(|name, theme, buffer| {
        // Row 1 of the 3-row top bar is the content row in every scene.
        let row: String = (0..buffer.area.width).map(|x| buffer[(x, 1)].symbol()).collect();
        let Some(rest) = row.strip_prefix(&" ".repeat(MARGIN)) else { return };
        let Some(after_brand) = rest.strip_prefix("mjolnir") else { return };
        // Only scenes whose bar actually carries a directory beside the name.
        if after_brand.trim_start().starts_with(['~', '/']) {
            let start = MARGIN + "mjolnir".chars().count() + (after_brand.len() - after_brand.trim_start().len());
            assert_eq!(start, BODY_COL, "{theme:?}/{name}: the cwd starts on cell {start}, not the body column\n{row}");
            seen += 1;
        }
    });
    assert!(seen > 0, "no scene drew an identity bar with a directory — the test measured nothing");
}

/// A transcript turn puts its speaker on the margin and its content on the
/// body column. Both halves matter: the label proves the margin, and the
/// content proves the 8-cell label column plus its 2-cell gutter, which is
/// the measurement that was wrong for the longest.
#[test]
fn transcript_turns_use_the_label_column_and_the_body_column() {
    let mut seen = 0;
    every_scene(|name, theme, buffer| {
        for y in 0..buffer.area.height {
            let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<Vec<_>>().join("");
            let cells: Vec<&str> = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect();
            for label in ["you", "harness"] {
                // The label must be at the margin, and must be the row's
                // first glyph — otherwise this is prose that merely
                // contains the word.
                if first_glyph(buffer, y) != Some(MARGIN) {
                    continue;
                }
                if !cells[MARGIN..MARGIN + label.len()].concat().eq(label) {
                    continue;
                }
                seen += 1;
                let after = cells[MARGIN + label.len()..BODY_COL].concat();
                assert!(after.trim().is_empty(), "{theme:?}/{name} row {y}: the label column must be padding after {label:?}, got {after:?}");
                let body_start = (BODY_COL..buffer.area.width as usize).find(|x| !cells[*x].trim().is_empty());
                assert_eq!(
                    body_start,
                    Some(BODY_COL),
                    "{theme:?}/{name} row {y}: a {label:?} turn's content must begin on the body column, cell {BODY_COL}: {row:?}"
                );
            }
        }
    });
    assert!(seen > 0, "the scenes must actually contain transcript turns, or this test proves nothing");
}

/// The chrome bands are whole rows of one tone, and the boundaries between
/// them are tonal rather than drawn — the Turn 13 rule, asserted over every
/// scene rather than the one screen it was first checked on.
///
/// **One documented exception, which no scene here contains:** a markdown
/// table in assistant prose is drawn, per ADR 0002 — a grid of boundaries
/// repeated down every row is the one thing a one-dimensional ground ladder
/// cannot express. Adding a table scene to `SCENES` will therefore trip the
/// box-drawing assertion below, and the fix is to exempt that scene, not to
/// un-draw the table.
#[test]
fn every_scene_parts_its_bands_by_tone_and_draws_no_rules() {
    every_scene(|name, theme, buffer| {
        // The top bar is three rows of one colour, and the row under it is
        // a different one.
        let bar = buffer[(0, 0)].bg;
        for y in 0..3u16 {
            assert_eq!(buffer[(0, y)].bg, bar, "{theme:?}/{name}: the top bar is 3 rows of one tone");
        }
        assert_ne!(buffer[(0, 3)].bg, bar, "{theme:?}/{name}: the band below the top bar must differ in tone — that step is the boundary");

        // Nothing anywhere is stroked, and no cell carries an underline
        // standing in for a border.
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                assert!(
                    !"─│┌┐└┘├┤┬┴┼╭╮╰╯━┃║╔╗╚╝▁▔".contains(cell.symbol()),
                    "{theme:?}/{name} at {x},{y}: {:?} is a box-drawing glyph — nothing inside a frame is stroked",
                    cell.symbol()
                );

            }

            // A border drawn as a cell attribute: a whole row of blank
            // cells carrying an underline, which is exactly the shape the
            // bars used before Turn 13. Asserted per row rather than per
            // cell, because an underlined *space inside a markdown heading*
            // is legitimate text styling, not a rule.
            let underlined_blanks = (0..buffer.area.width)
                .filter(|x| {
                    let cell = &buffer[(*x, y)];
                    cell.modifier.contains(ratatui::style::Modifier::UNDERLINED) && cell.symbol().trim().is_empty()
                })
                .count();
            assert!(
                underlined_blanks < buffer.area.width as usize / 2,
                "{theme:?}/{name} row {y}: {underlined_blanks} blank underlined cells — that is a border drawn as an attribute"
            );
        }
    });
}

/// Scrollbars are listed under "Deliberately absent" in the design system,
/// beside tabs, breadcrumbs and "any control that needs a mouse". One used
/// to render down the right edge whenever the transcript overflowed — the
/// single element in the frame that sat outside the right margin.
#[test]
fn no_scene_draws_a_scrollbar() {
    every_scene(|name, theme, buffer| {
        let right = buffer.area.width - 1;
        for y in 0..buffer.area.height {
            let symbol = buffer[(right, y)].symbol();
            assert!(
                symbol.trim().is_empty(),
                "{theme:?}/{name} row {y}: {symbol:?} in the frame's last column — a scrollbar track is deliberately absent"
            );
        }
    });
}
