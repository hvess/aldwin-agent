//! Conversation transcripts, the on-disk half of `/resume`
//! (`.claude/spec/archive/aldwin-history.md`).
//!
//! One session is one append-only JSONL file,
//! `~/.aldwin/history/<project-slug>/<session-id>.jsonl`: a [`SessionHeader`]
//! line, then one `LogRecord` per line. Do not use `fsio`'s atomic write
//! here: it would rewrite the whole file per record. A killed process costs
//! only its partial last line; [`load`] skips bad lines and unfinished turns.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aldwin_core::{LogRecord, SessionId};
use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// The transcript schema version; bump on an incompatible change. [`list`]
/// skips a file with any other version rather than failing.
pub const HISTORY_VERSION: u32 = 1;

/// The transcript's first line.
///
/// Holds no title: it is derived at listing time ([`SessionSummary::title`]),
/// since storing it here would mean rewriting line one of an append-only file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionHeader {
    /// The transcript's shape; [`HISTORY_VERSION`] when written by this build.
    pub version: u32,
    /// Unix epoch seconds. Rendered by the picker; never parsed back.
    pub started_at: u64,
    /// The directory the session was started in.
    pub cwd: String,
    /// The model the session began on, as the provider spells it.
    pub model: String,
}

/// One row of what `/resume` lists.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    /// The session, and the file name its transcript is under.
    pub id: SessionId,
    /// Unix epoch seconds, from the header.
    pub started_at: u64,
    /// The first user message's first non-blank line, at most 72 chars plus
    /// `…`; `(untitled)` when it has none.
    pub title: String,
    /// Completed turns only: [`load`] drops a turn with no `TurnEnded`, and
    /// this must count what a resume restores.
    pub turns: usize,
}

/// How much of a first user message becomes a title.
const TITLE_MAX: usize = 72;

/// Where a transcript's bytes go, and whether it exists yet.
#[derive(Debug)]
enum Sink {
    /// Nothing on disk yet; see [`HistoryStore::create`].
    Pending(SessionHeader),
    Open(File),
    /// A write failed. aldwin-cli's sink wrapper reports the first failure;
    /// every later record is dropped silently so it is not reported again.
    Off,
}

/// The transcript this session is writing to. Holds the file open for the
/// session rather than reopening per record, since it is written on every
/// committed record.
#[derive(Debug)]
pub struct HistoryStore {
    path: PathBuf,
    sink: Mutex<Sink>,
}

