//! The conversation log's entries, one per row group the design draws.
//!
//! Append-only (aldwin-tui.md) except three updated in place: `Work` gains
//! items, `Plan` is replaced, `Question` gains its answer.

use std::borrow::Cow;

use aldwin_core::{PlanStep, RetryInfo, ReviewOutcome};

/// One entry in the conversation log.
#[derive(Debug, Clone, PartialEq)]
pub enum LogEntry {
    /// The developer's message, echoed on the `tint` ground with a `›`.
    UserMessage {
        /// The message as it was sent.
        text: String,
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
}

impl LogEntry {
    /// Whether Space can open something here.
    pub(crate) fn has_details(&self) -> bool {
        matches!(
            self,
            LogEntry::Work { .. }
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

/// The sentence a failed turn leads with: what happened and what to do.
/// Status, body and retries stay in the detail (ADR 0009 §5).
///
/// Parses `LlmError`'s `Display` (`crates/core/src/client.rs`); keep the
/// prefixes in sync. Open-tasks 1 would carry the kind instead.
///
/// Zero attempts means Aldwin sent nothing (no model, or ADR 0012's no
/// account and no key), and the message is already the sentence.
pub fn failure_sentence(error: &str) -> Cow<'_, str> {
    match error.strip_prefix("terminal error after 0 attempts: ") {
        Some(said) => Cow::Borrowed(said),
        None => Cow::Borrowed(provider_sentence(error)),
    }
}

/// The sentence for a provider's error. Separate from [`failure_sentence`]
/// so the zero-attempt rule applies only to the whole error, never to a
/// nested provider message.
fn provider_sentence(error: &str) -> &'static str {
    const FALLBACK: &str = "The turn stopped before it finished. The detail says why.";
    if error.starts_with("network error:") {
        return "The provider could not be reached. Check your connection, then send again.";
    }
    if error.starts_with("stream interrupted:") {
        return "The reply was cut off partway. Send again to have it retried.";
    }
    if let Some(rest) = error.strip_prefix("terminal error after ") {
        // Retries exhausted: use the last error's sentence if it has one.
        return match rest
            .split_once(": ")
            .map(|(_, last)| provider_sentence(last))
        {
            Some(sentence) if sentence != FALLBACK => sentence,
            _ => "The provider kept failing. Send again in a moment.",
        };
    }
    let status = error
        .strip_prefix("provider error ")
        .and_then(|rest| rest.split(':').next())
        .and_then(|s| s.trim().parse::<u16>().ok());
    match status {
        Some(401 | 403) => {
            "The provider did not accept your key or account. The detail says which, and what to do."
        }
        Some(429) => "The provider is limiting requests right now. Wait a moment, then send again.",
        Some(500..=599) => "The provider had a problem on its side. Send again in a moment.",
        Some(_) => "The provider turned the request down. The detail says why.",
        None => FALLBACK,
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

    /// No status, body or retry count in the sentence; those are the detail.
    #[test]
    fn a_failure_reads_as_what_happened_and_what_to_do() {
        let cases = [
            (
                "provider error 400: {\"error\":{\"message\":\"unknown model gpt-5\"}}",
                "The provider turned the request down. The detail says why.",
            ),
            (
                "provider error 401: invalid x-api-key",
                "The provider did not accept your key or account. The detail says which, and what to do.",
            ),
            (
                "provider error 429: rate_limit_error",
                "The provider is limiting requests right now. Wait a moment, then send again.",
            ),
            (
                "provider error 529: overloaded_error",
                "The provider had a problem on its side. Send again in a moment.",
            ),
            (
                "network error: connection refused",
                "The provider could not be reached. Check your connection, then send again.",
            ),
            (
                "stream interrupted: unexpected EOF",
                "The reply was cut off partway. Send again to have it retried.",
            ),
            (
                "terminal error after 3 attempts: provider error 529: overloaded",
                "The provider had a problem on its side. Send again in a moment.",
            ),
            (
                "terminal error after 3 attempts: something else",
                "The provider kept failing. Send again in a moment.",
            ),
            (
                "command channel closed",
                "The turn stopped before it finished. The detail says why.",
            ),
        ];
        for (error, sentence) in cases {
            assert_eq!(failure_sentence(error), sentence, "{error}");
            assert!(!sentence.chars().any(|c| c.is_ascii_digit()), "{sentence}");
        }
    }

    /// Guards against "the provider kept failing" for a provider never
    /// asked.
    #[test]
    fn a_turn_aldwin_could_not_send_leads_with_aldwins_own_sentence() {
        let said = "No x.ai account is connected and XAI_API_KEY is not set. Connect one with /connect xai, or set XAI_API_KEY and start Aldwin again.";
        assert_eq!(
            failure_sentence(&format!("terminal error after 0 attempts: {said}")),
            said
        );
        // A nested zero-attempt prefix after real attempts is the provider's.
        assert_eq!(
            failure_sentence(
                "terminal error after 4 attempts: terminal error after 0 attempts: pay here"
            ),
            "The provider kept failing. Send again in a moment."
        );
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
