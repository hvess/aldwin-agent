//! Scenes: the states the real binary is driven into before capture, each a
//! seeded `HOME`/`.aldwin` config, a queue of canned replies, and keys.
//!
//! [`CATALOGUE`] must name exactly the scenes of `render_snapshot.rs`
//! (`the_catalogue_names_the_snapshots_scenes`). Each scene reaches its
//! snapshot's state through the real path; one that cannot is reshaped or
//! dropped with a note here, never pointed at a different state under the
//! same name.
//!
//! A running turn (`working`, `running`, `sent`) is held open by
//! `fake::held`. A review selection (`selecting`, `commented`, `sent`) uses
//! `Shift ↓` (ADR 0010).

use std::path::{Path, PathBuf};

use crate::fake::{self, Canned};
use crate::geometry::Theme;
use crate::{Error, Result};

/// What a scene seeds and replies, before it is written to disk.
#[derive(Debug)]
pub struct Script {
    /// One per request the scene makes, in order.
    pub replies: Vec<Canned>,
    /// `(path, contents)` seeded in the project. An `edit`'s `before` must
    /// occur exactly once in its file.
    pub files: Vec<(&'static str, &'static str)>,
    /// Past sessions as `(first user message, started_at, turns)`, written
    /// through `HistoryStore`, never a copy of its format.
    pub history: &'static [(&'static str, u64, usize)],
    /// A [`crate::keys::parse`] spec, typed once the app has settled.
    pub keys: &'static str,
    /// Whether `provider.yaml` is written; false only for `launch_unconfigured`.
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
    "sent",
    "asked",
    "wrapped",
    "stopping",
    "answering",
    "thinking",
];

const ROUTER: &str = "src/gateway/router.rs";

const ROUTER_RS: &str = "pub fn app(cfg: &Config) -> Router {\n    Router::new()\n        .route(\"/v1/chat\", post(chat))\n        .layer(auth_layer(cfg))\n        .layer(TraceLayer::new_for_http())\n}\n";

const MOD: &str = "src/gateway/mod.rs";

const MOD_RS: &str = "mod router;\n\npub use router::app;\n";

const LIMIT: &str = "src/gateway/limit.rs";

const PROSE: &str = "Looking at how requests move through the gateway. Every request passes auth and tracing and nothing counts them, so a limit belongs beside the auth layer where the key is already known.";

const TABLE: &str = "Three providers are configured here:\n\n| provider | key variable | streaming |\n| --- | --- | --- |\n| anthropic | `ANTHROPIC_API_KEY` | yes |\n| openai | `OPENAI_API_KEY` | yes |\n| google | `GOOGLE_API_KEY` | no |\n\n- `google` has no streaming yet\n- the others stream\n\n> A key variable must be exported before launch.\n\nThe default is set in `provider.yaml`:\n\n```yaml\nprovider: anthropic\nmodel: claude-sonnet-5\n```";

