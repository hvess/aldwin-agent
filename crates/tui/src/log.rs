use mjolnir_core::{PromptId, RetryInfo, StepId, TurnEndReason};
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
    /// `PromptRequested`/`PromptResponse`. Resolved in place once answered.
    PermissionPrompt { id: PromptId, payload: PromptPayload, resolution: Option<String> },
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

/// Rows the welcome banner (`ui::intro_lines`) always renders: `MJOLNIR_ART`'s
/// 16 traced Braille rows (the wordmark/tagline/version info renders beside
/// the art on those same rows, not as extra rows below it — see
/// `ui::intro_lines`) plus the top/bottom border `ui::bordered` always adds
/// regardless of render width. Fixed shape, not derived from any state but
/// the model name (itself always one line) and the render width (which
/// changes the border's length, not its row count), so this is a hand-kept
/// constant rather than a function — same contract as `line_count` below:
/// exact, not approximate, since `App::total_lines` needs precise scroll
/// math without this crate's `app` module depending on ratatui at all.
/// Update alongside `ui::intro_lines`/`ui::MJOLNIR_ART` if its shape ever
/// changes.
pub const INTRO_LINE_COUNT: usize = 18;

/// The number of terminal rows `ui::render_entry` will produce for this
/// entry — kept here (not in `ui.rs`) so `ScrollState`'s bookkeeping
/// (`App::total_lines`, `app.rs`) can stay accurate without `app.rs`
/// depending on ratatui at all.
///
/// Exact for every variant, with one narrow, transient exception:
/// `AssistantText` holding a still-open (unterminated) code fence — the
/// closing fence hasn't streamed in yet — renders one extra border line
/// (`ui::render_assistant_text` always emits the closing "└─" even without
/// a matching "```" in the source) that isn't in `text.lines().count()`
/// yet. Off by at most 1, only mid-stream, and self-corrects the moment
/// the fence closes — not worth threading fence-parsing state in here to
/// avoid.
///
/// For every other case this is provably exact, not approximate: a
/// fenced code block's `┌─ lang` / `└─` border lines exactly replace the
/// opening/closing "```" lines they're rendered instead of (same count),
/// so `AssistantText`'s total is `text.lines().count()` whether or not it
/// contains code fences.
pub fn line_count(entry: &LogEntry) -> usize {
    match entry {
        LogEntry::UserMessage { text } | LogEntry::AssistantText { text } => text.lines().count(),
        LogEntry::ToolActivity { calls, .. } => calls.len(),
        LogEntry::RetryAttempt { .. } => 1,
        // render_card: one header line + one line per body ("" for
        // PermissionPrompt, so exactly 2) + one footer line.
        LogEntry::ApprovalCard { diff, .. } => diff.lines().count() + 2,
        LogEntry::PermissionPrompt { .. } => 2,
        LogEntry::TurnEnded { .. } => 1,
        LogEntry::Error { .. } => 1,
        LogEntry::Notice { .. } => 1,
    }
}

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

    #[test]
    fn line_count_counts_text_lines_for_messages() {
        assert_eq!(line_count(&LogEntry::UserMessage { text: "a\nb\nc".into() }), 3);
        assert_eq!(line_count(&LogEntry::AssistantText { text: "one line".into() }), 1);
    }

    #[test]
    fn line_count_counts_one_line_per_tool_call() {
        let calls = vec![
            ToolActivityEntry { call_id: "1".into(), name: "read".into(), status: ToolActivityStatus::Running },
            ToolActivityEntry { call_id: "2".into(), name: "shell".into(), status: ToolActivityStatus::Running },
        ];
        assert_eq!(line_count(&LogEntry::ToolActivity { step_id: StepId(1), calls }), 2);
    }

    #[test]
    fn line_count_includes_the_cards_border_lines() {
        assert_eq!(line_count(&LogEntry::ApprovalCard { call_id: "c".into(), diff: "-a\n+b".into(), resolution: None }), 4);
        let payload = PromptPayload::Tool { kind: "shell".into(), target: "ls".into() };
        assert_eq!(line_count(&LogEntry::PermissionPrompt { id: PromptId(1), payload, resolution: None }), 2);
    }

    #[test]
    fn line_count_is_one_for_single_line_entries() {
        assert_eq!(line_count(&LogEntry::TurnEnded { reason: TurnEndReasonKind::EndTurn }), 1);
        assert_eq!(line_count(&LogEntry::Error { message: "boom".into() }), 1);
        assert_eq!(line_count(&LogEntry::Notice { message: "note".into() }), 1);
    }
}
