//! Retry policy for connection attempts.
//!
//! Mirrors the `RetryPolicy` / `ExponentialBackoffRetry` types in the other
//! five languages (Go/Python/Java/C#/C++) so defaults and tunables line up
//! across the polyglot clients.

use std::sync::Arc;
use std::time::Duration;

/// Callback fired once per failed retry attempt.
///
/// Receives `(attempt_zero_indexed, stringified_error)`. Matches Python's
/// `on_retry(attempt, exception)` hook semantically; the error is stringified
/// because Rust doesn't have a universal exception base type.
pub type OnRetry = Arc<dyn Fn(u32, &str) + Send + Sync>;

/// Exponential-backoff retry configuration.
///
/// Defaults match the cross-language spec: 10 attempts, 100 ms → 5 s with jitter.
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::{DomainClient, ExponentialBackoffRetry};
/// use std::time::Duration;
///
/// let policy = ExponentialBackoffRetry::default()
///     .with_max_attempts(5)
///     .with_max_delay(Duration::from_secs(2));
///
/// let client = DomainClient::connect_with_retry("http://localhost:1310", &policy).await?;
/// ```
#[derive(Clone)]
pub struct ExponentialBackoffRetry {
    /// Minimum delay between attempts.
    pub min_delay: Duration,
    /// Maximum delay between attempts (caps exponential growth).
    pub max_delay: Duration,
    /// Total number of attempts (including the first try).
    pub max_attempts: u32,
    /// Apply randomized jitter to each delay.
    pub jitter: bool,
    /// Optional hook called before each backoff sleep (not before the first
    /// attempt, not after the last failure).
    pub on_retry: Option<OnRetry>,
}

impl std::fmt::Debug for ExponentialBackoffRetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExponentialBackoffRetry")
            .field("min_delay", &self.min_delay)
            .field("max_delay", &self.max_delay)
            .field("max_attempts", &self.max_attempts)
            .field("jitter", &self.jitter)
            .field("has_on_retry", &self.on_retry.is_some())
            .finish()
    }
}

impl Default for ExponentialBackoffRetry {
    fn default() -> Self {
        Self {
            min_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            max_attempts: 10,
            jitter: true,
            on_retry: None,
        }
    }
}

impl ExponentialBackoffRetry {
    /// Alias for `Default::default`, named for parity with the other languages.
    pub fn exponential_backoff() -> Self {
        Self::default()
    }

    pub fn with_min_delay(mut self, d: Duration) -> Self {
        self.min_delay = d;
        self
    }

    pub fn with_max_delay(mut self, d: Duration) -> Self {
        self.max_delay = d;
        self
    }

    pub fn with_max_attempts(mut self, n: u32) -> Self {
        self.max_attempts = n;
        self
    }

    pub fn with_jitter(mut self, j: bool) -> Self {
        self.jitter = j;
        self
    }

    /// Install an `on_retry` callback fired before each backoff sleep.
    pub fn with_on_retry<F>(mut self, cb: F) -> Self
    where
        F: Fn(u32, &str) + Send + Sync + 'static,
    {
        self.on_retry = Some(Arc::new(cb));
        self
    }

    /// Compute the delay for the given zero-indexed attempt number.
    ///
    /// Matches Python's `_compute_delay`: `min_delay * 2^attempt`, capped at
    /// `max_delay`, optionally multiplied by `0.5 + rand()*0.5` when
    /// `jitter == true`.
    pub fn compute_delay(&self, attempt: u32) -> Duration {
        // min_delay * 2^attempt
        let raw_nanos = self.min_delay.as_nanos() * (1u128 << attempt.min(30));
        let cap_nanos = self.max_delay.as_nanos();
        let capped = raw_nanos.min(cap_nanos);
        let mut result = capped;
        if self.jitter {
            // Pseudo-random factor in [0.5, 1.0) without pulling in a `rand` dep:
            // hash the system clock to get a u32, map to [0, 1), rescale.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let frac = (now % 1_000_000) as f64 / 1_000_000.0; // [0, 1)
            let scale = 0.5 + frac * 0.5;
            result = (capped as f64 * scale) as u128;
        }
        Duration::from_nanos(result.min(u64::MAX as u128) as u64)
    }

