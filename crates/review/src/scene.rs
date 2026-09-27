//! Scenes — the states the binary is put into before capture.
//!
//! A scene is a seeded `HOME`/`.aldwin` config, a queue of canned provider
//! replies, and the keys to type. The real binary then plays it out: real
//! HTTP, real streaming adapter, real core loop, real tools, real TUI.
//! Nothing is poked into the UI's memory, which is the difference between
//! this and `render_snapshot.rs` — and the reason a scene here can catch
//! something the snapshot cannot: the review below is opened by the real
//! dispatcher over a really staged edit.
//!
//! The catalogue names the same scenes as `render_snapshot.rs`, so every
//! scene is both pinned by the snapshot and capturable: stage 8 judges a
//! scene only when its snapshot moved, and a name missing from either list
//! is a scene no change can call a judge for. A scene here reaches the
//! snapshot's state through the real path — the words and files are the
//! fake's, the state is the same — and one that cannot is reshaped or
//! dropped **with a note here**, never quietly pointed at a different state
//! under the same name.
//!
//! Two kinds of state took more than typing. A turn still running
//! (`working`, `running`) is held open by a reply that never ends
//! (`fake::held`), since the fake otherwise answers at once and the turn is
//! over before the frame is taken. A selection in the review (`selecting`,
//! `commented`) is made with `Shift ↓` and the arrows, the keyboard's way to
//! the same selection a drag leaves (ADR 0010).

use std::path::{Path, PathBuf};

use crate::fake::{self, Canned};
use crate::geometry::Theme;
use crate::{Error, Result};

/// The static half of a scene: what it needs seeded and what it will be told.
#[derive(Debug)]
pub struct Script {
    /// One per request the scene makes, in order.
    pub replies: Vec<Canned>,
    /// Files the scene needs in the project before it runs — a `read` needs
    /// something to read, an `edit` needs its `before` text to exist exactly
    /// once.
    pub files: Vec<(&'static str, &'static str)>,
    /// Past sessions to seed into this project's history directory, as
    /// `(first user message, started_at, turns)`. Written through the
    /// product's own `HistoryStore`, same rule as the global config: a copy
    /// of the JSONL format here would drift from the one being reviewed.
    pub history: &'static [(&'static str, u64, usize)],
    /// Typed once the app has settled.
    pub keys: &'static str,
    /// Written only when the scene wants a provider configured at all;
    /// `launch_unconfigured` is defined by its absence.
    pub provider: bool,
}

/// A scene seeded on disk and ready to launch the app into.
#[derive(Debug)]
pub struct Prepared {
    /// The project directory the app is started in, canonical.
    pub cwd: PathBuf,
    /// The `HOME` the app is started with, holding the seeded `~/.aldwin`.
    pub home: PathBuf,
    /// The scene's keys, parsed to the bytes each one writes.
    pub keys: Vec<Vec<u8>>,
}

/// Every scene [`script`] knows, by name.
pub const CATALOGUE: &[&str] = &[
    "launch",
    "launch_unconfigured",
    "plan",
    "details",
    "question",
    "commands",
    "review",
    "saved",
    "markdown",
    "failure",
    "long",
    "resume",
    "working",
    "running",
    "selecting",
    "commented",
    "wrapped",
    "stopping",
    "answering",
];

const ROUTER: &str = "src/gateway/router.rs";

const ROUTER_RS: &str = "pub fn app(cfg: &Config) -> Router {\n    Router::new()\n        .route(\"/v1/chat\", post(chat))\n        .layer(auth_layer(cfg))\n        .layer(TraceLayer::new_for_http())\n}\n";

const MOD: &str = "src/gateway/mod.rs";

const MOD_RS: &str = "mod router;\n\npub use router::app;\n";

const LIMIT: &str = "src/gateway/limit.rs";

const PROSE: &str = "Looking at how requests move through the gateway. Every request passes auth and tracing and nothing counts them, so a limit belongs beside the auth layer where the key is already known.";

const TABLE: &str = "Three providers are configured here:\n\n| provider | key variable | streaming |\n| --- | --- | --- |\n| anthropic | ANTHROPIC_API_KEY | yes |\n| openai | OPENAI_API_KEY | yes |\n| google | GOOGLE_API_KEY | no |\n\n- `google` has no streaming yet\n- the others stream\n\n> A key variable must be exported before launch.\n\nThe default is set in `provider.yaml`:\n\n```yaml\nprovider: anthropic\nmodel: claude-sonnet-5\n```";

