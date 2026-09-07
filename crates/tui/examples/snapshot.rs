//! Design-iteration harness — NOT part of the shipped app, and a companion
//! to `examples/preview.rs` rather than a replacement for it.
//!
//! `preview.rs` draws one scene to a *real* terminal and waits, so capturing
//! it needs a driving script and a live tmux/terminal session. This one
//! draws every scene into a `TestBackend` and writes each frame out as a
//! self-contained HTML page — one `<span>` run per styled cell run, on an
//! absolutely-positioned 9x20px cell grid matching `tokens/cells.css`. A
//! headless Chromium screenshot of that page can then be compared
//! side-by-side with a render of the design system's own handoff HTML, with
//! no terminal, no tmux session and no interactive step anywhere in the
//! loop.
//!
//! Two rendering choices exist to make defects *visible* rather than
//! plausible-looking:
//!
//! * `Color::Reset` — a cell whose colour the app never set, so the real
//!   terminal paints its own — renders as magenta. Every such cell is a hole
//!   in the frame's own palette, and the whole point of the exercise is to
//!   find them.
//! * Indexed/named ANSI colours render as their xterm defaults, but are
//!   equally suspect: the design system is authored in truecolor hex, so
//!   anything here that isn't `Color::Rgb` is drawing outside it.
//!
//! ```text
//! cargo run -p mjolnir-tui --example snapshot -- /tmp/out [cols] [rows]
//! ```

use std::io;
use std::sync::Arc;

use mjolnir_config::Config;
use mjolnir_permissions::{Engine, PromptPayload};
use mjolnir_tui::{App, LogEntry, ModelChoice, PromptResolution, ProviderChoice, Theme, ToolActivityEntry, ToolActivityStatus};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

/// Scenes drawn through `App` — everything with a transcript behind it.
/// `empty` is the design system's `14d`, the state a returning developer
/// opens into and the one `/clear` returns them to.
const SCENES: [&str; 7] = ["empty", "conversation", "tools", "approval", "prompt", "resolved", "long"];

/// First run's three steps (`14a`, `14b`, `14c`). Drawn from `FirstRun`
/// rather than `App` — the screen runs its own terminal loop, so it has its
/// own draw entry point (`__preview_draw_first_run`). The index is which
/// step is open; the spine shows all three either way, which is exactly the
/// thing worth screenshotting.
const FIRST_RUN_STEPS: [&str; 3] = ["first-run-provider", "first-run-model", "first-run-access"];

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| ".".into());
    let cols: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(120);
    let rows: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(36);
    std::fs::create_dir_all(&out_dir)?;

    for theme in [Theme::Dark, Theme::Light] {
        let suffix = if matches!(theme, Theme::Light) { "light" } else { "dark" };

        for scene_name in SCENES {
            let dir = tempfile::tempdir().unwrap();
            let config = Config::open_at(dir.path(), dir.path().join("global")).unwrap();
            let engine = Arc::new(Engine::new(config));
            let mut app = App::new("claude-sonnet-5".into(), engine).with_theme(theme);
            scene(scene_name, &mut app);

            let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
            terminal.draw(|f| mjolnir_tui::__preview_draw(f, &mut app)).unwrap();

            let path = format!("{out_dir}/{scene_name}-{suffix}.html");
            std::fs::write(&path, page(terminal.backend().buffer(), scene_name, suffix))?;
            println!("{path}");
        }

        for (index, name) in FIRST_RUN_STEPS.iter().enumerate() {
            let state = first_run_state(index);
            let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
            terminal.draw(|f| mjolnir_tui::__preview_draw_first_run(f, &state, theme)).unwrap();

            let path = format!("{out_dir}/{name}-{suffix}.html");
            std::fs::write(&path, page(terminal.backend().buffer(), name, suffix))?;
            println!("{path}");
        }
    }
    Ok(())
}

