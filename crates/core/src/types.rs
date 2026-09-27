use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide, unlike turn and step ids: `/clear` mints a second session in
/// one process. See [`SessionId`].
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

/// One turn: a developer's message and every step the agent takes to answer
/// it. Minted by the `Agent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnId(pub u64);

/// One step: a single request to the model and the tool calls it asked
/// for. Minted by the `Agent`; unique across turns, not only within one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StepId(pub u64);

/// One conversation's identity, and the stem of its transcript filename.
///
/// `{secs:010}-{pid}-{seq:06}`. Zero-padded epoch seconds make ids sort
/// lexicographically into start order, which `list` relies on to avoid
/// reading every header. The pid separates concurrent processes in one
/// project; the counter separates sessions in one process (`/clear`), since
/// transcripts open for appending and a shared id merges two conversations.
///
/// A key, not a display timestamp: the transcript header's `started_at` is
/// that.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    /// A fresh id, distinct from every other minted in this process and
    /// ordered after them by the second it was minted in.
    pub fn mint() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let seq = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
        // Counter zero-padded so `-10` sorts after `-2`.
        Self(format!("{secs:010}-{}-{seq:06}", std::process::id()))
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who a message is from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// The developer — and tool results, which the model reads as input.
    User,
    /// The model.
    Assistant,
}

/// A tool call the model asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// The provider's id for the call, which its result must echo.
    pub id: String,
    /// The tool's name, as its `ToolDefinition` declares it.
    pub name: String,
    /// The arguments, as the model wrote them against the input schema.
    pub input: serde_json::Value,
}

/// What a tool call returned, as the model will read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// The `ToolCall::id` this answers.
    pub call_id: String,
    /// The output, or a sentence saying why there is none.
    pub content: String,
    /// Whether the call failed; the model still reads the result.
    pub is_error: bool,
}

/// One piece of a message's content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Prose.
    Text {
        /// The text.
        text: String,
    },
    /// An extended-thinking block, kept verbatim with its provider signature.
    ///
    /// Must not be dropped (ADR 0006): the provider rejects a tool-calling
    /// assistant message without its thinking block, and a thinking-only
    /// step would otherwise render as a blank turn.
    Thinking {
        /// The thinking text.
        text: String,
        /// The provider's signature over it, sent back verbatim.
        signature: String,
    },
    /// Thinking the provider encrypted. Opaque, never rendered, and echoed
    /// back untouched for the same reason as `Thinking`.
    RedactedThinking {
        /// The encrypted payload.
        data: String,
    },
    /// A tool call, on an assistant message.
    ToolUse(ToolCall),
    /// A tool call's result, on the user message that follows it.
    ToolResult(ToolResult),
}

/// One message of the conversation as sent to the provider, rebuilt from
/// the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Who it is from.
    pub role: Role,
    /// Its blocks, in order.
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub(crate) fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }
}

/// A tool as the model is told about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The name the model calls it by.
    pub name: String,
    /// What it does and when to use it, written for the model.
    pub description: String,
    /// The JSON Schema its input must satisfy.
    pub input_schema: serde_json::Value,
}

/// Tokens one step read and wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageStats {
    /// Tokens the step read.
    pub input_tokens: u32,
    /// Tokens the step wrote.
    pub output_tokens: u32,
}

/// What the provider's prompt cache did for one step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheStats {
    /// Input tokens written into the cache.
    pub cache_creation_input_tokens: u32,
    /// Input tokens served from the cache.
    pub cache_read_input_tokens: u32,
}

/// Why the model stopped writing a step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// It finished its answer.
    EndTurn,
    /// It is waiting on the results of the tool calls it made.
    ToolUse,
}

// The plan, a question, and a review: ADR 0009.

/// Where one step of the plan stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Not started.
    Pending,
    /// Being worked on now.
    Running,
    /// Finished.
    Done,
}

/// One step of the plan and where it stands. Declared and advanced by the
/// `plan` tool; drawn as the design's `PlanStep` rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    /// The outcome in plain words ("Count requests per key"), never a command.
    pub text: String,
    /// Where it stands.
    pub state: StepState,
}