/// `⌃↩` under the Kitty keyboard protocol, which the app pushes at startup
/// (`CSI > 1 u`), so foot reports Enter with the control modifier as
/// `CSI 13 ; 5 u` rather than as a bare `\r`.
const CTRL_ENTER: &str = "\"\\e[13;5u\"";

fn plan(states: [&str; 3]) -> serde_json::Value {
    let texts = [
        "Count requests per key",
        "Turn away requests over the limit",
        "Check that it works",
    ];
    serde_json::json!({ "steps": texts.iter().zip(states).map(|(t, s)| serde_json::json!({ "text": t, "state": s })).collect::<Vec<_>>() })
}

/// A turn that asks whether requests without a key are limited too.
fn question() -> Vec<Canned> {
    vec![fake::tool_call(
        "call-ask",
        "ask",
        serde_json::json!({
            "question": "Should requests without an API key be limited too?",
            "detail":   "Right now they skip the limit. Limiting them by address stops anonymous floods.",
            "options":  ["Yes, limit them by address", "No, let them through"],
        }),
    )]
}

/// A turn that stages one edit and creates one file, then says what it did.
fn review() -> Vec<Canned> {
    vec![
        fake::tool_calls(&[
            ("call-plan", "plan", plan(["done", "running", "pending"])),
            (
                "call-edit",
                "edit",
                serde_json::json!({
                    "path":   ROUTER,
                    "before": "        .layer(auth_layer(cfg))\n",
                    "after":  "        .layer(RateLimitLayer::new(\n            Quota::per_minute(100),\n            cfg.limit_store.clone(),\n        ))\n        .layer(auth_layer(cfg))\n",
                }),
            ),
            (
                "call-new",
                "edit",
                serde_json::json!({ "path": LIMIT, "before": "", "after": "pub struct Limit { per_minute: u32, store: Arc<dyn LimitStore> }\n" }),
            ),
        ]),
        fake::text("Each key gets 100 requests a minute; the rest are turned away before auth."),
    ]
}

/// A turn on its last step, held open while the model is still saying so.
fn running() -> Vec<Canned> {
    vec![
        fake::said_then_calls(
            "The limit is in place. Checking that it works.",
            &[("call-plan", "plan", plan(["done", "done", "running"]))],
        ),
        fake::held("Running the tests. About ten seconds."),
    ]
}