/// A first-run screen with `index` as the open step, on a catalogue shaped
/// like the real one — three curated rows behind a `more`, each with its own
/// models. Deliberately not `mjolnir-llm`'s actual catalogue: this example
/// does not depend on that crate, and a screenshot harness that did would
/// change every time a provider was added.
fn first_run_state(index: usize) -> mjolnir_tui::__PreviewFirstRun {
    // Listed deepest-first so the purposes below zip onto the right rows —
    // the same order, and the same copy, as the reference's `14b`.
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

    let mut state = mjolnir_tui::__PreviewFirstRun::new(providers, 3, true, true);
    state.index = index;
    state
}

/// One frame as a standalone HTML page: an absolutely-positioned grid of
/// styled runs on the same 9x20px cell the design system's mocks use, so a
/// screenshot of this lines up cell-for-cell with a screenshot of theirs.
fn page(buf: &Buffer, scene_name: &str, theme: &str) -> String {
    let (w, h) = (buf.area.width, buf.area.height);
    // The desk the frame sits on — `--tui-scrim` for this theme, read from
    // the palette rather than copied. It was a fixed near-black, which put
    // every *light* frame on a dark desk: the one surface a reviewer uses
    // to judge whether the light theme's chrome bands are stepping the
    // right way was showing the wrong theme's ground. It was then a pair of
    // hex literals, which went stale the moment the light ladder was
    // regenerated — the desk sat two turns behind at `#a39fac` while the
    // token said `#cfcad9`, and nothing could catch it, because a literal
    // agrees with itself.
    let desk = mjolnir_tui::__preview_scrim_hex(if theme == "light" { Theme::Light } else { Theme::Dark });
    let mut out = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{scene_name} {theme}</title><style>\
         body{{margin:0;background:{desk};padding:20px}}\
         .f{{position:relative;width:{}px;height:{}px;font:400 15px/20px 'DejaVu Sans Mono','Liberation Mono',monospace;white-space:pre;overflow:hidden}}\
         .f i{{position:absolute;font-style:normal;height:20px}}\
         </style></head><body><div class=\"f\">",
        w as usize * 9,
        h as usize * 20
    );

    for y in 0..h {
        let mut x = 0u16;
        while x < w {
            let first = &buf[(x, y)];
            let (fg, bg, modifier, ul) = (first.fg, first.bg, first.modifier, first.underline_color);
            let start = x;
            let mut text = String::new();
            while x < w {
                let cell = &buf[(x, y)];
                if cell.fg != fg || cell.bg != bg || cell.modifier != modifier || cell.underline_color != ul {
                    break;
                }
                text.push_str(cell.symbol());
                x += 1;
            }
            let mut style = format!("left:{}px;top:{}px;color:{};background:{}", start as usize * 9, y as usize * 20, css(fg), css(bg));
            if modifier.contains(Modifier::BOLD) {
                style.push_str(";font-weight:700");
            }
            if modifier.contains(Modifier::ITALIC) {
                style.push_str(";font-style:italic");
            }
            if modifier.contains(Modifier::DIM) {
                style.push_str(";opacity:.6");
            }
            // A cell attribute, not a glyph — the band edges are drawn with
            // it, so it has to survive into the screenshot or the thing
            // being reviewed is invisible. `underline_color` falls back to
            // the cell's foreground, exactly as a terminal without SGR 58
            // does.
            if modifier.contains(Modifier::UNDERLINED) {
                let color = if matches!(ul, Color::Reset) { css(fg) } else { css(ul) };
                style.push_str(&format!(";text-decoration:underline;text-decoration-color:{color};text-decoration-thickness:1px;text-underline-offset:4px"));
            }
            out.push_str(&format!("<i style=\"{style}\">{}</i>", escape(&text)));
        }
    }
    out.push_str("</div></body></html>");
    out
}

