//! The conversation log's entries, one per row group the design draws.
//!
//! Append-only (aldwin-tui.md) except four updated in place: `Work` gains
//! items, `Thinking` its text and time, `Plan` is replaced, `Question` gains
//! its answer.

use aldwin_core::{Failure, FailureKind, PlanStep, RetryInfo, ReviewOutcome};

/// One entry in the conversation log.
#[derive(Debug, Clone, PartialEq)]
pub enum LogEntry {
    /// The developer's message, echoed on the `tint` ground with a `›`.
    UserMessage {
        /// The message as it was sent.
        text: String,
    },
    /// One thinking block, a summary (`Thought for 12s`) that Space opens
    /// to the reasoning (ADR 0015). Updated in place while it streams.
    Thinking {
        /// The reasoning so far.
        text: String,
        /// How long it took, once it has ended.
        took: Took,
        /// Whether the disclosure is open.
        open: bool,
    },
    /// The agent's prose, as markdown.
    AssistantText {
        /// The text so far; streaming deltas append to it.
        text: String,
    },
    /// One step's work, a summary (`Read 3 files · Ran 1 command`) that
    /// Space opens.
    Work {
        /// The step's calls, in the order asked.
        items: Vec<WorkItem>,
        /// Whether the disclosure is open.
        open: bool,
    },
    /// The plan as the `plan` tool last stated it; one per turn, replaced in
    /// place.
    Plan {
        /// The steps, in order.
        steps: Vec<PlanStep>,
    },
    /// A question asked through `ask`, drawn as the question, ` · `, and
    /// the answer.
    Question {
        /// The question as the agent put it.
        question: String,
        /// The answer, once one is given.
        answer: Option<String>,
    },
    /// A review's result row: `✓ Saved 3 files · 1 comment resolved`.
    Review {
        /// The decision and what it wrote.
        outcome: ReviewOutcome,
    },
    /// A message from outside the turn (a slash command's answer, a startup
    /// fact), in `label2`.
    Notice {
        /// The sentence to show.
        message: String,
    },
    /// A stop requested but not done; the turn's end replaces it with
    /// `Stopped.`.
    Stopping {
        /// The sentence to show.
        message: String,
    },
    /// An errored or cancelled turn, or a provider retry: a sentence in
    /// `label` with the detail disclosed below (ADR 0009 §5: no red, no
    /// glyph).
    Failure {
        /// What failed.
        message: String,
        /// The underlying error text.
        detail: Option<String>,
        /// Whether the detail is disclosed.
        open: bool,
    },
    /// The blank row after a turn that ended cleanly.
    TurnBreak,
    /// Messages sent while the turn runs, one band marked `Queued` (frame
    /// K). Never in `App::log`: `Transcript::sync` draws it after the last
    /// entry.
    Queued {
        /// The messages, in the order sent.
        messages: Vec<String>,
    },
}

impl LogEntry {
    /// Whether Space can open something here.
    pub(crate) fn has_details(&self) -> bool {
        matches!(
            self,
            LogEntry::Work { .. }
                | LogEntry::Thinking { .. }
                | LogEntry::Failure {
                    detail: Some(_),
                    ..
                }
        )
    }

    /// A provider retry as a `Failure`, the provider's message as detail.
    pub fn retry(info: &RetryInfo) -> Self {
        let message = match info.status {
            Some(status) => format!(
                "{} answered {status}; trying again ({}).",
                info.provider,
                ordinal(info.attempt)
            ),
            None => format!(
                "{} did not answer; trying again ({}).",
                info.provider,
                ordinal(info.attempt)
            ),
        };
        LogEntry::Failure {
            message,
            detail: Some(info.message.clone()),
            open: false,
        }
    }

