use std::time::Duration;

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
