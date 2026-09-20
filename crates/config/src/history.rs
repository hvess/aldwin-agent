//! Persisted conversation transcripts — the on-disk half of `/resume`.
//!
//! One session is one append-only JSONL file under
//! `~/.mjolnir/history/<project-slug>/<session-id>.jsonl`. The first line is
//! a [`SessionHeader`]; every line after it is one `LogRecord`.
//!
//! JSONL rather than one document because writes are appends: a process
//! killed mid-turn costs the partial last line and nothing else. That is
//! also why this module does not use `fsio`'s atomic write — atomicity here
//! would mean rewriting the whole transcript on every record, which is
//! exactly the O(n) write path mjolnir-history.md rules out.
//!
//! Reading is deliberately forgiving and lives in [`load`]: an unparseable
//! line is skipped, and the records are then truncated after the last
//! complete turn. See that function for why the truncation is load-bearing
//! rather than tidiness.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mjolnir_core::{LogRecord, SessionId};
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Bumped when the transcript's shape changes incompatibly. A file whose
/// header carries an unknown version is skipped by [`list`] rather than
/// failing the listing — one unreadable old transcript must not cost the
/// developer the rest of their history.
pub const HISTORY_VERSION: u32 = 1;

/// The transcript's first line.
///
/// Note what is *not* here: the title. It is derived from the first user
/// message at listing time ([`SessionSummary::title`]), because at the
/// moment a file is opened no user message exists yet — putting it in the
/// header would mean going back to rewrite line one mid-session, which is
/// the one thing an append-only file should never do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionHeader {
    pub version:    u32,
    /// Unix epoch seconds. Rendered by the picker; never parsed back.
    pub started_at: u64,
    pub cwd:        String,
    pub model:      String,
}

/// One row of what `/resume` lists.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub id:         SessionId,
    pub started_at: u64,
    /// First user message, trimmed to one line — see [`derive_title`].
    pub title:      String,
    /// Number of *completed* turns — what a resume would actually restore,
    /// since [`load`] truncates after the last `TurnEnded`. Counting started
    /// turns instead would promise a turn back that resume then drops.
    pub turns:      usize,
}

/// How much of a first user message becomes a title.
const TITLE_MAX: usize = 72;

/// Where a transcript's bytes go, and whether it exists yet.
#[derive(Debug)]
enum Sink {
    /// The header is composed but nothing is on disk. A session that says
    /// nothing must leave no file — see [`HistoryStore::create`].
    Pending(String),
    Open(File),
    /// A write failed. The first failure is reported by whoever owns the
    /// event channel (mjolnir-cli's sink wrapper); every subsequent record is
    /// dropped silently, because a disk that is full at record 200 is still
    /// full at record 201 and the developer does not need to be told 400
    /// times.
    Off,
}

/// The transcript this session is writing to.
///
/// Holds the file open for the life of the session rather than reopening per
/// record: a transcript is written on every committed record, and the open
/// is the expensive half of an append.
#[derive(Debug)]
pub struct HistoryStore {
    path: PathBuf,
    sink: Mutex<Sink>,
}

impl HistoryStore {
    /// Prepare this session's transcript. **The file is not created until
    /// the first record is appended.**
    ///
    /// Opening a session is not the same act as having a conversation, and
    /// every launch used to leave a header-only file behind — so did every
    /// `/clear`, which opens a fresh transcript the developer may never say
    /// anything into. Those files are unresumable by construction (`load`
    /// finds no completed turn) and listing them offered rows the picker
    /// would then refuse. Deferring the create means a session that says
    /// nothing leaves nothing.
    ///
    /// The directory *is* created here, eagerly: it is the cheap half, and
    /// it is what lets a session that cannot write history say so at startup
    /// rather than at the first committed record.
    pub fn create(dir: &Path, id: &SessionId, header: &SessionHeader) -> Result<Self, ConfigError> {
        fs::create_dir_all(dir).map_err(|e| ConfigError::Io { path: dir.to_path_buf(), source: e })?;
        let line = serde_json::to_string(header).expect("SessionHeader is always serialisable");
        Ok(Self { path: transcript_path(dir, id), sink: Mutex::new(Sink::Pending(line)) })
    }

