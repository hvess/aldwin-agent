//! What the conversation is made of, one entry per row group the design
//! draws: the echoed prompt, a sentence from the agent, a disclosure of
//! work, the plan, a settled question, the row a review leaves behind, and
//! a failure said as a sentence.
//!
//! Append-only per aldwin-tui.md, with three entries that are updated in
//! place because the design draws them that way: `Work` gains items as
//! calls complete, `Plan` is replaced whenever the `plan` tool speaks, and
//! `Question` gains its answer.

use aldwin_core::{PlanStep, RetryInfo, ReviewOutcome};

/// One entry in the conversation log.
#[derive(Debug, Clone, PartialEq)]
pub enum LogEntry {
    /// The developer's message, echoed on the `tint` ground with a `›`.
    UserMessage { text: String },
    /// A sentence — or several — from the agent. Markdown: fences and
    /// tables render, everything else is prose.
    AssistantText { text: String },
    /// The work of one step, collapsed to a summary (`Read 3 files · Ran
    /// 6 tests`) that Space opens into exact paths and counts.
    Work { items: Vec<WorkItem>, open: bool },
    /// The plan as the `plan` tool last stated it. One per turn, replaced
    /// in place.
    Plan { steps: Vec<PlanStep> },
    /// A question the agent asked through `ask`, and how it was answered
    /// once it was. Drawn as the question, then ` · ` and the answer.
    Question {
        question: String,
        answer: Option<String>,
    },
    /// What a review left behind: `✓ Saved 3 files · 1 comment resolved`.
    Review { outcome: ReviewOutcome },
    /// A message from outside the turn — the interceptor answering a slash
    /// command, a startup fact. A sentence in `label2`.
    Notice { message: String },
    /// Something failed: a turn that errored, a cancelled turn, a provider
    /// retry. A sentence in `label`, the detail one disclosure below
    /// (ADR 0009 §5: no red, no glyph).
    Failure {
        message: String,
        detail: Option<String>,
        open: bool,
    },
    /// The turn ended cleanly — the blank row between turns.
    TurnBreak,
}

impl LogEntry {
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

/// One call inside a `Work` disclosure — the verb the design writes
/// (`Read`, `Searched`, `Ran`), what it was pointed at, and the fact that
/// came back (`412 lines`, `7 matches`, `exit 1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItem {
    pub call_id: String,
    pub verb: String,
    pub target: String,
    /// Right-flush, once the call has finished.
    pub fact: Option<String>,
    pub failed: bool,
}