impl HistoryStore {
    /// Prepares this session's transcript. The file is created only on the
    /// first append, so a session or `/clear` that records nothing leaves no
    /// file. The directory is created now, so an unwritable history is
    /// reported at startup.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the directory cannot be created.
    pub fn create(dir: &Path, id: &SessionId, header: &SessionHeader) -> Result<Self, ConfigError> {
        fs::create_dir_all(dir).map_err(|e| ConfigError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        Ok(Self {
            path: transcript_path(dir, id),
            sink: Mutex::new(Sink::Pending(header.clone())),
        })
    }

    /// Creates the file and writes the header, on the first record.
    ///
    /// Mode `0600`: a transcript holds whatever tool results held
    /// (aldwin-history.md Pitfalls).
    fn materialise(path: &Path, header: &SessionHeader) -> Result<File, ConfigError> {
        // `create_new`, not `create`: an existing file is an id collision,
        // and appending would merge two conversations. Continuing on purpose
        // is [`HistoryStore::reopen`].
        let mut options = OpenOptions::new();
        options.create_new(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        write_line(&mut file, header).map_err(|e| ConfigError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Ok(file)
    }

    /// Reopens an existing transcript for appending, for `/resume`; the
    /// conversation continues in its own file (aldwin-history.md's fork-free
    /// Decision). Writes no header, so `started_at` stays the original.
    ///
    /// A missing final newline (a torn record) is written first; otherwise
    /// the next record fuses into the torn line and [`load`] skips both.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the transcript does not exist or cannot be
    /// opened, read or written.
    pub fn reopen(dir: &Path, id: &SessionId) -> Result<Self, ConfigError> {
        let path = transcript_path(dir, id);
        let io = |e| ConfigError::Io {
            path: path.clone(),
            source: e,
        };
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .map_err(io)?;
        if file.metadata().map_err(io)?.len() > 0 {
            let mut last = [0u8; 1];
            file.seek(SeekFrom::End(-1)).map_err(io)?;
            file.read_exact(&mut last).map_err(io)?;
            if last != *b"\n" {
                file.write_all(b"\n").map_err(io)?;
            }
        }
        Ok(Self {
            path,
            sink: Mutex::new(Sink::Open(file)),
        })
    }

    /// Where the transcript is, or will be once the first record lands.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one record, creating the transcript on the first. `Err` on
    /// the first failure only; every later call is a silent no-op.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the transcript cannot be created or the
    /// record cannot be written.
    ///
    /// # Panics
    ///
    /// If an earlier append panicked while holding the transcript's lock.
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
        let Sink::Open(file) = &mut *guard else {
            return Ok(());
        };

        match write_line(file, record) {
            Ok(()) => Ok(()),
            Err(e) => {
                *guard = Sink::Off;
                Err(ConfigError::Io {
                    path: self.path.clone(),
                    source: e,
                })
            }
        }
    }
}

/// Writes `value` as one line in one `write_all`. Do not use `writeln!`: it
/// is two writes, and `O_APPEND` makes only each one atomic, so a second
/// process on the same transcript (ADR 0005 allows two) can land between them.
fn write_line(out: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).expect("transcript lines are always serialisable");
    line.push(b'\n');
    out.write_all(&line)
}

/// A transcript's lines, as bytes; an I/O error ends them.
///
/// Not `BufRead::lines`: a record torn mid-character is not UTF-8 and would
/// end the read there; as bytes it only fails to parse.
fn lines(file: File) -> impl Iterator<Item = Vec<u8>> {
    BufReader::new(file).split(b'\n').map_while(Result::ok)
}

/// Every resumable session in `dir`, newest first.
///
/// Invariant: listed if and only if [`load`] returns a completed turn; keep
/// `summarise` and `load` agreeing on `TurnEnded`. Unreadable or unknown
/// files are skipped; a missing `dir` is an empty list.
pub fn list(dir: &Path) -> Vec<SessionSummary> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut sessions: Vec<SessionSummary> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let id = SessionId(e.path().file_stem()?.to_str()?.to_string());
            summarise(&e.path(), id)
        })
        .collect();

    // By `started_at`, not `SessionId`: it is the field the row shows.
    sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
    sessions
}