/// `⌃↩` as foot reports it under the Kitty keyboard flag the app pushes at
/// startup (`CSI > 1 u`): `CSI 13 ; 5 u`, not `\r`.
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
        // The launch card and the field.
        "launch" => Script { replies: vec![], history: &[], files: vec![], keys: "", provider: true },

        // No provider: the card reads `Model  not set`; no first-run screen
        // (ADR 0009 §6).
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

        // The `ask` tool's question panel.
        "question" => Script { replies: question(), history: &[], files: vec![], keys: ask, provider: true },

        // Frame F types `/c`: the list narrowed, the field completed in grey.
        "commands" => Script { replies: vec![], history: &[], files: vec![], keys: "\"/\",\"c\"", provider: true },

        // The review at the end of a turn that staged one edit and one new
        // file (ADR 0009 §4).
        "review" => Script { replies: review(), history: &[], files: vec![(ROUTER, ROUTER_RS)], keys: ask, provider: true },

        // The router's first two added lines selected, a comment typed:
        // `Tab` to the router, `Shift ↓` selects the top row, the arrows move
        // it to the first added line, `Shift ↓` extends it by one.
        "selecting" => Script {
            replies: review(),
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Tab,ShiftDown,Down,Down,ShiftDown,\"Read the limit from config, not 100.\"",
            provider: true,
        },

        // The comment added to the lines, the field closed.
        "commented" => Script {
            replies: review(),
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Tab,ShiftDown,Down,Down,ShiftDown,\"Read the limit from config, not 100.\",Enter",
            provider: true,
        },

        // The comment sent with `⌃↩`: the review stays open and waits inside
        // the follow-up turn, which the model never finishes.
        "sent" => Script {
            replies: review().into_iter().chain([fake::held("")]).collect(),
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Tab,ShiftDown,Down,Down,ShiftDown,\"Read the limit from config, not 100.\",Enter,{CTRL_ENTER}",
            provider: true,
        },

        // `sent`, but the follow-up turn asks: the question in the waiting
        // review's bottom band, the review kept under it.
        "asked" => Script {
            replies: review().into_iter().chain(question()).collect(),
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Tab,ShiftDown,Down,Down,ShiftDown,\"Read the limit from config, not 100.\",Enter,{CTRL_ENTER}",
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

        // A turn in flight: prose, folded work, a plan step running, the
        // model still answering.
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

        // `Esc` mid-turn.
        "stopping" => Script {
            replies: running(),
            history: &[],
            files: vec![],
            keys: "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Esc",
            provider: true,
        },

        // Option 3, "Chat about this": the question stays and the typed text
        // is the answer.
        "answering" => Script {
            replies: question(),
            history: &[],
            files:   vec![],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,\"3\",\"Only the ones\"",
            provider: true,
        },

        // A reply the model reasoned before, the thought opened by Space
        // (ADR 0015). The fake answers at once: `Thought for 1s`.
        "thinking" => Script {
            replies: vec![fake::thought_then_text("The request is a limit per API key. The router adds auth and then tracing, and the key is only known after auth, so the limit goes right after it. The quota should come from config rather than a literal.", "A limit fits beside the auth layer, where the key is already known.")],
            history: &[],
            files:   vec![],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,Space",
            provider: true,
        },

        // Approved: the review folds into one conversation row.
        "saved" => Script {
            replies: vec![edit(), fake::text("Each key gets 100 requests a minute; the rest are turned away before auth.")],
            history: &[],
            files:   vec![(ROUTER, ROUTER_RS)],
            keys:    "\"Add rate limiting to the gateway. 100 requests a minute per API key.\",Enter,{CTRL_ENTER}",
            provider: true,
        },

        "markdown" => Script { replies: vec![fake::text(TABLE)], history: &[], files: vec![], keys: "\"which providers are set up?\",Enter", provider: true },

        // A non-retried provider error: a sentence in `label`, detail in a
        // disclosure, no red (ADR 0009 §5).
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

        // Bare `/resume` over two past sessions.
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

/// Writes a scene's config, files and history under `root`, and parses its
/// keys.
///
/// # Errors
///
/// When a directory or file cannot be created or written, the config or
/// history writer fails, or the scene's keys do not parse.
pub fn seed(script: &Script, theme: Theme, root: &Path, endpoint: &str) -> Result<Prepared> {
    // Must be absolute: the app resolves a relative `HOME` against its cwd.
    let home = root.join("home");
    let cwd = home.join("proj");
    std::fs::create_dir_all(&cwd)?;
    let home = home.canonicalize()?;
    let cwd = cwd.canonicalize()?;

    // The product's own writer, never a copy of its templates.
    let global = home.join(".aldwin");
    let config = aldwin_config::Config::open_at(&cwd, &global)
        .map_err(|e| Error::Scene(format!("seeding global config: {e}")))?;
    config
        .init_global_if_empty()
        .map_err(|e| Error::Scene(format!("seeding global config: {e}")))?;

    // Theme is global-only and read at startup, so it is seeded, not typed.
    // Reduced motion holds the working line still, so a held turn settles;
    // only its timer moves, once a second.
    std::fs::write(
        global.join("tui.yaml"),
        format!("version: 1\ntheme: {theme}\nmotion: reduced\n"),
    )?;

    if script.provider {
        // `openai-compatible` because the anthropic provider ignores
        // `base_url` (see `fake`).
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

/// Writes the scene's past sessions through `HistoryStore`.
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
    use crate::Error;

    /// A scene on one list only could never reach the stage 8 judge.
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
    fn a_scene_outside_the_catalogue_is_refused_by_name() {
        let Err(Error::Scene(message)) = script("nowhere") else {
            panic!("an unknown scene must be refused");
        };
        assert!(message.contains("\"nowhere\""), "{message}");
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
