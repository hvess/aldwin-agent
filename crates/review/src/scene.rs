//! Scenes — the states the binary is put into before capture.
//!
//! A scene is a seeded `HOME`/`.aldwin` config, a queue of canned provider
//! replies, and the keys to type. The real binary then plays it out: real
//! HTTP, real streaming adapter, real core loop, real TUI. Nothing is poked
//! into the UI's memory, which is the difference between this and
//! `render_snapshot.rs` — and the reason a scene here can catch something the
//! snapshot cannot.
//!
//! The catalogue shares its names with `render_snapshot.rs` on purpose: two
//! harnesses, one vocabulary. A scene that cannot be reached through the real
//! path is reshaped or dropped **with a note here**, never quietly pointed at
//! a different state under the same name.

use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use crate::fake::{self, Canned};
use crate::geometry::Theme;

/// The static half of a scene: what it needs seeded and what it will be told.
pub struct Script {
    pub name:    &'static str,
    /// Grants written to the project's `permissions.yaml`. An empty list is
    /// still an answer — the file's *existence* is what says this directory's
    /// access posture has been declared (`cli/src/bootstrap.rs`).
    pub grants:  Vec<&'static str>,
    /// One per request the scene makes, in order.
    pub replies: Vec<Canned>,
    /// Files the scene needs in the project before it runs — a `read` needs
    /// something to read, an `edit` needs its `before` text to exist exactly
    /// once.
    pub files:   Vec<(&'static str, &'static str)>,
    /// Past sessions to seed into this project's history directory, as
    /// `(first user message, started_at, turns)`. Written through the
    /// product's own `HistoryStore`, same rule as the global config: a copy
    /// of the JSONL format here would drift from the one being reviewed.
    ///
    /// `started_at` is fixed per scene so the picker's date column is stable
    /// between runs. It is rendered in *local* time by aldwin-cli, so the
    /// date can still differ between machines in distant timezones — noon
    /// UTC is chosen to keep that within a day either way.
    pub history: &'static [(&'static str, u64, usize)],
    /// Typed once the app has settled.
    pub keys:    &'static str,
    /// Written only when the scene wants a provider configured at all;
    /// `first_run` is defined by its absence.
    pub provider: bool,
}

pub struct Prepared {
    pub cwd:  PathBuf,
    pub home: PathBuf,
    pub keys: Vec<Vec<u8>>,
}

pub const CATALOGUE: &[&str] = &[
    "first_run",
    "empty",
    "conversation",
    "markdown",
    "fenced_diff",
    "long",
    "tools",
    "approval",
    "approval_large",
    "prompt",
    "prompt_path",
    "prompt_scoped",
    "resume",
];

/// Scenes that are wired up. The rest stay in `CATALOGUE` so the vocabulary is
/// visible, and refuse to run rather than capture something else.
pub const IMPLEMENTED: &[&str] = &[
    "first_run",
    "empty",
    "conversation",
    "markdown",
    "fenced_diff",
    "long",
    "tools",
    "approval",
    "approval_large",
    "prompt",
    "prompt_path",
    "prompt_scoped",
    "resume",
];

const DISPATCHER: &str = "crates/tools/src/dispatcher.rs";

const RETRY_RS: &str = "fn backoff(attempt: u32) -> u64 {\n    100\n}\n";

/// Long enough that the approval panel has to clamp it, which is the whole
/// point of the `approval_large` scene.
const LONG_BEFORE: &str = "fn backoff(attempt: u32) -> u64 {\n    let base = 100;\n    let jitter = 0;\n    let ceiling = 30_000;\n    let raw = base + jitter;\n    if raw > ceiling { ceiling } else { raw }\n}\n";

const PROSE: &str = "The retry loop currently sleeps a flat 100ms between attempts, so a rate-limited provider is hit at the same rate that got it rate-limited. Exponential backoff with a cap is the usual fix, and the cap matters more than the curve.";

const TABLE: &str = "Three providers are configured here:\n\n| provider | key variable | streaming |\n| --- | --- | --- |\n| anthropic | ANTHROPIC_API_KEY | yes |\n| openai | OPENAI_API_KEY | yes |\n| google | GOOGLE_API_KEY | no |\n\nOnly the first two stream today.";

const DIFF: &str = "That change is one function:\n\n```diff\n@@ -12,7 +12,9 @@\n-fn backoff(attempt: u32) -> u64 {\n-    100\n+fn backoff(attempt: u32) -> u64 {\n+    let ms = 100u64 << attempt.min(6);\n+    ms.min(30_000)\n }\n```\n\nThe shift caps at six so the wait never runs past thirty seconds.";