/// The script for the scene called `name`.
///
/// # Errors
///
/// When `name` is not in [`CATALOGUE`].
pub fn script(name: &str) -> Result<Script> {
    if !CATALOGUE.contains(&name) {
        return Err(Error::Scene(format!(
            "unknown scene {name:?}; see scene::CATALOGUE"
        )));
    }

    let ask = "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter";
    let edit = || {
        fake::tool_call(
            "call-edit",
            "edit",
            serde_json::json!({
                "path":   ROUTER,
                "before": "        .layer(auth_layer(cfg))\n",
                "after":  "        .layer(RateLimitLayer::new(\n            Quota::per_minute(100),\n            cfg.limit_store.clone(),\n        ))\n        .layer(auth_layer(cfg))\n",
            }),
        )
    };
    Ok(match name {
        // Every launch, the first included: the card and the field.
        "launch" => Script { replies: vec![], history: &[], files: vec![], keys: "", provider: true },

        // Nothing configured: the card reads `Model  not set`. There is no
        // first-run screen (ADR 0009 §6).
        "launch_unconfigured" => Script { replies: vec![], history: &[], files: vec![], keys: "", provider: false },

        // The plan and a collapsed disclosure, as a finished turn leaves them.
        "plan" => Script {
            replies: vec![
                fake::tool_calls(&[("call-plan", "plan", plan(["running", "pending", "pending"])), ("call-read", "read", serde_json::json!({ "path": ROUTER }))]),
                fake::tool_call("call-plan-2", "plan", plan(["done", "running", "pending"])),
                fake::text(PROSE),
            ],
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    ask,
            provider: true,
        },

        // The same, with the disclosure opened by Space.
        "details" => Script {
            replies: vec![
                fake::tool_calls(&[("call-plan", "plan", plan(["running", "pending", "pending"])), ("call-read", "read", serde_json::json!({ "path": ROUTER }))]),
                fake::tool_call("call-plan-2", "plan", plan(["done", "running", "pending"])),
                fake::text(PROSE),
            ],
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Space",
            provider: true,
        },

        // The `ask` tool: the panel takes the band with the three answers.
        "question" => Script { replies: question(), history: &[], files: vec![], keys: ask, provider: true },

        // Frame F types `/c`: the list narrowed, the field completed in grey.
        "commands" => Script { replies: vec![], history: &[], files: vec![], keys: "\"/\",\"c\"", provider: true },

        // The review, opened by the real dispatcher at the end of a turn
        // that staged one edit and created one file (ADR 0009 §4).
        "review" => Script { replies: review(), history: &[], files: vec![(ROUTER, ROUTER_RS)], keys: ask, provider: true },

        // The router's first two added lines selected, and a comment typed
        // against them: `Tab` to the router, `Shift ↓` selects the top row
        // shown, the arrows carry it to the first added line and `Shift ↓`
        // extends it by one.
        "selecting" => Script {
            replies: review(),
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Tab,ShiftDown,Down,Down,ShiftDown,\"Read the limit from config, not 100.\"",
            provider: true,
        },

        // The comment added: it rides on the lines, and the field closes.
        "commented" => Script {
            replies: review(),
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Tab,ShiftDown,Down,Down,ShiftDown,\"Read the limit from config, not 100.\",Enter",
            provider: true,
        },

        // A diff line wider than the pane wraps (baseline
        // `long-diff-lines-wrap`).
        "wrapped" => Script {
            replies: vec![
                fake::tool_call("call-edit", "edit", serde_json::json!({
                    "path":   LIMIT,
                    "before": "pub struct Limit;\n",
                    "after":  "pub struct Limit;\n\nconst MESSAGE: &str = \"This key has made more than its 100 requests this minute; wait for the next minute, or ask for a higher limit.\";\n",
                })),
                fake::text("Requests over the limit are told why and when to try again."),
            ],
            history: &[],
            files:   vec![(LIMIT, "pub struct Limit;\n")],
            keys:    ask,
            provider: true,
        },

        // A turn in flight: its prose, its work folded, the plan with a
        // step running, and the model still answering.
        "working" => Script {
            replies: vec![
                fake::said_then_calls("Looking at how requests move through the gateway.", &[
                    ("call-read-mod", "read", serde_json::json!({ "path": MOD })),
                    ("call-read-router", "read", serde_json::json!({ "path": ROUTER })),
                ]),
                fake::said_then_calls("Nothing limits requests yet. Adding a limit for each key.", &[("call-plan", "plan", plan(["done", "running", "pending"]))]),
                fake::held(""),
            ],
            history: &[],
            files:   vec![(MOD, MOD_RS), (ROUTER, ROUTER_RS)],
            keys:    ask,
            provider: true,
        },

        // The last step running, and the model saying what it is doing.
        "running" => Script { replies: running(), history: &[], files: vec![], keys: ask, provider: true },

        // `esc` mid-turn: stopped, and nothing else.
        "stopping" => Script {
            replies: running(),
            history: &[],
            files: vec![],
            keys: "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Esc",
            provider: true,
        },

        // "Chat about this": the question stays, the turn waits on you,
        // and what is typed is the answer.
        "answering" => Script {
            replies: question(),
            history: &[],
            files:   vec![],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,\"3\",\"Only the ones\"",
            provider: true,
        },

        // Approved: the review folds into the one row the conversation keeps.
        "saved" => Script {
            replies: vec![edit(), fake::text("Each key gets 100 requests a minute; the rest are turned away before auth.")],
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,{CTRL_ENTER}",
            provider: true,
        },

        "markdown" => Script { replies: vec![fake::text(TABLE)], history: &[], files: vec![], keys: "\"which providers are set up?\",Enter", provider: true },

        // A turn that did not finish: the provider answered with an error
        // the client does not retry. A sentence in `label`, its detail one
        // disclosure below — no red (ADR 0009 §5).
        "failure" => Script {
            replies: vec![Canned::Status(400, "{\"error\":{\"message\":\"the request was malformed: unknown model gpt-5\"}}".into())],
            history: &[],
            files:   vec![],
            keys:    ask,
            provider: true,
        },

        // Long enough to fill the tallest frame and scroll the shortest.
        "long" => Script {
            replies: vec![fake::text(&format!("{PROSE}\n\n{TABLE}\n\n{PROSE}\n\n{PROSE}"))],
            history: &[],
            files:   vec![],
            keys:    ask,
            provider: true,
        },

        // Bare `/resume`: the session question over two past sessions, so
        // the list is a list.
        "resume" => Script {
            replies: vec![],
            history: &[("how should the retry loop back off?", 1_789_732_800, 4), ("which providers are set up?", 1_789_819_200, 1)],
            files:   vec![],
            keys:    "\"/\",\"r\",Enter",
            provider: true,
        },

        _ => unreachable!("guarded by CATALOGUE above"),
    })
}

