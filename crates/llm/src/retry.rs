use std::time::Duration;

use aldwin_core::LlmError;

/// What one read off the SSE stream produced this iteration — shared by both
/// clients' attempt loops since neither variant is wire-type-specific
/// (`LlmEvent` is core's normalised type, `String` is a plain error message).
pub enum AttemptOutcome {
    Events(Vec<aldwin_core::LlmEvent>),
    Failed(String),
}

/// Chooses the terminal error variant for a failure we're not retrying.
/// `attempt == 1` means nothing was ever retried — surface the specific
/// cause (`Provider`/`Network`). `attempt > 1` means retries were exhausted
/// — surface `Terminal`, core's "gave up after N tries" bucket, since by
/// that point the specific final-attempt cause is less useful than the
/// retry count. Shared by both `AnthropicClient` and `OpenAiCompatibleClient`
/// — the rule is provider-agnostic.
pub fn terminal_error(attempt: u32, status: Option<u16>, message: String) -> LlmError {
    if attempt == 1 {
        match status {
            Some(status) => LlmError::Provider { status, message },
            None => LlmError::Network(message),
        }
    } else {
        LlmError::Terminal { attempts: attempt, message }
    }
}

/// Max total attempts (the first try plus up to three retries).
pub const MAX_ATTEMPTS: u32 = 4;
const BASE: Duration = Duration::from_secs(1);
const CAP: Duration = Duration::from_secs(30);

/// 60s SSE silence drops the stream and engages the retry path.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504 | 529)
}

/// Full-jitter exponential backoff: `random(0, min(cap, base * 2^(attempt-1)))`.
/// `attempt` is the attempt number that just failed (1-indexed).
pub fn backoff(attempt: u32) -> Duration {
    let exp = BASE.saturating_mul(1u32 << attempt.saturating_sub(1).min(30));
    let ceiling = exp.min(CAP);
    Duration::from_secs_f64(rand::random::<f64>() * ceiling.as_secs_f64())
}

/// Whether there's another attempt left after this one failed.
pub fn should_retry(attempt: u32) -> bool {
    attempt < MAX_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_status_set_matches_the_spec() {
        for s in [408, 429, 500, 502, 503, 504, 529] {
            assert!(is_retryable_status(s), "{s} should be retryable");
        }
        for s in [400, 401, 403, 404, 413, 422] {
            assert!(!is_retryable_status(s), "{s} should not be retryable");
        }
    }

    #[test]
    fn backoff_never_exceeds_the_cap() {
        for attempt in 1..10 {
            assert!(backoff(attempt) <= CAP);
        }
    }

    #[test]
    fn backoff_grows_with_attempt_number_on_average() {
        // Full jitter means any single sample can be near zero, but the
        // ceiling should still climb — sample many draws and compare maxima.
        let max_at = |attempt: u32| (0..200).map(|_| backoff(attempt)).max().unwrap();
        assert!(max_at(1) < max_at(3));
    }

    #[test]
    fn should_retry_allows_exactly_max_attempts_total() {
        assert!(should_retry(1));
        assert!(should_retry(2));
        assert!(should_retry(3));
        assert!(!should_retry(4));
    }
}
