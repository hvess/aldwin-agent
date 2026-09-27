use crate::{
    event::LlmEvent,
    types::{Message, ToolDefinition},
};
use futures::Stream;
use std::pin::Pin;
use thiserror::Error;

/// Why a step's stream failed, in terms no provider owns.
#[derive(Debug, Error)]
pub enum LlmError {
    /// The request never got an HTTP answer, on the first attempt.
    #[error("network error: {0}")]
    Network(String),
    /// The provider answered with an error status, on the first attempt.
    #[error("provider error {status}: {message}")]
    Provider {
        /// The HTTP status the provider answered with.
        status: u16,
        /// The provider's own description of the error.
        message: String,
    },
    /// The stream broke after events had already been emitted, so it could
    /// not be retried without splicing two completions together.
    #[error("stream interrupted: {0}")]
    StreamInterrupted(String),
    /// Retries were exhausted, or the request could not be sent at all; the
    /// attempt count says more than the last attempt's specific cause.
    #[error("terminal error after {attempts} attempts: {message}")]
    Terminal {
        /// How many attempts were made; zero when none could be.
        attempts: u32,
        /// What went wrong on the last attempt.
        message: String,
    },
}

/// Everything one step asks of the provider, borrowed from the agent.
#[derive(Debug)]
pub struct LlmRequest<'a> {
    /// The system prompt.
    pub system: &'a str,
    /// The tools the model may call this step.
    pub tools: &'a [ToolDefinition],
    /// The conversation so far, rebuilt from the log.
    pub messages: &'a [Message],
    /// The index into `messages` after which the provider should insert a
    /// cache breakpoint — the last message, so the whole conversation so far
    /// is the cached prefix. `None` for an empty conversation.
    pub cache_breakpoint: Option<usize>,
}

/// A provider the agent can stream a step from. Implemented in aldwin-llm;
/// this trait is the boundary no Anthropic wire type crosses.
pub trait LlmClient: Send + Sync {
    /// Sends `request` and yields the step as provider-agnostic events. The
    /// agent reads until `LlmEvent::StepEnded` or the first `Err`; a stream
    /// that closes before either is treated as an error.
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>>;
}