/// Write the config a scene runs under, and parse its keys.
///
/// # Errors
///
/// When a directory or file cannot be created or written, the product's own
/// config or history writer fails, or the scene's keys do not parse.
pub fn seed(script: &Script, theme: Theme, root: &Path, endpoint: &str) -> Result<Prepared> {
    // Absolute, always. `HOME` is resolved by the app against its own working
    // directory, so a relative one sends it looking for `~/.aldwin` inside
    // the project.
    let home = root.join("home");
    let cwd = home.join("proj");
    std::fs::create_dir_all(&cwd)?;
    let home = home.canonicalize()?;
    let cwd = cwd.canonicalize()?;

    // `~/.aldwin` is materialised by the product's own writer rather than a
    // copy of its templates here.
    let global = home.join(".aldwin");
    let config = aldwin_config::Config::open_at(&cwd, &global)
        .map_err(|e| Error::Scene(format!("seeding global config: {e}")))?;
    config
        .init_global_if_empty()
        .map_err(|e| Error::Scene(format!("seeding global config: {e}")))?;

    // Theme is global-only and read once at startup, so it is seeded as config
    // rather than sent as a command.
    std::fs::write(
        global.join("tui.yaml"),
        format!("version: 1\ntheme: {theme}\n"),
    )?;

    if script.provider {
        // The endpoint carries an ephemeral port, so this is written per run.
        // `openai-compatible` is not a preference: `base_url` is ignored for
        // the anthropic provider, and a local fake is nothing but a base_url.
        std::fs::write(
            global.join("provider.yaml"),
            format!("version: 1\nprovider: openai-compatible\nmodel: gpt-5\nbase_url: {endpoint}\napi_key_env: ALDWIN_SHOT_KEY\n"),
        )?;
    }

    seed_history(script, &global, &cwd)?;

    for (path, contents) in &script.files {
        let file = cwd.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, contents)?;
    }

    let keys = crate::keys::parse(&script.keys.replace("{CTRL_ENTER}", CTRL_ENTER))?;
    Ok(Prepared { cwd, home, keys })
}

/// Write this scene's past sessions, through the product's own writer.
fn seed_history(script: &Script, global: &Path, cwd: &Path) -> Result<()> {
    if script.history.is_empty() {
        return Ok(());
    }
    use aldwin_config::{HistoryStore, SessionHeader, HISTORY_VERSION};
    use aldwin_core::{LogRecord, SessionId, StepId, TurnEndReason, TurnId};

    let dir = aldwin_config::history_project_dir(&global.join("history"), cwd);
    for (index, (text, started_at, turns)) in script.history.iter().enumerate() {
        let id = SessionId(format!("{started_at:010}-0-{index}"));
        let header = SessionHeader {
            version: HISTORY_VERSION,
            started_at: *started_at,
            cwd: cwd.to_string_lossy().into_owned(),
            model: "gpt-5".into(),
        };
        let store = HistoryStore::create(&dir, &id, &header)
            .map_err(|e| Error::Scene(format!("seeding history: {e}")))?;
        for turn in 0..*turns {
            let turn_id = TurnId(turn as u64 + 1);
            let step_id = StepId(turn as u64 + 1);
            let user = if turn == 0 {
                (*text).to_string()
            } else {
                format!("and then? ({turn})")
            };
            for record in [
                LogRecord::TurnStarted { turn_id },
                LogRecord::UserMessage {
                    turn_id,
                    text: user,
                },
                LogRecord::AssistantMessage {
                    turn_id,
                    step_id,
                    text: PROSE.into(),
                },
                LogRecord::TurnEnded {
                    turn_id,
                    reason: TurnEndReason::EndTurn,
                },
            ] {
                store
                    .append(&record)
                    .map_err(|e| Error::Scene(format!("seeding history: {e}")))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{script, CATALOGUE, CTRL_ENTER};

    /// Stage 8 judges a scene only when its snapshot section moved and
    /// capture can draw it, so a name on one list and not the other is a
    /// scene no change can call a judge for.
    #[test]
    fn the_catalogue_names_the_snapshots_scenes() {
        let snapshot = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tui/tests/snapshots/render.snap"
        ))
        .expect("render.snap is committed");
        let pinned: BTreeSet<&str> = snapshot.lines().filter_map(crate::git::scene_of).collect();
        let captured: BTreeSet<&str> = CATALOGUE.iter().copied().collect();
        assert_eq!(captured, pinned);
    }

    #[test]
    fn every_scene_has_a_script_whose_keys_parse() {
        for name in CATALOGUE {
            let keys = script(name).expect("in the catalogue").keys;
            crate::keys::parse(&keys.replace("{CTRL_ENTER}", CTRL_ENTER))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}
