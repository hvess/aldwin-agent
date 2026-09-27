//! The agent loop and the boundary types around it. See
//! `.claude/spec/archive/aldwin-core.md`.
//!
//! Core drives a conversation: it takes `Command`s from the TUI, streams a
//! step from an `LlmClient`, hands tool calls to a `ToolDispatcher`, and
//! emits `Event`s back up. Both traits are implemented elsewhere
//! (aldwin-llm, aldwin-tools), so no provider wire type and no filesystem
//! dependency reaches this crate.

mod agent;
mod client;
mod dispatcher;
mod event;
mod log;
mod prompt;
mod types;

pub use agent::Agent;
pub use client::{LlmClient, LlmError, LlmRequest};
pub use dispatcher::{DispatchContext, PendingMap, PendingReply, ToolDispatcher};
pub use event::{Command, Event, LlmEvent, LogRecord, StepOutcome, TurnEndReason};
pub use log::{ConversationLog, RecordSink};
pub use types::*;