    /// Create the file and write the header — the deferred half of
    /// [`HistoryStore::create`], run on the first record.
    ///
    /// Mode `0600`: a transcript carries whatever the session's tool results
    /// carried — file contents, command output, anything a `.env` held — so
    /// it is readable by its owner and nobody else. See mjolnir-history.md's
    /// Pitfalls.
    fn materialise(path: &Path, header: &str) -> Result<File, ConfigError> {
        // `create_new`, not `create`: an id that already has a transcript is
        // a collision, and appending onto one would merge two conversations
        // into a file `load` then resumes as a single history. Loud is the
        // only safe failure here. Continuing a transcript on purpose is
        // [`HistoryStore::reopen`], which says so by name.
        let mut options = OpenOptions::new();
        options.create_new(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|e| ConfigError::Io { path: path.to_path_buf(), source: e })?;
        writeln!(file, "{header}").map_err(|e| ConfigError::Io { path: path.to_path_buf(), source: e })?;
        Ok(file)
    }

    /// Reopen an existing transcript for appending — what `/resume` does, so
    /// a resumed conversation continues in the file it came from rather than
    /// forking a second one (mjolnir-history.md's fork-free Decision).
    ///
    /// No header is written: the file already has one, and its `started_at`
    /// should keep saying when the conversation began, not when it was last
    /// picked up.
    pub fn reopen(dir: &Path, id: &SessionId) -> Result<Self, ConfigError> {
        let path = transcript_path(dir, id);
        let file = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| ConfigError::Io { path: path.clone(), source: e })?;
        Ok(Self { path, sink: Mutex::new(Sink::Open(file)) })
    }

    pub fn path(&self) -> &Path { &self.path }

    /// Append one record, creating the transcript if this is the first.
    /// `Err` on the first failure only; every call after that is a silent
    /// no-op (see [`Sink::Off`]).
    pub fn append(&self, record: &LogRecord) -> Result<(), ConfigError> {
        let mut guard = self.sink.lock().expect("history file lock poisoned");

        if let Sink::Pending(header) = &*guard {
            match Self::materialise(&self.path, header) {
                Ok(file) => *guard = Sink::Open(file),
                Err(e) => {
                    *guard = Sink::Off;
                    return Err(e);
                }
            }
        }
        let Sink::Open(file) = &mut *guard else { return Ok(()) };

        let line = serde_json::to_string(record).expect("LogRecord is always serialisable");
        match writeln!(file, "{line}") {
            Ok(()) => Ok(()),
            Err(e) => {
                // Drop the handle so the next record short-circuits above
                // rather than retrying a write that just failed.
                *guard = Sink::Off;
                Err(ConfigError::Io { path: self.path.clone(), source: e })
            }
        }
    }
}

/// Every *resumable* session in this project, newest first.
///
/// **A session is listed if and only if it can be resumed.** A transcript
/// with no completed turn — a session that was opened and quit, or one killed
/// inside its first turn — loads as nothing (see [`load`]), so listing it
/// would offer a row the picker then refuses. The two rules are one rule and
/// they are deliberately written against the same record.
///
/// Unreadable and unrecognised files are skipped rather than failing the
/// listing: one corrupt transcript must not cost the developer the rest of
/// their history. A missing directory is an empty list, not an error — it is
/// the normal "nothing recorded here yet" state.
pub fn list(dir: &Path) -> Vec<SessionSummary> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };

    let mut sessions: Vec<SessionSummary> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let id = SessionId(e.path().file_stem()?.to_str()?.to_string());
            summarise(&e.path(), id)
        })
        .collect();

    // Newest first. `SessionId` sorts into start order too, but `started_at`
    // is the field the row actually shows, so it is the one to sort on —
    // otherwise a list could display out of the order it claims.
    sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
    sessions
}