/// A question from the `ask` tool: its options offer a positive, a negative
/// and "Chat about this". The tool appends the last and requires two others;
/// which is positive the prompt asks for (aldwin-tools.md Decisions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// One line of question.
    pub question: String,
    /// One line of why it is asked.
    pub detail: String,
    /// The answers on offer, ending with "Chat about this".
    pub options: Vec<String>,
}

impl Question {
    /// The row every question ends with; the design's copy.
    pub const CHAT_ABOUT_THIS: &'static str = "Chat about this";

    /// Whether `option` is that row, case-insensitively. The tool and the
    /// TUI must both use this comparison.
    pub fn is_chat_about_this(option: &str) -> bool {
        option.trim().eq_ignore_ascii_case(Self::CHAT_ABOUT_THIS)
    }
}

/// The developer's answer to a [`Question`]: an option, or text typed after
/// "Chat about this".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    /// They chose one of the options.
    Chose {
        /// Its index in `Question::options`.
        index: usize,
    },
    /// They chose "Chat about this" and typed a reply.
    Said {
        /// What they typed.
        text: String,
    },
}

impl Answer {
    const CHOSE: &'static str = "The developer chose: ";
    const SAID: &'static str = "The developer said: ";

    /// The tool result this answer becomes for the model, or `None` for a
    /// choice that is not one of `options`.
    pub fn to_result(&self, options: &[String]) -> Option<String> {
        match self {
            Answer::Chose { index } => options.get(*index).map(|o| format!("{}{o}", Self::CHOSE)),
            Answer::Said { text } => Some(format!("{}{text}", Self::SAID)),
        }
    }

    /// The developer's words from a `to_result` string, as the transcript
    /// shows them.
    pub fn words_of(result: &str) -> &str {
        result
            .strip_prefix(Self::CHOSE)
            .or_else(|| result.strip_prefix(Self::SAID))
            .unwrap_or(result)
    }
}

/// One file of a staged changeset, as the review draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// The file's path.
    pub path: String,
    /// Its whole contents before the changeset, or `None` if it is new.
    pub before: Option<String>,
    /// Its whole contents after every staged edit.
    pub after: String,
}

/// Everything a turn's edits have staged and nothing has written yet.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Changeset {
    /// Each file the turn's edits touched, once.
    pub files: Vec<ChangedFile>,
}

/// A comment left on a run of lines in the review, on the *after* side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    /// The file the lines are in.
    pub path: String,
    /// Inclusive, 1-based line numbers in the file as it would be written.
    pub lines: (usize, usize),
    /// What the developer wrote.
    pub text: String,
}

/// What the developer decided at a review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewDecision {
    /// Write every file; the only way a change reaches disk.
    Approve,
    /// Nothing is written; the comments go back to the agent and the
    /// changeset stays staged for the next review.
    Comment {
        /// The comments, each on a run of lines.
        comments: Vec<ReviewComment>,
    },
    /// Nothing is written and the changeset is dropped.
    Discard,
}

/// How a review ended — the row the conversation keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewOutcome {
    /// Approved: every file was written.
    Saved {
        /// The paths written.
        files: Vec<String>,
        /// How many comments from earlier reviews this changeset answered.
        comments_resolved: usize,
    },
    /// Comments went back to the agent; nothing was written.
    Commented {
        /// How many comments were left.
        comments: usize,
    },
    /// The changeset was dropped; nothing was written.
    Discarded {
        /// The paths that would have been written.
        files: Vec<String>,
    },
}

/// A failed attempt at a provider request that is being retried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryInfo {
    /// The provider being retried.
    pub provider: String,
    /// The HTTP status of the failure, or `None` if there was no answer.
    pub status: Option<u16>,
    /// What went wrong.
    pub message: String,
    /// Which attempt failed, counting from 1.
    pub attempt: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins that two `/clear`s in one second do not share a transcript.
    #[test]
    fn minted_session_ids_are_distinct_within_one_process() {
        let ids: Vec<SessionId> = (0..64).map(|_| SessionId::mint()).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "every mint is its own session");
    }

    /// More than ten mints, so the counter crosses a digit boundary.
    #[test]
    fn session_ids_sort_into_the_order_they_were_minted() {
        let mut ids: Vec<SessionId> = (0..24).map(|_| SessionId::mint()).collect();
        let minted = ids.clone();
        ids.sort();
        assert_eq!(ids, minted);
    }
}
