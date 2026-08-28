use std::pin::Pin;
use futures::Stream;
use thiserror::Error;
use crate::{event::LlmEvent, types::{Message, ToolDefinition}};

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("network error: {0}")]
    Network(String),
    #[error("provider error {status}: {message}")]
    Provider { status: u16, message: String },
    #[error("stream interrupted: {0}")]
    StreamInterrupted(String),
    #[error("terminal error after {attempts} retries: {message}")]
    Terminal { attempts: u32, message: String },
}

pub struct LlmRequest<'a> {
    pub model:             &'a str,
    pub system:            &'a str,
    pub tools:             &'a [ToolDefinition],
    pub messages:          &'a [Message],
    /// Indices into `messages` after which the provider should insert a cache breakpoint.
    pub cache_breakpoints: &'a [usize],
}

pub trait LlmClient: Send + Sync {
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>>;
}