/// One session's records, ready to become a `ConversationLog`.
///
/// **Truncates after the last `TurnEnded`.** A tool call and its result are a
/// pair: a process killed between them leaves a `ToolUse` on disk with no
/// `ToolResult`, and `messages_from_log` would rebuild that into an assistant
/// message carrying an unmatched tool-use block — which every provider
/// rejects outright. Dropping back to the last finished turn is what makes
/// the crash case resume exists for actually resumable, and it subsumes the
/// torn-final-line case for free.
pub fn load(dir: &Path, id: &SessionId) -> Result<Vec<LogRecord>, ConfigError> {
    let path = transcript_path(dir, id);
    let file = File::open(&path).map_err(|e| ConfigError::Io { path: path.clone(), source: e })?;

    let mut records: Vec<LogRecord> = BufReader::new(file)
        .lines()
        .skip(1) // the header
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect();

    match records.iter().rposition(|r| matches!(r, LogRecord::TurnEnded { .. })) {
        Some(last) => records.truncate(last + 1),
        // No turn ever finished — there is nothing safely resumable in this
        // file, and half a turn is worse than none.
        None => records.clear(),
    }
    Ok(records)
}

/// Where this project's transcripts live: `<history root>/<project slug>`.
pub fn project_dir(history_root: &Path, project_root: &Path) -> PathBuf {
    history_root.join(project_slug(project_root))
}

fn transcript_path(dir: &Path, id: &SessionId) -> PathBuf {
    dir.join(format!("{id}.jsonl"))
}

/// A filesystem-safe, collision-resistant name for a project root.
///
/// The readable half is for a human listing the directory; the hash is what
/// actually distinguishes two projects, since basenames collide constantly
/// (every `src`, every `web`). FNV-1a rather than `DefaultHasher` because
/// this name has to mean the same thing across Rust releases — a hash that
/// changes under the developer's feet silently orphans their history.
fn project_slug(project_root: &Path) -> String {
    let name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in project_root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{safe}-{hash:016x}")
}

fn summarise(path: &Path, id: SessionId) -> Option<SessionSummary> {
    let file = File::open(path).ok()?;
    let mut lines = BufReader::new(file).lines().map_while(Result::ok);

    let header: SessionHeader = serde_json::from_str(&lines.next()?).ok()?;
    if header.version != HISTORY_VERSION {
        return None;
    }

    let mut title = None;
    let mut turns = 0;
    for record in lines.filter_map(|l| serde_json::from_str::<LogRecord>(&l).ok()) {
        match record {
            LogRecord::TurnEnded { .. } => turns += 1,
            LogRecord::UserMessage { text, .. } if title.is_none() => title = Some(derive_title(&text)),
            _ => {}
        }
    }

    // Nothing finished, so there is nothing to resume and nothing to list.
    // This is also what keeps transcripts left by older builds — which were
    // created eagerly at startup — out of the picker.
    if turns == 0 {
        return None;
    }

    Some(SessionSummary {
        id,
        started_at: header.started_at,
        // A completed turn without a user message is not reachable through
        // the app, but a damaged transcript can look like one.
        title: title.unwrap_or_else(|| "(untitled)".into()),
        turns,
    })
}