impl WorkItem {
    /// The verb and target for a tool call, from its name and input. The
    /// vocabulary is the design's: outcomes, never tool names — `run` is
    /// what it ran, `explain` is what it looked up.
    pub fn describe(name: &str, input: &serde_json::Value) -> (String, String) {
        let s = |key: &str| {
            input
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match name {
            "read" => ("Read".into(), s("path")),
            "edit" => ("Changed".into(), s("path")),
            "run" => {
                let mut target = s("program");
                if let Some(args) = input.get("args").and_then(serde_json::Value::as_array) {
                    for a in args.iter().filter_map(serde_json::Value::as_str) {
                        target.push(' ');
                        target.push_str(a);
                    }
                }
                ("Ran".into(), target)
            }
            // The design's own verb for a lookup — and eight characters,
            // which is what fits the 9-cell `--detail-col` with its gap.
            "explain" => {
                let target = if !s("path").is_empty() {
                    s("path")
                } else {
                    s("query")
                };
                ("Searched".into(), target)
            }
            "plan" => ("Planned".into(), String::new()),
            "ask" => ("Asked".into(), s("question")),
            other => ("Used".into(), other.to_string()),
        }
    }

    /// The fact a finished call reports, from its result: a line count for
    /// a read, the exit for a run, otherwise the first line.
    pub fn fact_for(verb: &str, content: &str, failed: bool) -> String {
        match verb {
            "Read" if !failed => {
                let n = content.lines().count();
                format!("{n} {}", if n == 1 { "line" } else { "lines" })
            }
            "Ran" => {
                if failed {
                    first_line(content, 24)
                } else {
                    "ok".into()
                }
            }
            "Changed" if !failed => "staged".into(),
            _ => first_line(content, 24),
        }
    }
}

/// The collapsed summary of a `Work` entry: one clause per verb, counted,
/// joined with ` · `. `Read 3 files · Ran 2 programs`.
pub fn summarise_work(items: &[WorkItem]) -> String {
    let mut order: Vec<&str> = Vec::new();
    let mut counts: std::collections::HashMap<&str, (usize, usize)> =
        std::collections::HashMap::new();
    for item in items {
        let entry = counts.entry(item.verb.as_str()).or_insert_with(|| {
            order.push(item.verb.as_str());
            (0, 0)
        });
        entry.0 += 1;
        if item.failed {
            entry.1 += 1;
        }
    }
    order
        .into_iter()
        .map(|verb| {
            let (n, failed) = counts[verb];
            let noun = match verb {
                "Read" => plural(n, "file"),
                "Ran" => plural(n, "program"),
                "Searched" => plural(n, "symbol"),
                "Changed" => plural(n, "file"),
                "Asked" => plural(n, "question"),
                "Planned" => return "Planned".to_string(),
                _ => plural(n, "tool"),
            };
            if failed > 0 {
                format!("{verb} {noun}, {failed} failed")
            } else {
                format!("{verb} {noun}")
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn plural(n: usize, noun: &str) -> String {
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
/// N retries: …`. Anything else — an error from the loop itself — gets the
/// plain fallback. Open-tasks 32 is carrying the kind instead.
pub fn failure_sentence(error: &str) -> &'static str {
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
            .map(|(_, last)| failure_sentence(last))
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
            "The provider did not accept your API key. Check the key, then send again."
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
                "The provider did not accept your API key. Check the key, then send again.",
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
                "terminal error after 3 retries: provider error 529: overloaded",
                "The provider had a problem on its side. Send again in a moment.",
            ),
            (
                "terminal error after 3 retries: something else",
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
    use serde_json::json;

    #[test]
    fn calls_are_described_as_outcomes_not_tool_names() {
        assert_eq!(
            WorkItem::describe("read", &json!({"path": "src/x.rs"})),
            ("Read".into(), "src/x.rs".into())
        );
        assert_eq!(
            WorkItem::describe("run", &json!({"program": "cargo", "args": ["test", "-q"]})),
            ("Ran".into(), "cargo test -q".into())
        );
        assert_eq!(
            WorkItem::describe("explain", &json!({"query": "tower::limit"})),
            ("Searched".into(), "tower::limit".into())
        );
    }

    #[test]
    fn the_summary_counts_by_verb_in_first_seen_order() {
        let item = |verb: &str, failed: bool| WorkItem {
            call_id: "c".into(),
            verb: verb.into(),
            target: String::new(),
            fact: None,
            failed,
        };
        let items = vec![
            item("Read", false),
            item("Read", false),
            item("Ran", true),
            item("Read", false),
        ];
        assert_eq!(
            summarise_work(&items),
            "Read 3 files · Ran 1 program, 1 failed"
        );
        assert_eq!(summarise_work(&[item("Read", false)]), "Read 1 file");
    }

    #[test]
    fn facts_are_short_and_right() {
        assert_eq!(WorkItem::fact_for("Read", "a\nb\nc", false), "3 lines");
        assert_eq!(WorkItem::fact_for("Ran", "anything", false), "ok");
        assert_eq!(WorkItem::fact_for("Ran", "exit 1\nstderr", true), "exit 1");
        assert_eq!(
            first_line(&"x".repeat(40), 10),
            format!("{}…", "x".repeat(9))
        );
    }
}
