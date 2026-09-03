use mjolnir_core::{RetryInfo, StepId, TurnEndReason};
use mjolnir_permissions::PromptPayload;

/// One entry in the conversation log. Append-only per mjolnir-tui.md — the
/// one exception is the transient thinking indicator, which isn't a log
/// entry at all (see `App::thinking`), since ThinkingEnd removes it rather
/// than leaving a record.
#[derive(Debug, Clone, PartialEq)]
pub enum LogEntry {
    UserMessage { text: String },
    AssistantText { text: String },
    /// Grouped per step, per mjolnir-tui.md's Pitfalls ("tool-activity
    /// entries flooding the log during parallel runs — group by step").
    ToolActivity { step_id: StepId, calls: Vec<ToolActivityEntry> },
    RetryAttempt { info: RetryInfo },
    /// Edit's binary approval gate — `ToolApprovalRequested`/`ApproveTool`/
    /// `DenyTool`. Resolved in place once answered (see `resolution`).
    ApprovalCard { call_id: String, diff: String, resolution: Option<bool> },
    /// The permission engine's four-tier / two-tier prompt —
    /// `PromptRequested`/`PromptResponse`. Resolved in place once answered,
    /// matched by `call_id` (same identifier `ApprovalCard` uses).
    PermissionPrompt { call_id: String, payload: PromptPayload, resolution: Option<PromptResolution> },
    TurnEnded { reason: TurnEndReasonKind },
    Error { message: String },
    /// From `Event::Notice` — a message from outside the turn/step
    /// lifecycle (mjolnir-cli rejecting a slash command, a
    /// `/reload-config` result). Rendered dim, not red like `Error` — it
    /// isn't necessarily bad news (a successful reload is a Notice too).
    Notice { message: String },
}

/// How a `PermissionPrompt` was answered, in the two forms the log needs:
/// `allowed` picks the row's glyph colour, `label` is the phrase shown
/// flush-right on it.
///
/// The label is written for a developer reading back over the session
/// ("allowed for this project"), not derived from the wire type. It used to
/// be `format!("{response:?}")`, so a resolved prompt printed a line of
/// Rust — `Tool { decision: Allow, tier: Once, pattern: "touch
/// /Users/…/hello.html" }` — into the middle of the conversation. Built by
/// `App::describe_response`, which is the one place the mapping lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptResolution {
    pub allowed: bool,
    pub label:   String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolActivityEntry {
    pub call_id: String,
    pub name:    String,
    pub status:  ToolActivityStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolActivityStatus {
    Running,
    Completed { is_error: bool, summary: String },
}

/// Own copy of core's `TurnEndReason` shape, since core's doesn't derive
/// `PartialEq`/`Clone` in a way this crate wants to lean on for tests and
/// `matches!` — see the `From` impl below for the one place they meet.
#[derive(Debug, Clone, PartialEq)]
pub enum TurnEndReasonKind {
    EndTurn,
    Cancelled,
    Error(String),
}

impl From<TurnEndReason> for TurnEndReasonKind {
    fn from(reason: TurnEndReason) -> Self {
        match reason {
            TurnEndReason::EndTurn => Self::EndTurn,
            TurnEndReason::Cancelled => Self::Cancelled,
            TurnEndReason::Error(message) => Self::Error(message),
        }
    }
}

/// Rows the welcome hero (`ui::intro_content`) always renders: the tagline,
/// a blank row, then `model`/`version`/`commit`/`access` fact rows — 6
/// total. No art any more — the Mjolnir Design System's own "no logo"
/// rule replaced the traced hammer/wordmark with this plain fact block
/// (see `ui::intro_content`'s doc comment). Fixed shape, not derived from
/// any state but the model name (itself always one line), so this is a
/// hand-kept constant rather than a function.
///
/// Not used to compute "the screen row the first real log entry starts
/// on" — the hero and real log entries are mutually exclusive (see
/// `ui::build_log_lines`), so there's no such offset to compute. Its only
/// remaining job is pinning down the hero's exact content-row count in
/// `intro_banner_shows_the_active_model_and_is_exactly_intro_line_count_rows`,
/// hence `#[cfg(test)]`. Update alongside `ui::intro_content` if its shape
/// ever changes.
#[cfg(test)]
pub const INTRO_LINE_COUNT: usize = 6;

/// Truncates a tool result to a one-line summary for a closed
/// `ToolActivityEntry` — the full content already went into the model's
/// context; the log just needs enough to glance at.
pub fn summarise(content: &str, max_len: usize) -> String {
    let first_line = content.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= max_len {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(max_len).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarise_short_content_is_unchanged() {
        assert_eq!(summarise("ok", 50), "ok");
    }

    #[test]
    fn summarise_takes_only_the_first_line() {
        assert_eq!(summarise("line one\nline two", 50), "line one");
    }

    #[test]
    fn summarise_truncates_long_lines_with_an_ellipsis() {
        let out = summarise(&"x".repeat(100), 10);
        assert_eq!(out, format!("{}…", "x".repeat(10)));
    }

    #[test]
    fn turn_end_reason_converts_from_core() {
        assert_eq!(TurnEndReasonKind::from(TurnEndReason::EndTurn), TurnEndReasonKind::EndTurn);
        assert_eq!(TurnEndReasonKind::from(TurnEndReason::Cancelled), TurnEndReasonKind::Cancelled);
        assert_eq!(TurnEndReasonKind::from(TurnEndReason::Error("x".into())), TurnEndReasonKind::Error("x".into()));
    }

}
