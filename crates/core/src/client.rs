use crate::{
    event::{Failure, FailureKind, LlmEvent},
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
    /// Retries were exhausted, or a later attempt was refused.
    #[error("terminal error after {attempts} attempts: {message}")]
    Terminal {
        /// Attempts made.
        attempts: u32,
        /// The last attempt's error status, if it had one.
        status: Option<u16>,
        /// The last attempt's failure.
        message: String,
    },
    /// Nothing could be sent (no model, or ADR 0012's no account and no
    /// key); the text is already the sentence to show.
    #[error("{0}")]
    NotSent(String),
}

impl From<LlmError> for Failure {
    fn from(error: LlmError) -> Self {
        let kind = match error {
            LlmError::Network(_) => FailureKind::Network,
            LlmError::Provider { status, .. } => FailureKind::Provider { status },
            LlmError::StreamInterrupted(_) => FailureKind::Interrupted,
            LlmError::Terminal { status, .. } => FailureKind::Exhausted { status },
            LlmError::NotSent(_) => FailureKind::NotSent,
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind carries what the text only spelled; the text stays the
    /// error's, except where it is already the sentence.
    #[test]
    fn an_llm_error_becomes_a_failure_of_its_kind() {
        let cases = [
            (LlmError::Network("refused".into()), FailureKind::Network),
            (
                LlmError::Provider {
                    status: 429,
                    message: "slow down".into(),
                },
                FailureKind::Provider { status: 429 },
            ),
            (
                LlmError::StreamInterrupted("eof".into()),
                FailureKind::Interrupted,
            ),
            (
                LlmError::Terminal {
                    attempts: 4,
                    status: Some(529),
                    message: "overloaded".into(),
                },
                FailureKind::Exhausted { status: Some(529) },
            ),
        ];
        for (error, kind) in cases {
            let text = error.to_string();
            assert_eq!(
                Failure::from(error),
                Failure {
                    kind,
                    message: text
                }
            );
        }
        let said = "No model is configured yet. Pick one with /model.";
        assert_eq!(
            Failure::from(LlmError::NotSent(said.into())),
            Failure {
                kind: FailureKind::NotSent,
                message: said.into(),
            }
        );
    }
}
