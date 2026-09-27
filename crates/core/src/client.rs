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
    /// No HTTP answer, on the first attempt.
    #[error("network error: {0}")]
    Network(String),
    /// The provider answered with an error status, on the first attempt.
    #[error("provider error {status}: {message}")]
    Provider {
        /// HTTP status.
        status: u16,
        /// The provider's error text.
        message: String,
    },
    /// The stream broke after events were emitted; not retried, since a retry
    /// would splice two completions together.
    #[error("stream interrupted: {0}")]
    StreamInterrupted(String),
    /// Retries were exhausted, or the request could not be sent at all.
    #[error("terminal error after {attempts} attempts: {message}")]
    Terminal {
        /// Attempts made; zero when none could be.
        attempts: u32,
        /// The last attempt's failure.
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
    /// Index into `messages` after which the provider inserts a cache
    /// breakpoint: the last message. `None` for an empty conversation.
    pub cache_breakpoint: Option<usize>,
}

/// A provider the agent streams a step from; implemented in aldwin-llm.
/// No Anthropic wire type crosses this trait.
pub trait LlmClient: Send + Sync {
    /// Sends `request` and yields the step as provider-agnostic events. The
    /// agent reads until `LlmEvent::StepEnded` or the first `Err`; a stream
    /// that closes before either is an error.
    fn stream<'a>(
        &'a self,
        request: LlmRequest<'a>,
    ) -> Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>>;
}