/// A ratatui `Color` as a CSS colour. `Reset` deliberately renders as
/// magenta rather than as anything plausible — see this file's module doc.
fn css(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => format!("/*{other:?}*/#ff00ff"),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn scene(name: &str, app: &mut App) {
    match name {
        // `14d` needs no log entries at all — an empty log *is* the scene.
        // It does need a named provider, since the row that falls back to
        // the model alone is the degraded case, not the one to review.
        "empty" => app.current_provider = Some("anthropic".into()),
        "conversation" => conversation(app),
        "tools" => tools(app),
        "approval" => approval(app),
        "prompt" => prompt(app),
        "resolved" => resolved(app),
        "long" => long(app),
        other => panic!("unknown scene {other:?}"),
    }
}

fn conversation(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "Can you refactor the retry logic in llm/src/client.rs to use exponential backoff?".into() });
    app.log.push(LogEntry::AssistantText {
        text: "Sure — here's the plan:\n\n1. Add a `backoff_ms` helper\n2. Wire it into the retry loop\n3. Cap at **5** attempts\n\n```rust\nfn backoff_ms(attempt: u32) -> u64 {\n    100 * 2u64.pow(attempt)\n}\n```\n\nThat gives `100ms, 200ms, 400ms, ...`.".into(),
    });
    app.log.push(LogEntry::AssistantText {
        text: "Here is the change:\n\n```diff\n--- a/src/retry.rs\n+++ b/src/retry.rs\n unchanged context line\n-    let delay = 100;\n+    let delay = backoff_ms(attempt);\n     sleep(delay).await;\n```".into(),
    });
    app.status.turn = Some(3);
    app.status.step = Some(2);
}

fn tools(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "grep for TODO across the repo and summarize".into() });
    app.log.push(LogEntry::ToolActivity {
        step_id: mjolnir_core::StepId(1),
        calls: vec![
            ToolActivityEntry { call_id: "c1".into(), name: "shell".into(), status: ToolActivityStatus::Completed { is_error: false, summary: "42 matches across 17 files".into() } },
            ToolActivityEntry { call_id: "c2".into(), name: "read".into(), status: ToolActivityStatus::Running },
        ],
    });
    app.turn_active = true;
    app.status.running_tools = vec![mjolnir_tui::__PreviewRunningTool { call_id: "c2".into(), name: "read".into() }];
    app.status.turn = Some(4);
    app.status.step = Some(1);
}

const DIFF: &str = "--- a/src/page.rs\n+++ b/src/page.rs\n@@\n fn page(items: &[Item], size: usize, n: usize) -> &[Item] {\n     let start = n * size;\n-    let end = start + size;\n+    let end = (start + size).min(items.len());\n     &items[start..end]\n }\n";

fn approval(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "fix the off-by-one in the pagination helper".into() });
    app.log.push(LogEntry::ApprovalCard { call_id: "call-1".into(), diff: DIFF.into(), resolution: None });
    app.pending_approvals.push_back(mjolnir_tui::__PreviewPendingApproval { call_id: "call-1".into(), diff: DIFF.into() });
}

fn prompt(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "run the test suite".into() });
    let payload = PromptPayload::Tool { kind: "shell".into(), target: "cargo test --workspace".into(), path_like: false };
    app.log.push(LogEntry::PermissionPrompt { call_id: "call-2".into(), payload: payload.clone(), resolution: None });
    app.pending_prompts.push_back(mjolnir_tui::__PreviewPendingPrompt { call_id: "call-2".into(), payload });
}

/// The scene the developer reported as "misaligned and wonky": a *resolved*
/// permission prompt and a resolved Edit approval sitting inline in the log,
/// which is where both leave their permanent record.
fn resolved(app: &mut App) {
    app.log.push(LogEntry::UserMessage { text: "make me an empty html file in Downloads".into() });
    app.log.push(LogEntry::PermissionPrompt {
        call_id: "call-4".into(),
        payload: PromptPayload::Tool { kind: "shell".into(), target: "touch ~/Downloads/hello.html".into(), path_like: false },
        resolution: Some(PromptResolution { allowed: true, label: "allowed once".into() }),
    });
    app.log.push(LogEntry::ApprovalCard { call_id: "call-5".into(), diff: DIFF.into(), resolution: Some(true) });
    app.status.turn = Some(2);
    app.status.step = Some(1);
}

fn long(app: &mut App) {
    for i in 0..8 {
        app.log.push(LogEntry::UserMessage { text: format!("message {i}") });
        app.log.push(LogEntry::AssistantText { text: format!("reply {i} with a bit more text to see wrapping behavior across the pane width") });
    }
    app.log.push(LogEntry::Error { message: "provider returned 529 overloaded".into() });
    app.status.turn = Some(9);
    app.status.step = Some(3);
}