    /// A failed turn as a `Failure`: what happened and what to do, the error
    /// itself the detail (ADR 0009 §5).
    pub fn failed_turn(failure: Failure) -> Self {
        let message = match failure.kind {
            // Already the sentence: nothing more to disclose.
            FailureKind::NotSent => {
                return LogEntry::Failure {
                    message: failure.message,
                    detail: None,
                    open: false,
                }
            }
            FailureKind::Network => {
                "The provider could not be reached. Check your connection, then send again."
            }
            FailureKind::Interrupted => {
                "The reply was cut off partway. Send again to have it retried."
            }
            // A timeout retried to the end is not a refusal.
            FailureKind::Exhausted {
                status: None | Some(408),
            } => "The provider kept failing. Send again in a moment.",
            FailureKind::Provider { status }
            | FailureKind::Exhausted {
                status: Some(status),
            } => provider_sentence(status),
            FailureKind::Other => "The turn stopped before it finished. The detail says why.",
        };
        LogEntry::Failure {
            message: message.into(),
            detail: Some(failure.message),
            open: false,
        }
    }
}

/// How long a thinking block took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Took {
    /// Still streaming.
    Running,
    /// Ended, in whole seconds.
    Seconds(u64),
    /// Ended with no time: a transcript from before ADR 0015, a block the
    /// provider closed without opening, or a thought the turn cut off.
    Unknown,
}

impl Took {
    /// The disclosure's summary: `Thinking`, `Thought for 12s`,
    /// `Thought for 2m 05s`, or `Thought`.
    pub(crate) fn summary(self) -> String {
        match self {
            Took::Running => "Thinking".into(),
            Took::Seconds(s) if s < 60 => format!("Thought for {s}s"),
            Took::Seconds(s) => format!("Thought for {}m {:02}s", s / 60, s % 60),
            Took::Unknown => "Thought".into(),
        }
    }
}

impl From<Option<u64>> for Took {
    fn from(seconds: Option<u64>) -> Self {
        seconds.map_or(Took::Unknown, Took::Seconds)
    }
}

fn ordinal(n: u32) -> String {
    match n {
        1 => "first try".into(),
        2 => "second try".into(),
        3 => "third try".into(),
        n => format!("try {n}"),
    }
}

/// What a call did, as an outcome, never a tool name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// A file was read.
    Read,
    /// An edit was staged for the review.
    Changed,
    /// A shell command was run.
    Ran,
    /// A lookup. Eight characters, the most a word can have to fit the
    /// 9-cell `--detail-col` with its gap.
    Searched,
    /// Any other tool, such as an MCP server's.
    Used,
}

impl Verb {
    /// The verb as the summary and the disclosure print it.
    pub fn word(self) -> &'static str {
        match self {
            Verb::Read => "Read",
            Verb::Changed => "Changed",
            Verb::Ran => "Ran",
            Verb::Searched => "Searched",
            Verb::Used => "Used",
        }
    }

    /// The verb as the working line says it while the call runs, lower case.
    pub(crate) fn doing(self) -> &'static str {
        match self {
            Verb::Read => "reading",
            Verb::Changed => "changing",
            Verb::Ran => "running",
            Verb::Searched => "looking up",
            Verb::Used => "using",
        }
    }

    /// What the summary counts: `Read 3 files`, `Ran 1 command`.
    fn noun(self) -> &'static str {
        match self {
            Verb::Read | Verb::Changed => "file",
            Verb::Ran => "command",
            Verb::Searched => "symbol",
            Verb::Used => "tool",
        }
    }

    /// A finished call's fact: a line count for a read, `ok` for a run,
    /// `staged` for an edit; on failure or another verb, the result's first
    /// line.
    pub fn fact(self, content: &str, failed: bool) -> String {
        match (self, failed) {
            (Verb::Read, false) => plural(content.lines().count(), "line"),
            (Verb::Ran, false) => "ok".into(),
            (Verb::Changed, false) => "staged".into(),
            _ => first_line(content, 24),
        }
    }
}

/// One call inside a `Work` disclosure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItem {
    /// The tool call's id; later events find the row by it.
    pub call_id: String,
    /// What kind of work the call was.
    pub verb: Verb,
    /// A path, a command or a symbol.
    pub target: String,
    /// The result fact (`412 lines`, `exit 1`), drawn right-flush; `None`
    /// until the call finishes.
    pub fact: Option<String>,
    /// Whether the result was an error; counted in the summary.
    pub failed: bool,
}