pub fn script(name: &str) -> Result<Script> {
    if !CATALOGUE.contains(&name) {
        return Err(Error::new(ErrorKind::InvalidInput, format!("unknown scene {name:?}; see scene::CATALOGUE")));
    }
    if !IMPLEMENTED.contains(&name) {
        return Err(Error::new(
            ErrorKind::Unsupported,
            format!("scene {name:?} is in the catalogue but not wired up yet; implemented: {IMPLEMENTED:?}"),
        ));
    }

    let ask = "\"how should the retry loop back off?\",Enter";
    Ok(match name {
        // First run *is* the absence of both answers: no provider resolves,
        // and this directory has no permissions file.
        "first_run" => Script { name: "first_run", grants: vec![], replies: vec![], history: &[], files: vec![], keys: "", provider: false },

        // Configured and answered, nothing said yet — the state a returning
        // developer opens into, and the one `/clear` returns them to.
        "empty" => Script { name: "empty", grants: vec![], replies: vec![], history: &[], files: vec![], keys: "", provider: true },

        "conversation" => Script { name: "conversation", grants: vec![], replies: vec![fake::text(PROSE)], history: &[], files: vec![], keys: ask, provider: true },

        "markdown" => Script { name: "markdown", grants: vec![], replies: vec![fake::text(TABLE)], history: &[], files: vec![], keys: "\"which providers are set up?\",Enter", provider: true },

        "fenced_diff" => Script { name: "fenced_diff", grants: vec![], replies: vec![fake::text(DIFF)], history: &[], files: vec![], keys: ask, provider: true },

        // --- the tool-driven scenes -------------------------------------
        // What each of these reaches is decided by the *grants it seeds*, not
        // by faking a panel: an allowed tool runs, an unallowed one asks, and
        // an edit always asks with a diff because `edit` is outside the
        // permissions model entirely (ADR 0004 §3).
        "tools" => Script {
            name:    "tools",
            grants:  vec!["read: read"],
            replies: vec![
                fake::tool_call("call-1", "read", serde_json::json!({ "path": DISPATCHER })),
                fake::text("The dispatcher denies by absence: a tool with no registration is refused before any permission check runs."),
            ],
            history: &[],
            files:   vec![(DISPATCHER, "// the dispatcher\n")],
            keys:    "\"what does the dispatcher do on a deny-by-absence?\",Enter",
            provider: true,
        },

        "prompt" => Script {
            name:    "prompt",
            grants:  vec![],
            replies: vec![fake::tool_call("call-1", "read", serde_json::json!({ "path": "README.md" }))],
            history: &[],
            files:   vec![("README.md", "# project\n")],
            keys:    "\"read the readme\",Enter",
            provider: true,
        },

        "prompt_path" => Script {
            name:    "prompt_path",
            grants:  vec![],
            replies: vec![fake::tool_call("call-3", "read", serde_json::json!({ "path": DISPATCHER }))],
            history: &[],
            files:   vec![(DISPATCHER, "// the dispatcher\n")],
            keys:    "\"what does the dispatcher do on a deny-by-absence?\",Enter",
            provider: true,
        },

        // **The name is historical and the scene has been repurposed.** It
        // existed to widen a grant with `Tab` and show the panel naming the
        // broadened rule; ADR 0003 removed the toggle and ADR 0004 removed
        // the pattern it toggled between, so there was nothing left for it
        // to show — it rendered a panel byte-identical to `prompt`'s, which
        // a stage 5 judge noticed and correctly could not make anything of.
        //
        // It now carries the one prompt shape nothing else covers: a `run`
        // call declared a **write**. `prompt` and `prompt_path` are both
        // `read`-class built-ins, so without this the whole catalogue showed
        // one side of ADR 0004 §2's class axis. Renaming it is a
        // catalogue-and-baseline change, deliberately not taken here.
        "prompt_scoped" => Script {
            name:    "prompt_scoped",
            grants:  vec![],
            replies: vec![fake::tool_call(
                "call-4",
                "run",
                serde_json::json!({ "program": "git", "args": ["commit", "-m", "wire the dispatcher"], "class": "write" }),
            )],
            history: &[],
            files:   vec![],
            keys:    "\"commit what we have\",Enter",
            provider: true,
        },

        "approval" => Script {
            name:    "approval",
            grants:  vec!["read: read"],
            replies: vec![fake::tool_call(
                "call-1",
                "edit",
                serde_json::json!({ "path": "src/retry.rs", "before": "    100\n", "after": "    100u64 << attempt.min(6)\n" }),
            )],
            history: &[],
            files:   vec![("src/retry.rs", RETRY_RS)],
            keys:    ask,
            provider: true,
        },

        "approval_large" => Script {
            name:    "approval_large",
            grants:  vec!["read: read"],
            replies: vec![fake::tool_call(
                "call-1",
                "edit",
                serde_json::json!({
                    "path":   "src/retry.rs",
                    "before": LONG_BEFORE,
                    "after":  "fn backoff(attempt: u32) -> u64 {\n    (100u64 << attempt.min(6)).min(30_000)\n}\n",
                }),
            )],
            history: &[],
            files:   vec![("src/retry.rs", LONG_BEFORE)],
            keys:    ask,
            provider: true,
        },

        // Long enough to fill the tallest frame and scroll the shortest, so
        // the transcript's auto-follow is exercised rather than described.
        "long" => Script {
            name:    "long",
            grants:  vec![],
            replies: vec![fake::text(&format!("{PROSE}\n\n{TABLE}\n\n{DIFF}\n\n{PROSE}"))],
            history: &[],
            files:   vec![],
            keys:    ask,
            provider: true,
        },

        // The session picker. Two past sessions rather than one, so the
        // list is a list — a single row cannot show the cursor sitting on
        // one entry among others, which is most of what the control does.
        // Different turn counts on purpose: the row's detail half
        // distinguishes "1 turn" from "4 turns", and a scene where every
        // count matched would not show it.
        "resume" => Script {
            name:    "resume",
            grants:  vec![],
            replies: vec![],
            history: &[
                ("how should the retry loop back off?", 1_789_732_800, 4),
                ("which providers are set up?", 1_789_819_200, 1),
            ],
            files:   vec![],
            keys:    "\"/resume\",Enter",
            provider: true,
        },

        _ => unreachable!("guarded by IMPLEMENTED above"),
    })
}