    /// Run `op` up to `max_attempts` times, sleeping with exponential backoff
    /// between attempts. Returns the first `Ok`; if every attempt fails,
    /// returns the last error.
    ///
    /// The operation closure is synchronous; `std::thread::sleep` runs between
    /// attempts. For async retries, use an async-aware wrapper at the call site.
    pub fn execute<F, T, E>(&self, mut op: F) -> Result<T, E>
    where
        F: FnMut() -> Result<T, E>,
        E: std::fmt::Display,
    {
        debug_assert!(self.max_attempts > 0);
        let mut last_err: Option<E> = None;
        for attempt in 0..self.max_attempts {
            match op() {
                Ok(value) => return Ok(value),
                Err(e) => {
                    let is_last = attempt + 1 >= self.max_attempts;
                    if !is_last {
                        if let Some(cb) = &self.on_retry {
                            cb(attempt, &e.to_string());
                        }
                        std::thread::sleep(self.compute_delay(attempt));
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.expect("max_attempts >= 1 implies last_err is Some"))
    }
}

/// Backward-compatible name for the exponential-backoff config used throughout
/// the crate. Some older call sites (e.g. `DomainClient::connect_with_retry`)
/// still use this name, which is now an alias.
pub type RetryPolicy = ExponentialBackoffRetry;

/// The default retry policy (alias for `ExponentialBackoffRetry::default`).
pub fn default_retry_policy() -> ExponentialBackoffRetry {
    ExponentialBackoffRetry::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn default_matches_cross_language_spec() {
        let p = ExponentialBackoffRetry::default();
        assert_eq!(p.min_delay, Duration::from_millis(100));
        assert_eq!(p.max_delay, Duration::from_secs(5));
        assert_eq!(p.max_attempts, 10);
        assert!(p.jitter);
        assert!(p.on_retry.is_none());
    }

    #[test]
    fn execute_returns_first_ok() {
        let counter = AtomicU32::new(0);
        let policy = ExponentialBackoffRetry::default()
            .with_max_attempts(5)
            .with_min_delay(Duration::from_nanos(1))
            .with_jitter(false);
        let result: Result<u32, &'static str> = policy.execute(|| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err("not yet")
            } else {
                Ok(n)
            }
        });
        assert_eq!(result, Ok(2));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn execute_returns_last_err_when_all_fail() {
        let counter = AtomicU32::new(0);
        let policy = ExponentialBackoffRetry::default()
            .with_max_attempts(3)
            .with_min_delay(Duration::from_nanos(1))
            .with_jitter(false);
        let result: Result<u32, String> = policy.execute(|| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            Err(format!("fail-{n}"))
        });
        assert_eq!(result, Err("fail-2".to_string()));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn execute_stops_after_first_success() {
        let counter = AtomicU32::new(0);
        let policy = ExponentialBackoffRetry::default()
            .with_max_attempts(5)
            .with_min_delay(Duration::from_nanos(1))
            .with_jitter(false);
        let result: Result<u32, &'static str> = policy.execute(|| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(42)
        });
        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn on_retry_fires_between_attempts_only() {
        let fired = Arc::new(AtomicU32::new(0));
        let f = fired.clone();
        let policy = ExponentialBackoffRetry::default()
            .with_max_attempts(3)
            .with_min_delay(Duration::from_nanos(1))
            .with_jitter(false)
            .with_on_retry(move |_attempt, _msg| {
                f.fetch_add(1, Ordering::SeqCst);
            });
        let _: Result<u32, &'static str> = policy.execute(|| Err("nope"));
        // max_attempts=3: callback fires after attempts 0 and 1, not after 2 (the last).
        assert_eq!(fired.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn compute_delay_caps_at_max_delay() {
        let policy = ExponentialBackoffRetry::default()
            .with_min_delay(Duration::from_millis(100))
            .with_max_delay(Duration::from_secs(1))
            .with_jitter(false);
        // 100ms * 2^20 would be way more than 1s; must cap at max_delay.
        assert_eq!(policy.compute_delay(20), Duration::from_secs(1));
    }
}