/// The first user message, on one line, bounded.
///
/// Derived rather than authored, and deliberately not summarised by the
/// model: filing is not what the developer's tokens are for. If the result
/// reads badly in a list, the fix is the list.
fn derive_title(text: &str) -> String {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if first.chars().count() <= TITLE_MAX {
        first.to_string()
    } else {
        let head: String = first.chars().take(TITLE_MAX).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mjolnir_core::{StepId, ToolCall, TurnEndReason, TurnId};
    use tempfile::tempdir;

    fn header() -> SessionHeader {
        SessionHeader { version: HISTORY_VERSION, started_at: 1_700_000_000, cwd: "/tmp/p".into(), model: "m".into() }
    }

    fn turn(n: u64, text: &str) -> Vec<LogRecord> {
        vec![
            LogRecord::TurnStarted { turn_id: TurnId(n) },
            LogRecord::UserMessage { turn_id: TurnId(n), text: text.into() },
            LogRecord::AssistantMessage { turn_id: TurnId(n), step_id: StepId(n), text: "sure".into() },
            LogRecord::TurnEnded { turn_id: TurnId(n), reason: TurnEndReason::EndTurn },
        ]
    }

    #[test]
    fn a_written_transcript_loads_back_as_the_records_that_went_in() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000001-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        let records = turn(1, "hello");
        for record in &records {
            store.append(record).unwrap();
        }
        assert_eq!(load(dir.path(), &id).unwrap(), records);
    }

    /// Verify for Step 2: a killed process leaves a file whose earlier lines
    /// all parse. The kill is simulated by appending a half-written line,
    /// which is the only damage an append-only file can take.
    #[test]
    fn a_torn_final_line_costs_that_line_and_nothing_else() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000002-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        let records = turn(1, "hello");
        for record in &records {
            store.append(record).unwrap();
        }
        drop(store);

        let path = transcript_path(dir.path(), &id);
        let mut raw = fs::read_to_string(&path).unwrap();
        raw.push_str("{\"type\":\"turn_star");
        fs::write(&path, raw).unwrap();

        assert_eq!(load(dir.path(), &id).unwrap(), records, "the intact turn survives the torn tail");
    }

    /// Verify for Step 3, and the reason the truncation rule exists: a
    /// process killed between a tool call and its result must not resume
    /// into an unmatched tool-use block.
    #[test]
    fn an_unfinished_turn_is_dropped_back_to_the_last_finished_one() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000003-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();

        let finished = turn(1, "first");
        for record in &finished {
            store.append(record).unwrap();
        }
        // A second turn that got as far as calling a tool and then died.
        store.append(&LogRecord::TurnStarted { turn_id: TurnId(2) }).unwrap();
        store.append(&LogRecord::UserMessage { turn_id: TurnId(2), text: "second".into() }).unwrap();
        store
            .append(&LogRecord::ToolUse {
                turn_id: TurnId(2),
                step_id: StepId(2),
                call:    ToolCall { id: "c1".into(), name: "read".into(), input: serde_json::json!({}) },
            })
            .unwrap();

        let loaded = load(dir.path(), &id).unwrap();
        assert_eq!(loaded, finished, "the half-turn is dropped whole");
        assert!(
            !loaded.iter().any(|r| matches!(r, LogRecord::ToolUse { .. })),
            "no tool call survives without its result"
        );
    }

    #[test]
    fn a_transcript_whose_first_turn_never_finished_loads_as_nothing() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000004-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        store.append(&LogRecord::TurnStarted { turn_id: TurnId(1) }).unwrap();
        store.append(&LogRecord::UserMessage { turn_id: TurnId(1), text: "hi".into() }).unwrap();

        assert!(load(dir.path(), &id).unwrap().is_empty(), "half a turn is worse than none");
    }

    /// Verify for Step 2: history must never be able to fail a turn, so the
    /// failure it hands back has to be an error rather than a panic.
    #[test]
    fn an_unwritable_history_directory_is_an_error_not_a_panic() {
        let dir = tempdir().unwrap();
        let blocked = dir.path().join("wall");
        fs::write(&blocked, "not a directory").unwrap();

        let result = HistoryStore::create(&blocked.join("history"), &SessionId("x".into()), &header());
        assert!(matches!(result, Err(ConfigError::Io { .. })));
    }

    #[test]
    fn listing_titles_each_session_from_its_first_user_message_newest_first() {
        let dir = tempdir().unwrap();
        for (id, started_at, text) in [
            ("0000000010-1", 1_700_000_010, "older question"),
            ("0000000020-1", 1_700_000_020, "newer question"),
        ] {
            let id = SessionId(id.into());
            let store = HistoryStore::create(dir.path(), &id, &SessionHeader { started_at, ..header() }).unwrap();
            for record in turn(1, text) {
                store.append(&record).unwrap();
            }
        }

        let sessions = list(dir.path());
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].title, "newer question", "newest first");
        assert_eq!(sessions[1].title, "older question");
        assert_eq!(sessions[0].turns, 1);
    }

    /// A session that is opened and quit without a word must leave nothing
    /// behind — not a file, and not a row.
    #[test]
    fn a_session_that_says_nothing_writes_no_file_at_all() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000060-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        drop(store);

        assert!(!transcript_path(dir.path(), &id).exists(), "no transcript for a session with no records");
        assert!(list(dir.path()).is_empty());
    }

    /// The listing's invariant: a row is offered only if resuming it would
    /// restore something. These two rules are written against the same
    /// record and must not drift apart.
    #[test]
    fn a_session_is_listed_if_and_only_if_it_can_be_resumed() {
        let dir = tempdir().unwrap();

        // Finished — listed and resumable.
        let finished = SessionId("0000000070-1".into());
        let store = HistoryStore::create(dir.path(), &finished, &header()).unwrap();
        for record in turn(1, "answered") {
            store.append(&record).unwrap();
        }
        drop(store);

        // Started, never finished — neither listed nor resumable.
        let unfinished = SessionId("0000000080-1".into());
        let store = HistoryStore::create(dir.path(), &unfinished, &header()).unwrap();
        store.append(&LogRecord::TurnStarted { turn_id: TurnId(2) }).unwrap();
        store.append(&LogRecord::UserMessage { turn_id: TurnId(2), text: "cut off".into() }).unwrap();
        drop(store);

        for id in [&finished, &unfinished] {
            let listed = list(dir.path()).iter().any(|s| s.id == *id);
            let resumable = !load(dir.path(), id).unwrap().is_empty();
            assert_eq!(listed, resumable, "{id} is listed={listed} but resumable={resumable}");
        }
        assert_eq!(list(dir.path()).len(), 1);
    }

    /// An older build created the file eagerly, so a developer upgrading has
    /// header-only transcripts already on disk. They must not be offered.
    #[test]
    fn a_header_only_transcript_from_an_older_build_is_not_listed() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000090-1".into());
        fs::create_dir_all(dir.path()).unwrap();
        let line = serde_json::to_string(&header()).unwrap();
        fs::write(transcript_path(dir.path(), &id), format!("{line}\n")).unwrap();

        assert!(list(dir.path()).is_empty(), "a session with no completed turn is not a session to resume");
    }

    /// The count has to be what resume restores, not what was attempted —
    /// `load` drops the unfinished tail, so counting started turns would
    /// promise a turn back that never arrives.
    #[test]
    fn the_turn_count_is_completed_turns_not_attempted_ones() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000100-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        for n in 1..=2 {
            for record in turn(n, "question") {
                store.append(&record).unwrap();
            }
        }
        store.append(&LogRecord::TurnStarted { turn_id: TurnId(3) }).unwrap();
        drop(store);

        let listed = &list(dir.path())[0];
        assert_eq!(listed.turns, 2, "the third turn never finished");
        assert_eq!(load(dir.path(), &id).unwrap().iter().filter(|r| matches!(r, LogRecord::TurnEnded { .. })).count(), 2);
    }

    #[test]
    fn a_session_from_a_future_schema_is_skipped_rather_than_failing_the_listing() {
        let dir = tempdir().unwrap();
        let good = SessionId("0000000030-1".into());
        let store = HistoryStore::create(dir.path(), &good, &header()).unwrap();
        for record in turn(1, "readable") {
            store.append(&record).unwrap();
        }
        let future = SessionHeader { version: HISTORY_VERSION + 1, ..header() };
        HistoryStore::create(dir.path(), &SessionId("0000000040-1".into()), &future).unwrap();

        let sessions = list(dir.path());
        assert_eq!(sessions.len(), 1, "the unreadable one is skipped");
        assert_eq!(sessions[0].id, good);
    }

    #[test]
    fn listing_a_directory_that_does_not_exist_is_empty_rather_than_an_error() {
        let dir = tempdir().unwrap();
        assert!(list(&dir.path().join("never-used")).is_empty());
    }

    #[test]
    fn reopening_continues_the_same_file_rather_than_forking_one() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000050-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        for record in turn(1, "first") {
            store.append(&record).unwrap();
        }
        drop(store);

        let store = HistoryStore::reopen(dir.path(), &id).unwrap();
        for record in turn(2, "second") {
            store.append(&record).unwrap();
        }
        drop(store);

        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "one conversation is one file");
        let loaded = load(dir.path(), &id).unwrap();
        assert_eq!(loaded.len(), 8);
        assert_eq!(list(dir.path())[0].turns, 2);
    }

    #[test]
    fn two_projects_with_the_same_basename_get_different_slugs() {
        let a = project_slug(Path::new("/home/dev/one/src"));
        let b = project_slug(Path::new("/home/dev/two/src"));
        assert_ne!(a, b);
        assert!(a.starts_with("src-"), "the readable half survives: {a}");
    }

    #[test]
    fn a_project_slug_is_filesystem_safe() {
        let slug = project_slug(Path::new("/home/dev/my project (v2)"));
        assert!(slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{slug}");
    }
}