/// Write the config a scene runs under, and parse its keys.
pub fn seed(script: &Script, theme: Theme, root: &Path, endpoint: &str) -> Result<Prepared> {
    // Absolute, always. `HOME` is resolved by the app against its own working
    // directory, so a relative one sends it looking for `~/.aldwin` inside
    // the project — it finds nothing, opens first run, and every frame in the
    // session is the same wrong screen in the wrong theme. That is exactly
    // what a relative `--run` produced, and it looked plausible.
    let home = root.join("home");
    let cwd = home.join("proj");
    std::fs::create_dir_all(&cwd)?;
    let home = home.canonicalize()?;
    let cwd = cwd.canonicalize()?;

    // `~/.aldwin` is materialised by the product's own writer rather than a
    // copy of its templates here. Seeding one file by hand does not work and
    // fails loudly: `init_global_if_empty` treats a directory missing any of
    // permissions.yaml, mcp.yaml or tui.yaml as half-deleted and refuses to
    // start.
    let global = home.join(".aldwin");
    let config = aldwin_config::Config::open_at(&cwd, &global)
        .map_err(|e| Error::other(format!("seeding global config: {e}")))?;
    config
        .init_global_if_empty()
        .map_err(|e| Error::other(format!("seeding global config: {e}")))?;

    // Theme is global-only and read once at startup, so it is seeded as config
    // rather than sent as a command.
    std::fs::write(global.join("tui.yaml"), format!("version: 1\ntheme: {theme}\n"))?;

    if script.provider {
        // The endpoint carries an ephemeral port, so this is written per run.
        // `openai-compatible` is not a preference: `base_url` is ignored for
        // the anthropic provider, and a local fake is nothing but a base_url.
        std::fs::write(
            global.join("provider.yaml"),
            format!(
                "version: 1\nprovider: openai-compatible\nmodel: gpt-5\nbase_url: {endpoint}\napi_key_env: ALDWIN_SHOT_KEY\n"
            ),
        )?;

        // The file's existence is what says this directory's access posture
        // has been declared; an empty allow list is a real answer.
        let project = cwd.join(".aldwin");
        std::fs::create_dir_all(&project)?;
        // v2 (ADR 0004): entries are `program: class`, not `kind:pattern`.
        // Writing a v1 file here would not merely be stale — `Config::open`
        // retires one, so the scene would silently start with no grants at
        // all and every seeded scene would draw a prompt it was not meant to.
        let allow = script.grants.iter().map(|g| format!("  - {g}\n")).collect::<String>();
        let allow = if allow.is_empty() { "allow: []\n".to_string() } else { format!("allow:\n{allow}") };
        std::fs::write(project.join("permissions.yaml"), format!("version: 2\n{allow}deny: []\n"))?;
    }

    seed_history(script, &global, &cwd)?;

    for (path, contents) in &script.files {
        let file = cwd.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, contents)?;
    }

    let keys = crate::keys::parse(script.keys).map_err(Error::other)?;
    Ok(Prepared { cwd, home, keys })
}

/// Write this scene's past sessions, through the product's own writer.
///
/// The ids are derived from `started_at` rather than minted, so a scene's
/// transcripts have the same names every run — `SessionId::mint` deliberately
/// carries a pid and a counter, neither of which is reproducible.
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
            version:    HISTORY_VERSION,
            started_at: *started_at,
            cwd:        cwd.to_string_lossy().into_owned(),
            model:      "gpt-5".into(),
        };
        let store = HistoryStore::create(&dir, &id, &header).map_err(|e| Error::other(format!("seeding history: {e}")))?;
        for turn in 0..*turns {
            let turn_id = TurnId(turn as u64 + 1);
            let step_id = StepId(turn as u64 + 1);
            // Only the first turn carries the seeded text: the title comes
            // from the first user message, and the rest exist to make the
            // turn count true.
            let user = if turn == 0 { (*text).to_string() } else { format!("and then? ({turn})") };
            for record in [
                LogRecord::TurnStarted { turn_id },
                LogRecord::UserMessage { turn_id, text: user },
                LogRecord::AssistantMessage { turn_id, step_id, text: PROSE.into() },
                LogRecord::TurnEnded { turn_id, reason: TurnEndReason::EndTurn },
            ] {
                store.append(&record).map_err(|e| Error::other(format!("seeding history: {e}")))?;
            }
        }
    }
    Ok(())
}
