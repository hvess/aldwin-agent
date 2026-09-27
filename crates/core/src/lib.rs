//! The agent loop and its boundary types; spec `.claude/spec/archive/aldwin-core.md`.
//!
//! `LlmClient` and `ToolDispatcher` are implemented in aldwin-llm and
//! aldwin-tools, so no provider wire type or filesystem dependency reaches
//! this crate.

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
