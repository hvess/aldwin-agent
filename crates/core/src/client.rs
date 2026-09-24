use crate::{
    event::LlmEvent,
    types::{Message, ToolDefinition},
};
use futures::Stream;
use std::pin::Pin;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("network error: {0}")]
    Network(String),
    #[error("provider error {status}: {message}")]
    Provider { status: u16, message: String },
    #[error("stream interrupted: {0}")]
    StreamInterrupted(String),
    #[error("terminal error after {attempts} attempts: {message}")]
    Terminal { attempts: u32, message: String },
}

pub struct LlmRequest<'a> {
    pub system: &'a str,
    pub tools: &'a [ToolDefinition],
    pub messages: &'a [Message],
    /// The index into `messages` after which the provider should insert a
    /// cache breakpoint — the last message, so the whole conversation so far
    /// is the cached prefix. `None` for an empty conversation.
    pub cache_breakpoint: Option<usize>,
}

pub trait LlmClient: Send + Sync {
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>>;
}
