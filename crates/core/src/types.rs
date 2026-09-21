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
        Self(format!("{secs:010}-{}-{seq}", std::process::id()))
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

    /// `list` orders a directory by filename before it reads any header.
    #[test]
    fn session_ids_sort_into_the_order_they_were_minted() {
        let mut ids: Vec<SessionId> = (0..8).map(|_| SessionId::mint()).collect();
        let minted = ids.clone();
        ids.sort();
        assert_eq!(ids, minted);
    }
}
