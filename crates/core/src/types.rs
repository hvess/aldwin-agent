use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TURN: AtomicU64 = AtomicU64::new(1);
static NEXT_STEP: AtomicU64 = AtomicU64::new(1);
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StepId(pub u64);

/// One conversation's identity, and the stem of its transcript filename.
///
/// Three fields, each answering a different collision. Epoch seconds order
/// the directory — the field is fixed-width until the year 2286, so ids sort
/// lexicographically into start order and `list` can read a directory rather
/// than every header in it. The pid separates two Aldwins running in the
/// same project at the same time. The counter separates two sessions in *one*
/// process: `/clear` seals and opens a new one, and seconds alone would hand
/// two clears in the same second the same id — which, since transcripts are
/// opened for appending, silently merged two conversations into one file.
///
/// It is deliberately not a timestamp *for display*. The transcript header
/// carries `started_at` for that; this is a key.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn mint() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let seq = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        // The counter is fixed-width for the same reason the seconds are:
        // unpadded, the tenth mint in one second sorted ahead of the second.
        Self(format!("{secs:010}-{}-{seq:06}", std::process::id()))
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TurnId {
    pub fn next() -> Self { Self(NEXT_TURN.fetch_add(1, Ordering::Relaxed)) }
}

impl StepId {
    pub fn next() -> Self { Self(NEXT_STEP.fetch_add(1, Ordering::Relaxed)) }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role { User, Assistant }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id:    String,
    pub name:  String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id:  String,
    pub content:  String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text       { text: String },
    /// An extended-thinking block, kept verbatim with the signature the
    /// provider stamped it with.
    ///
    /// It is carried rather than dropped for two separate reasons. The first
    /// is correctness: when a turn that produced thinking goes on to call a
    /// tool, the provider requires the thinking block back — signature and
    /// all — on the assistant message that requested the call, and rejects
    /// the request without it. The second is that a step whose entire output
    /// was a thinking block used to reach the developer as a blank turn
    /// (14,096 tokens spent, nothing rendered, "Continue" typed by hand).
    /// See ADR 0006.
    Thinking   { text: String, signature: String },
    /// Thinking the provider encrypted rather than showed. Opaque to us and
    /// echoed back untouched, for the same wire-correctness reason as
    /// `Thinking` — there is nothing here to render.
    RedactedThinking { data: String },
    ToolUse    (ToolCall),
    ToolResult (ToolResult),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role:    Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: vec![ContentBlock::Text { text: text.into() }] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name:         String,
    pub description:  String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageStats {
    pub input_tokens:  u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheStats {
    pub cache_creation_input_tokens: u32,
    pub cache_read_input_tokens:     u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason { EndTurn, ToolUse }

// ── The plan, a question, and a review (ADR 0009) ───────────────────────────

/// Where one step of the plan stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    Running,
    Done,
}

/// One step of the plan: an outcome in plain words — *Count requests per
/// key*, never a command — and where it stands. The `plan` tool declares and
/// advances these; the TUI draws them as the design's `PlanStep` rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub text:  String,
    pub state: StepState,
}

/// A question the agent puts to the developer through the `ask` tool: one
/// line of question, one line of why, and a short list of answers. The
/// design's rule is that the list always carries a yes, a no and "Chat about
/// this"; the tool enforces the third and the prompt asks for the first two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    pub detail:   String,
    pub options:  Vec<String>,
}

/// The developer's answer to a [`Question`]: the option they chose, or —
/// for "Chat about this" — what they typed instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Chose { index: usize },
    Said { text: String },
}

/// One file of a staged changeset, as the review draws it: the whole file
/// before (`None` for a file that did not exist) and after every staged
/// edit to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path:   String,
    pub before: Option<String>,
    pub after:  String,
}

/// Everything a turn's edits have staged and nothing has written yet.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Changeset {
    pub files: Vec<ChangedFile>,
}

/// A comment left on a run of lines in the review, on the *after* side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub path:  String,
    /// Inclusive, 1-based line numbers in the file as it would be written.
    pub lines: (usize, usize),
    pub text:  String,
}

/// What the developer decided at a review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewDecision {
    /// Write every file. The one way a change reaches disk.
    Approve,
    /// Nothing is written; the comments go back to the agent and the
    /// changeset stays staged for the next review.
    Comment { comments: Vec<ReviewComment> },
    /// Nothing is written and the changeset is dropped.
    Discard,
}

/// How a review ended — the row the conversation keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewOutcome {
    Saved { files: Vec<String>, comments_resolved: usize },
    Commented { comments: usize },
    Discarded { files: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryInfo {
    pub provider: String,
    pub status:   Option<u16>,
    pub message:  String,
    pub attempt:  u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two sessions in one process — `/clear` seals and opens a new one, and
    /// two clears inside the same second must not name the same transcript.
    #[test]
    fn minted_session_ids_are_distinct_within_one_process() {
        let ids: Vec<SessionId> = (0..64).map(|_| SessionId::mint()).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "every mint is its own session");
    }

    /// More than ten, so the counter crosses a digit boundary — unpadded,
    /// `-10` sorted ahead of `-2` and this failed whenever it did.
    #[test]
    fn session_ids_sort_into_the_order_they_were_minted() {
        let mut ids: Vec<SessionId> = (0..24).map(|_| SessionId::mint()).collect();
        let minted = ids.clone();
        ids.sort();
        assert_eq!(ids, minted);
    }
}
