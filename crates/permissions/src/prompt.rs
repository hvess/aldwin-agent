//! Wire shapes for the prompt round trip. `mjolnir-core`'s
//! `Event::PromptRequested` / `Command::PromptResponse` carry an opaque
//! `serde_json::Value` — these types are what that value actually is.

use std::path::PathBuf;

use mjolnir_config::Class;
use serde::{Deserialize, Serialize};

/// Persistence tier for a context-file two-tier prompt response. No "once"
/// (injection is system-prompt-level, so "once" has no meaningful boundary)
/// and no "always" (context-file paths are intrinsically project-scoped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextFileTier {
    Session,
    Project,
}

/// One row of ADR 0004 §8's eight-row prompt. Four allow tiers and four deny
/// tiers, mirrored, drawn as a single vertical list.
///
/// Rows 2–4 and 6–7 are class-qualified — they are about this program's
/// *reads* or *writes*, so a `git: read` grant survives a `git: write` deny.
/// [`Self::NeverAllow`] is the exception and is deliberately blunt: the whole
/// program, every class, in the global file, as the lock of §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    AllowOnce,
    AllowSession,
    AllowProject,
    AllowEverywhere,
    DenyOnce,
    DenySession,
    DenyProject,
    NeverAllow,
}

impl Choice {
    /// In the order the rows are drawn.
    pub const ORDER: [Choice; 8] = [
        Choice::AllowOnce,
        Choice::AllowSession,
        Choice::AllowProject,
        Choice::AllowEverywhere,
        Choice::DenyOnce,
        Choice::DenySession,
        Choice::DenyProject,
        Choice::NeverAllow,
    ];

    pub fn is_allow(self) -> bool {
        matches!(
            self,
            Choice::AllowOnce | Choice::AllowSession | Choice::AllowProject | Choice::AllowEverywhere
        )
    }

    /// The rule this row quotes inside its sentence — the part drawn one
    /// step quieter, because it is the thing that would actually be written.
    /// `None` for the two rows that write nothing.
    pub fn rule(self, program: &str, class: Class) -> Option<String> {
        match self {
            Choice::AllowOnce | Choice::DenyOnce => None,
            Choice::NeverAllow => Some(program.to_string()),
            // A built-in whose name is already the thing it does would
            // otherwise read `Allow read reads for this session`. The rule
            // written is unchanged — `read: read` either way — but the
            // sentence says it once.
            _ if program == class.label() => Some(program.to_string()),
            _ => Some(format!("{program} {}s", class.label())),
        }
    }

    /// The sentence this row shows, stating the rule it would write
    /// (ADR 0003 §1, which ADR 0004 leaves standing).
    pub fn sentence(self, program: &str, class: Class) -> String {
        let rule = self.rule(program, class).unwrap_or_default();
        match self {
            Choice::AllowOnce => "Allow once".into(),
            Choice::AllowSession => format!("Allow {rule} for this session"),
            Choice::AllowProject => format!("Always allow {rule} in this project"),
            Choice::AllowEverywhere => format!("Always allow {rule} everywhere"),
            Choice::DenyOnce => "Deny once".into(),
            Choice::DenySession => format!("Deny {rule} for this session"),
            Choice::DenyProject => format!("Deny {rule} in this project"),
            Choice::NeverAllow => format!("Never allow {rule}"),
        }
    }
}

/// What the developer is being asked to decide.
///
/// [`Self::WriteAttempt`] is ADR 0004 §4's second prompt: a call the agent
/// declared a read was executed where writing was impossible, and it tried to
/// write anyway. Nothing landed — the sandbox is why — so answering yes
/// re-runs it with the tree writable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum PromptPayload {
    Tool { program: String, argv: Vec<String>, declared: Class },
    WriteAttempt { program: String, argv: Vec<String> },
    ContextFile { path: PathBuf },
    Edit { kind: String },
}

/// The developer's answer. `tier` on `ContextFile` is only meaningful when
/// `approve` is `true` — declining persists nothing at any tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum PromptResponse {
    Tool { choice: Choice },
    WriteAttempt { choice: Choice },
    ContextFile {
        approve: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tier: Option<ContextFileTier>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_payload_round_trips_through_json_value() {
        let payload = PromptPayload::Tool {
            program:  "git".into(),
            argv:     vec!["status".into()],
            declared: Class::Read,
        };
        let value = serde_json::to_value(&payload).unwrap();
        assert_eq!(value["shape"], "tool");
        assert_eq!(serde_json::from_value::<PromptPayload>(value).unwrap(), payload);
    }

    /// `read` is the one built-in whose name is also its class, and the
    /// sentence must not stutter.
    #[test]
    fn a_program_named_after_its_class_is_said_once() {
        assert_eq!(
            Choice::AllowSession.sentence("read", Class::Read),
            "Allow read for this session"
        );
        assert_eq!(
            Choice::AllowSession.sentence("git", Class::Read),
            "Allow git reads for this session"
        );
    }

    #[test]
    fn every_row_states_the_rule_it_would_write() {
        let sentences: Vec<String> =
            Choice::ORDER.iter().map(|c| c.sentence("git", Class::Write)).collect();
        // The four persisting allow rows and the two persisting deny rows each
        // name the program and the class; the two "once" rows deliberately do
        // not, because they write nothing.
        for (choice, sentence) in Choice::ORDER.iter().zip(&sentences) {
            match choice {
                Choice::AllowOnce | Choice::DenyOnce => assert!(!sentence.contains("git")),
                Choice::NeverAllow => assert!(sentence.contains("git") && !sentence.contains("write")),
                _ if false => unreachable!(),
                _ => assert!(sentence.contains("git") && sentence.contains("write"), "{sentence}"),
            }
        }
        assert_eq!(sentences.len(), 8);
    }
}
