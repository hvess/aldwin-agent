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
    PermissionPrompt { call_id: String, payload: PromptPayload, resolution: Option<String> },
    TurnEnded { reason: TurnEndReasonKind },
    Error { message: String },
    /// From `Event::Notice` — a message from outside the turn/step
    /// lifecycle (mjolnir-cli rejecting a slash command, a
    /// `/reload-config` result). Rendered dim, not red like `Error` — it
    /// isn't necessarily bad news (a successful reload is a Notice too).
    Notice { message: String },
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

/// Rows the welcome banner (`ui::intro_lines`) always renders: a blank
/// padding row, `MJOLNIR_ART`'s 13 traced Braille rows (a 2026-08-31
/// downscale of the original 21-row trace, still proportionate — see
/// `ui::MJOLNIR_ART`'s doc comment) (the `WORDMARK_ART` block plus the
/// tagline/stats/access-permissions lines render beside the hammer art on
/// those same rows, as one combined right-hand column — not as extra rows
/// above, below, or around it — see `ui::intro_lines`), then another blank
/// padding row — 15 content rows — plus the top/bottom border
/// `ui::bordered` always adds regardless of render width. Fixed shape, not
/// derived from any state but the model name (itself always one line) and
/// the render width (which changes the border's length, not its row
/// count), so this is a hand-kept constant rather than a function.
///
/// No longer part of live scroll math (`App::total_lines` gets exact
/// wrapped-row counts from `ui::log_row_count` instead — see its doc
/// comment) — this constant's only remaining job is letting `ui.rs`'s own
/// tests compute "the screen row the first real log entry starts on"
/// without duplicating the banner's shape, hence `#[cfg(test)]`. Update
/// alongside `ui::intro_lines`/`ui::WORDMARK_ART`/`ui::MJOLNIR_ART` if its
/// shape ever changes.
#[cfg(test)]
pub const INTRO_LINE_COUNT: usize = 17;

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