impl WorkItem {
    /// The verb and target for a tool call; the only place a tool name is
    /// read. `None` for `plan` and `ask`, which have their own entries.
    pub fn describe(name: &str, input: &serde_json::Value) -> Option<(Verb, String)> {
        let field = |key: &str| {
            input
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        Some(match name {
            "read" => (Verb::Read, field("path")),
            "edit" => (Verb::Changed, field("path")),
            "run" => (Verb::Ran, field("command")),
            "explain" => {
                let path = field("path");
                (
                    Verb::Searched,
                    if path.is_empty() {
                        field("query")
                    } else {
                        path
                    },
                )
            }
            "plan" | "ask" => return None,
            other => (Verb::Used, other.to_string()),
        })
    }
}

/// A `Work` entry's summary: one counted clause per verb, in first-seen
/// order, joined with ` · `.
pub fn summarise_work(items: &[WorkItem]) -> String {
    let mut counts: Vec<(Verb, usize, usize)> = Vec::new();
    for item in items {
        let at = match counts.iter().position(|(verb, ..)| *verb == item.verb) {
            Some(at) => at,
            None => {
                counts.push((item.verb, 0, 0));
                counts.len() - 1
            }
        };
        counts[at].1 += 1;
        counts[at].2 += usize::from(item.failed);
    }
    counts
        .into_iter()
        .map(|(verb, n, failed)| {
            let clause = format!("{} {}", verb.word(), plural(n, verb.noun()));
            if failed > 0 {
                format!("{clause}, {failed} failed")
            } else {
                clause
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `1 file`, `3 files`: every count the app writes goes through this.
pub(crate) fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The sentence for the status a provider refused a request with. No digit:
/// the status is the detail's.
fn provider_sentence(status: u16) -> &'static str {
    match status {
        401 | 403 => {
            "The provider did not accept your key or account. The detail says which, and what to do."
        }
        429 => "The provider is limiting requests right now. Wait a moment, then send again.",
        500..=599 => "The provider had a problem on its side. Send again in a moment.",
        _ => "The provider turned the request down. The detail says why.",
    }
}

/// The first line of `content`, elided to `max` characters.
pub fn first_line(content: &str, max: usize) -> String {
    let line = content.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        line.to_string()
    } else {
        let cut: String = line.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_core::LlmError;

    #[test]
    fn a_thought_says_how_long_it_took_in_seconds_then_minutes() {
        let cases = [
            (Took::Running, "Thinking"),
            (Took::Seconds(1), "Thought for 1s"),
            (Took::Seconds(59), "Thought for 59s"),
            (Took::Seconds(60), "Thought for 1m 00s"),
            (Took::Seconds(125), "Thought for 2m 05s"),
            (Took::Unknown, "Thought"),
        ];
        for (took, summary) in cases {
            assert_eq!(took.summary(), summary, "{took:?}");
        }
        assert_eq!(Took::from(Some(12)), Took::Seconds(12));
        assert_eq!(Took::from(None), Took::Unknown);
    }

    fn provider(status: u16, message: &str) -> LlmError {
        LlmError::Provider {
            status,
            message: message.into(),
        }
    }

    fn exhausted(status: Option<u16>, message: &str) -> LlmError {
        LlmError::Terminal {
            attempts: 4,
            status,
            message: message.into(),
        }
    }

    /// No status, body or retry count in the sentence; those are the detail.
    #[test]
    fn a_failure_reads_as_what_happened_and_what_to_do() {
        let cases = [
            (
                provider(400, r#"{"error":{"message":"unknown model gpt-5"}}"#),
                "The provider turned the request down. The detail says why.",
            ),
            (
                provider(401, "invalid x-api-key"),
                "The provider did not accept your key or account. The detail says which, and what to do.",
            ),
            (
                provider(429, "rate_limit_error"),
                "The provider is limiting requests right now. Wait a moment, then send again.",
            ),
            (
                provider(529, "overloaded_error"),
                "The provider had a problem on its side. Send again in a moment.",
            ),
            (
                LlmError::Network("connection refused".into()),
                "The provider could not be reached. Check your connection, then send again.",
            ),
            (
                LlmError::StreamInterrupted("unexpected EOF".into()),
                "The reply was cut off partway. Send again to have it retried.",
            ),
            (
                exhausted(Some(529), "overloaded"),
                "The provider had a problem on its side. Send again in a moment.",
            ),
            (
                exhausted(Some(429), "rate_limit_error"),
                "The provider is limiting requests right now. Wait a moment, then send again.",
            ),
            (
                exhausted(Some(401), "invalid x-api-key"),
                "The provider did not accept your key or account. The detail says which, and what to do.",
            ),
            (
                exhausted(Some(408), "request timeout"),
                "The provider kept failing. Send again in a moment.",
            ),
            (
                exhausted(None, "idle timeout"),
                "The provider kept failing. Send again in a moment.",
            ),
        ];
        for (error, sentence) in cases {
            let detail = error.to_string();
            assert_eq!(
                LogEntry::failed_turn(error.into()),
                LogEntry::Failure {
                    message: sentence.into(),
                    detail: Some(detail),
                    open: false,
                }
            );
            assert!(!sentence.chars().any(|c| c.is_ascii_digit()), "{sentence}");
        }
        assert!(matches!(
            LogEntry::failed_turn(Failure::other("command channel closed")),
            LogEntry::Failure { message, .. } if message == "The turn stopped before it finished. The detail says why."
        ));
    }

    /// Guards against "the provider kept failing" for a provider never
    /// asked.
    #[test]
    fn a_turn_aldwin_could_not_send_leads_with_aldwins_own_sentence() {
        let said = "No x.ai account is connected and XAI_API_KEY is not set. Connect one with /connect xai, or set XAI_API_KEY and start Aldwin again.";
        assert_eq!(
            LogEntry::failed_turn(LlmError::NotSent(said.into()).into()),
            LogEntry::Failure {
                message: said.into(),
                detail: None,
                open: false,
            }
        );
        // A provider's text that reads like Aldwin's own is still the
        // provider's.
        assert!(matches!(
            LogEntry::failed_turn(exhausted(None, "No model is configured yet.").into()),
            LogEntry::Failure { message, .. } if message == "The provider kept failing. Send again in a moment."
        ));
    }

    use serde_json::json;

    #[test]
    fn calls_are_described_as_outcomes_not_tool_names() {
        assert_eq!(
            WorkItem::describe("read", &json!({"path": "src/x.rs"})),
            Some((Verb::Read, "src/x.rs".into()))
        );
        assert_eq!(
            WorkItem::describe("run", &json!({"command": "cargo test -q 2>&1 | tail"})),
            Some((Verb::Ran, "cargo test -q 2>&1 | tail".into())),
            "a run is the command it ran (ADR 0011 §2)"
        );
        assert_eq!(
            WorkItem::describe("explain", &json!({"query": "tower::limit"})),
            Some((Verb::Searched, "tower::limit".into()))
        );
        assert_eq!(
            WorkItem::describe("fs_read", &json!({})),
            Some((Verb::Used, "fs_read".into()))
        );
        assert_eq!(WorkItem::describe("plan", &json!({})), None);
        assert_eq!(WorkItem::describe("ask", &json!({})), None);
    }

    #[test]
    fn the_summary_counts_by_verb_in_first_seen_order() {
        let item = |verb, failed| WorkItem {
            call_id: "c".into(),
            verb,
            target: String::new(),
            fact: None,
            failed,
        };
        let items = vec![
            item(Verb::Read, false),
            item(Verb::Read, false),
            item(Verb::Ran, true),
            item(Verb::Read, false),
        ];
        assert_eq!(
            summarise_work(&items),
            "Read 3 files · Ran 1 command, 1 failed"
        );
        assert_eq!(summarise_work(&[item(Verb::Read, false)]), "Read 1 file");
    }

    #[test]
    fn facts_are_short_and_right() {
        assert_eq!(Verb::Read.fact("a\nb\nc", false), "3 lines");
        assert_eq!(Verb::Read.fact("a", false), "1 line");
        assert_eq!(Verb::Ran.fact("anything", false), "ok");
        assert_eq!(Verb::Ran.fact("exit 1\nstderr", true), "exit 1");
        assert_eq!(
            first_line(&"x".repeat(40), 10),
            format!("{}…", "x".repeat(9))
        );
    }
}