/// One session's records, ready to become a `ConversationLog`.
///
/// Only turns that reached `TurnEnded` are returned, wherever they sit in
/// the file: an unfinished one can hold a `ToolUse` with no `ToolResult`,
/// which `messages_from_log` turns into a tool-use block every provider
/// rejects. A resumed crash leaves such a turn mid-file.
///
/// # Errors
///
/// [`ConfigError::Io`] when the transcript cannot be opened. A line that
/// cannot be read or parsed is skipped, not an error.
pub fn load(dir: &Path, id: &SessionId) -> Result<Vec<LogRecord>, ConfigError> {
    let path = transcript_path(dir, id);
    let file = File::open(&path).map_err(|e| ConfigError::Io {
        path: path.clone(),
        source: e,
    })?;

    let mut records = Vec::new();
    let mut turn = Vec::new();
    let parsed = lines(file)
        .skip(1) // the header
        .filter_map(|line| serde_json::from_slice::<LogRecord>(&line).ok());
    for record in parsed {
        if matches!(record, LogRecord::TurnStarted { .. }) {
            turn.clear();
        }
        let ended = matches!(record, LogRecord::TurnEnded { .. });
        turn.push(record);
        if ended {
            records.append(&mut turn);
        }
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

/// A filesystem-safe name for a project root: its basename, then a hash of
/// the full path that tells same-named projects apart.
///
/// FNV-1a, not `DefaultHasher`: the slug must be stable across Rust
/// releases, or existing history is orphaned.
fn project_slug(project_root: &Path) -> String {
    let name = project_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in project_root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{safe}-{hash:016x}")
}

/// How a `TurnEnded` and a `UserMessage` line begin. Relies on serde
/// writing `LogRecord`'s internal tag first.
const TURN_ENDED: &[u8] = br#"{"type":"turn_ended""#;
const USER_MESSAGE: &[u8] = br#"{"type":"user_message""#;

/// One row of the listing. Every transcript is summarised at session start,
/// so only `TurnEnded` lines and the first `UserMessage` are parsed; the
/// rest are skipped by their tag.
fn summarise(path: &Path, id: SessionId) -> Option<SessionSummary> {
    let file = File::open(path).ok()?;
    let mut lines = lines(file);

    let header: SessionHeader = serde_json::from_slice(&lines.next()?).ok()?;
    if header.version != HISTORY_VERSION {
        return None;
    }

    let mut title = None;
    let mut turns = 0;
    for line in lines {
        if line.starts_with(TURN_ENDED) {
            // Parsed, not only matched: a torn `TurnEnded` does not end its
            // turn for `load`, so it must not count one here.
            if serde_json::from_slice::<LogRecord>(&line).is_ok() {
                turns += 1;
            }
        } else if title.is_none() && line.starts_with(USER_MESSAGE) {
            if let Ok(LogRecord::UserMessage { text, .. }) = serde_json::from_slice(&line) {
                title = Some(derive_title(&text));
            }
        }
    }

    // Nothing to resume, so nothing to list; this also hides header-only
    // files from older builds.
    if turns == 0 {
        return None;
    }

    Some(SessionSummary {
        id,
        started_at: header.started_at,
        // A blank first message, or none: only a damaged transcript has either.
        title: title.flatten().unwrap_or_else(|| "(untitled)".into()),
        turns,
    })
}

/// A message's first non-blank line, cut to `TITLE_MAX` chars; `None` when
/// every line is blank. Never summarise it with the model: that would spend
/// the developer's tokens.
fn derive_title(text: &str) -> Option<String> {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(if first.chars().count() <= TITLE_MAX {
        first.to_string()
    } else {
        let head: String = first.chars().take(TITLE_MAX).collect();
        format!("{head}…")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_core::{StepId, ToolCall, TurnEndReason, TurnId};
    use tempfile::tempdir;

    fn header() -> SessionHeader {
        SessionHeader {
            version: HISTORY_VERSION,
            started_at: 1_700_000_000,
            cwd: "/tmp/p".into(),
            model: "m".into(),
        }
    }

    fn turn(n: u64, text: &str) -> Vec<LogRecord> {
        vec![
            LogRecord::TurnStarted { turn_id: TurnId(n) },
            LogRecord::UserMessage {
                turn_id: TurnId(n),
                text: text.into(),
            },
            LogRecord::AssistantMessage {
                turn_id: TurnId(n),
                step_id: StepId(n),
                text: "sure".into(),
            },
            LogRecord::TurnEnded {
                turn_id: TurnId(n),
                reason: TurnEndReason::EndTurn,
            },
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

    /// Counts the `write` calls it is given, the way `O_APPEND` sees them.
    #[derive(Default)]
    struct Writes(Vec<Vec<u8>>);

    impl Write for Writes {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.push(buf.to_vec());
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Regression: `writeln!` was two writes, which a second appending
    /// process could land between.
    #[test]
    fn a_line_is_one_write_newline_included() {
        let mut out = Writes::default();
        write_line(&mut out, &header()).unwrap();
        for record in turn(1, "hello") {
            write_line(&mut out, &record).unwrap();
        }
        assert_eq!(out.0.len(), 1 + 4, "one write per line");
        for write in &out.0 {
            assert_eq!(write.iter().filter(|b| **b == b'\n').count(), 1);
            assert_eq!(
                write.last(),
                Some(&b'\n'),
                "the newline is in the same write"
            );
        }
    }

    /// Regression: reading lines as text stopped at a non-UTF-8 torn record,
    /// losing every later turn.
    #[test]
    fn a_record_torn_mid_character_costs_that_line_and_nothing_after_it() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000009-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        for record in turn(1, "before") {
            store.append(&record).unwrap();
        }
        drop(store);

        let path = transcript_path(dir.path(), &id);
        let mut raw = fs::read(&path).unwrap();
        // "café", cut after the first byte of `é`.
        raw.extend_from_slice(b"{\"type\":\"user_message\",\"turn_id\":2,\"text\":\"caf\xC3");
        fs::write(&path, raw).unwrap();

        let store = HistoryStore::reopen(dir.path(), &id).unwrap();
        for record in turn(3, "after") {
            store.append(&record).unwrap();
        }

        let loaded = load(dir.path(), &id).unwrap();
        assert_eq!(loaded, [turn(1, "before"), turn(3, "after")].concat());
        assert_eq!(
            list(dir.path())[0].turns,
            2,
            "the listing reads past it too"
        );
    }

    /// `summarise` matches lines by their opening bytes.
    #[test]
    fn the_tags_the_listing_matches_are_the_ones_serde_writes() {
        let [started, said, _, ended] = &turn(1, "hi")[..] else {
            unreachable!()
        };
        let line = |r: &LogRecord| serde_json::to_vec(r).unwrap();
        assert!(line(ended).starts_with(TURN_ENDED));
        assert!(line(said).starts_with(USER_MESSAGE));
        assert!(!line(started).starts_with(TURN_ENDED));
    }

    /// aldwin-history.md Step 2. A half-written line simulates a kill, the
    /// only damage an append-only file takes.
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

        assert_eq!(
            load(dir.path(), &id).unwrap(),
            records,
            "the intact turn survives the torn tail"
        );
    }

    /// aldwin-history.md Step 3: a kill between a tool call and its result
    /// must not resume into an unmatched tool-use block.
    #[test]
    fn an_unfinished_turn_is_dropped_back_to_the_last_finished_one() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000003-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();

        let finished = turn(1, "first");
        for record in &finished {
            store.append(record).unwrap();
        }
        // A second turn killed after a tool call.
        store
            .append(&LogRecord::TurnStarted { turn_id: TurnId(2) })
            .unwrap();
        store
            .append(&LogRecord::UserMessage {
                turn_id: TurnId(2),
                text: "second".into(),
            })
            .unwrap();
        store
            .append(&LogRecord::ToolUse {
                turn_id: TurnId(2),
                step_id: StepId(2),
                call: ToolCall {
                    id: "c1".into(),
                    name: "read".into(),
                    input: serde_json::json!({}),
                },
            })
            .unwrap();

        let loaded = load(dir.path(), &id).unwrap();
        assert_eq!(loaded, finished, "the half-turn is dropped whole");
        assert!(
            !loaded
                .iter()
                .any(|r| matches!(r, LogRecord::ToolUse { .. })),
            "no tool call survives without its result"
        );
    }

    #[test]
    fn a_transcript_whose_first_turn_never_finished_loads_as_nothing() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000004-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        store
            .append(&LogRecord::TurnStarted { turn_id: TurnId(1) })
            .unwrap();
        store
            .append(&LogRecord::UserMessage {
                turn_id: TurnId(1),
                text: "hi".into(),
            })
            .unwrap();

        assert!(
            load(dir.path(), &id).unwrap().is_empty(),
            "half a turn is worse than none"
        );
    }

    /// aldwin-history.md Step 2: history must never fail a turn.
    #[test]
    fn an_unwritable_history_directory_is_an_error_not_a_panic() {
        let dir = tempdir().unwrap();
        let blocked = dir.path().join("wall");
        fs::write(&blocked, "not a directory").unwrap();

        let result =
            HistoryStore::create(&blocked.join("history"), &SessionId("x".into()), &header());
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
            let store = HistoryStore::create(
                dir.path(),
                &id,
                &SessionHeader {
                    started_at,
                    ..header()
                },
            )
            .unwrap();
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

    /// Regression: a first message of blank lines titled the session "".
    #[test]
    fn a_blank_first_message_is_untitled_never_an_empty_title() {
        let dir = tempdir().unwrap();
        let store =
            HistoryStore::create(dir.path(), &SessionId("0000000030-1".into()), &header()).unwrap();
        for record in turn(1, "\n  \n")
            .into_iter()
            .chain(turn(2, "add rate limiting"))
        {
            store.append(&record).unwrap();
        }

        assert_eq!(list(dir.path())[0].title, "(untitled)");
    }

    #[test]
    fn a_session_that_says_nothing_writes_no_file_at_all() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000060-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        drop(store);

        assert!(
            !transcript_path(dir.path(), &id).exists(),
            "no transcript for a session with no records"
        );
        assert!(list(dir.path()).is_empty());
    }

    #[test]
    fn a_session_is_listed_if_and_only_if_it_can_be_resumed() {
        let dir = tempdir().unwrap();

        let finished = SessionId("0000000070-1".into());
        let store = HistoryStore::create(dir.path(), &finished, &header()).unwrap();
        for record in turn(1, "answered") {
            store.append(&record).unwrap();
        }
        drop(store);

        let unfinished = SessionId("0000000080-1".into());
        let store = HistoryStore::create(dir.path(), &unfinished, &header()).unwrap();
        store
            .append(&LogRecord::TurnStarted { turn_id: TurnId(2) })
            .unwrap();
        store
            .append(&LogRecord::UserMessage {
                turn_id: TurnId(2),
                text: "cut off".into(),
            })
            .unwrap();
        drop(store);

        for id in [&finished, &unfinished] {
            let listed = list(dir.path()).iter().any(|s| s.id == *id);
            let resumable = !load(dir.path(), id).unwrap().is_empty();
            assert_eq!(
                listed, resumable,
                "{id} is listed={listed} but resumable={resumable}"
            );
        }
        assert_eq!(list(dir.path()).len(), 1);
    }

    #[test]
    fn a_header_only_transcript_from_an_older_build_is_not_listed() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000090-1".into());
        fs::create_dir_all(dir.path()).unwrap();
        let line = serde_json::to_string(&header()).unwrap();
        fs::write(transcript_path(dir.path(), &id), format!("{line}\n")).unwrap();

        assert!(
            list(dir.path()).is_empty(),
            "a session with no completed turn is not a session to resume"
        );
    }

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
        store
            .append(&LogRecord::TurnStarted { turn_id: TurnId(3) })
            .unwrap();
        drop(store);

        let listed = &list(dir.path())[0];
        assert_eq!(listed.turns, 2, "the third turn never finished");
        assert_eq!(
            load(dir.path(), &id)
                .unwrap()
                .iter()
                .filter(|r| matches!(r, LogRecord::TurnEnded { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn a_session_from_a_future_schema_is_skipped_rather_than_failing_the_listing() {
        let dir = tempdir().unwrap();
        let good = SessionId("0000000030-1".into());
        let store = HistoryStore::create(dir.path(), &good, &header()).unwrap();
        for record in turn(1, "readable") {
            store.append(&record).unwrap();
        }
        let future = SessionHeader {
            version: HISTORY_VERSION + 1,
            ..header()
        };
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

        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "one conversation is one file"
        );
        let loaded = load(dir.path(), &id).unwrap();
        assert_eq!(loaded.len(), 8);
        assert_eq!(list(dir.path())[0].turns, 2);
    }

    /// After `/resume`, a crashed half turn sits mid-file, where dropping
    /// only the tail would miss it.
    #[test]
    fn a_half_turn_left_by_a_crash_stays_dropped_after_the_session_is_continued() {
        let dir = tempdir().unwrap();
        let id = SessionId("0000000055-1".into());
        let store = HistoryStore::create(dir.path(), &id, &header()).unwrap();
        for record in turn(1, "first") {
            store.append(&record).unwrap();
        }
        store
            .append(&LogRecord::TurnStarted { turn_id: TurnId(2) })
            .unwrap();
        store
            .append(&LogRecord::ToolUse {
                turn_id: TurnId(2),
                step_id: StepId(2),
                call: ToolCall {
                    id: "c1".into(),
                    name: "read".into(),
                    input: serde_json::json!({}),
                },
            })
            .unwrap();
        drop(store);
        // Killed mid-write: no final newline.
        let path = transcript_path(dir.path(), &id);
        let mut raw = fs::read_to_string(&path).unwrap();
        raw.push_str("{\"type\":\"tool_res");
        fs::write(&path, raw).unwrap();

        let store = HistoryStore::reopen(dir.path(), &id).unwrap();
        for record in turn(2, "again") {
            store.append(&record).unwrap();
        }
        drop(store);

        let expected: Vec<LogRecord> = turn(1, "first")
            .into_iter()
            .chain(turn(2, "again"))
            .collect();
        assert_eq!(
            load(dir.path(), &id).unwrap(),
            expected,
            "the torn line must not swallow the record after it"
        );
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
        assert!(
            slug.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{slug}"
        );
    }
}
