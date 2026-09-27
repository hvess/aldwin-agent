//! What the conversation is made of, one entry per row group the design
//! draws: the echoed prompt, a sentence from the agent, a disclosure of
//! work, the plan, a settled question, the row a review leaves behind, and
//! a failure said as a sentence.
//!
//! Append-only per aldwin-tui.md, with three entries that are updated in
//! place because the design draws them that way: `Work` gains items as
//! calls complete, `Plan` is replaced whenever the `plan` tool speaks, and
//! `Question` gains its answer.

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
    /// A sentence — or several — from the agent. Markdown: fences and
    /// tables render, everything else is prose.
    AssistantText {
        /// The text so far; streaming deltas append to it.
        text: String,
    },
    /// The work of one step, collapsed to a summary (`Read 3 files · Ran
    /// 6 tests`) that Space opens into exact paths and counts.
    Work {
        /// The calls of the step, in the order they were asked for.
        items: Vec<WorkItem>,
        /// Whether the disclosure is open onto its items.
        open: bool,
    },
    /// The plan as the `plan` tool last stated it. One per turn, replaced
    /// in place.
    Plan {
        /// The steps, in order, each with its state.
        steps: Vec<PlanStep>,
    },
    /// A question the agent asked through `ask`, and how it was answered
    /// once it was. Drawn as the question, then ` · ` and the answer.
    Question {
        /// The question as the agent put it.
        question: String,
        /// The answer, once one is given.
        answer: Option<String>,
    },
    /// What a review left behind: `✓ Saved 3 files · 1 comment resolved`.
    Review {
        /// What the developer decided and what it wrote.
        outcome: ReviewOutcome,
    },
    /// A message from outside the turn — the interceptor answering a slash
    /// command, a startup fact. A sentence in `label2`.
    Notice {
        /// The sentence to show.
        message: String,
    },
    /// Something failed: a turn that errored, a cancelled turn, a provider
    /// retry. A sentence in `label`, the detail one disclosure below
    /// (ADR 0009 §5: no red, no glyph).
    Failure {
        /// The sentence that says what failed.
        message: String,
        /// The underlying error text, folded below the sentence.
        detail: Option<String>,
        /// Whether the detail is disclosed.
        open: bool,
    },
    /// The turn ended cleanly — the blank row between turns.
    TurnBreak,
}

impl LogEntry {
    /// Whether Space has something here to open: a work disclosure, or a
    /// failure's detail.
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

    /// A provider retry, said as a failure with the provider's own message
    /// as the detail.
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

/// What a call did, in the design's words — outcomes, never tool names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// A file was read.
    Read,
    /// An edit was staged for the review.
    Changed,
    /// A shell command was run.
    Ran,
    /// The design's own verb for a lookup — and eight characters, which is
    /// what fits the 9-cell `--detail-col` with its gap.
    Searched,
    /// A tool the vocabulary has no word for: an MCP server's.
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

    /// The fact a finished call reports, from its result: a line count for
    /// a read, `ok` for a run that exited cleanly, otherwise the first line.
    pub fn fact(self, content: &str, failed: bool) -> String {
        match (self, failed) {
            (Verb::Read, false) => plural(content.lines().count(), "line"),
            (Verb::Ran, false) => "ok".into(),
            (Verb::Changed, false) => "staged".into(),
            _ => first_line(content, 24),
        }
    }
}

/// One call inside a `Work` disclosure — what it did, what it was pointed
/// at, and the fact that came back (`412 lines`, `7 matches`, `exit 1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItem {
    /// The tool call's id, which later events use to find this row.
    pub call_id: String,
    /// What kind of work the call was.
    pub verb: Verb,
    /// What it was pointed at: a path, a command, a symbol.
    pub target: String,
    /// Right-flush, once the call has finished.
    pub fact: Option<String>,
    /// Whether the call's result was an error, counted in the summary.
    pub failed: bool,
}

impl WorkItem {
    /// The verb and target for a tool call, from its name and input — the
    /// one place a tool's name is read. `None` for `plan` and `ask`, which
    /// are not work: the plan is drawn as itself and a question is its own
    /// row.
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

/// The collapsed summary of a `Work` entry: one clause per verb, counted,
/// joined with ` · `, in the order the verbs first appear. `Read 3 files ·
/// Ran 2 commands`.
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

/// `1 file`, `3 files` — every count the app writes.
pub(crate) fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The sentence a failed turn leads with: what happened, in plain words, and
/// what you can do about it (HIG, "Write clear error messages"). The error
/// itself — status, body, retries — is the detail one disclosure below,
/// exact and unabridged (ADR 0009 §5), so none of it is repeated here.
///
/// `TurnEndReason::Error` carries a string, so this reads `LlmError`'s
/// `Display` (`aldwin-core`, `client.rs`): `network error: …`,
/// `provider error 429: …`, `stream interrupted: …`, `terminal error after
/// N attempts: …`. Anything else — an error from the loop itself — gets the
/// plain fallback. Open-tasks 32 is carrying the kind instead.
///
/// Zero attempts is the one case whose message is the sentence already:
/// nothing was sent, because Aldwin itself could not reach a model — none
/// configured, or an account-or-key provider with neither (ADR 0012) — and
/// said so in words written for this row.
pub fn failure_sentence(error: &str) -> Cow<'_, str> {
    match error.strip_prefix("terminal error after 0 attempts: ") {
        Some(said) => Cow::Borrowed(said),
        None => Cow::Borrowed(provider_sentence(error)),
    }
}

/// The sentence for an error that came back from a provider. Kept apart
/// from [`failure_sentence`] so the zero-attempt rule applies to the whole
/// error only: a provider's own message that happens to begin the same way
/// is still a provider's message, not a sentence of Aldwin's.
fn provider_sentence(error: &str) -> &'static str {
    const FALLBACK: &str = "The turn stopped before it finished. The detail says why.";
    if error.starts_with("network error:") {
        return "The provider could not be reached. Check your connection, then send again.";
    }
    if error.starts_with("stream interrupted:") {
        return "The reply was cut off partway. Send again to have it retried.";
    }
    if let Some(rest) = error.strip_prefix("terminal error after ") {
        // Retries exhausted: say why they were needed, if the last one says.
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

    /// No status code, no body, no retry count on the surface — each is in
    /// the detail — and every sentence says what to do next.
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

    /// Nothing was sent, and the message is Aldwin's own: it is the
    /// sentence, whole — not "the provider kept failing", which would send
    /// the developer to wait for a provider that was never asked.
    #[test]
    fn a_turn_aldwin_could_not_send_leads_with_aldwins_own_sentence() {
        let said = "No x.ai account is connected and XAI_API_KEY is not set. Connect one with /connect xai, or set XAI_API_KEY and start Aldwin again.";
        assert_eq!(
            failure_sentence(&format!("terminal error after 0 attempts: {said}")),
            said
        );
        // A provider's message that begins the same way, after real
        // attempts, is still the provider's, and never the headline.
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
