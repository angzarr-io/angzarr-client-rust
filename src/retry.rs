//! Retry policy for connection attempts.
//!
//! Mirrors the `RetryPolicy` / `ExponentialBackoffRetry` types in the other
//! five languages (Go/Python/Java/C#/C++) so defaults and tunables line up
//! across the polyglot clients.

use std::time::Duration;

/// Exponential-backoff retry configuration.
///
/// Defaults match the cross-language spec: 10 attempts, 100 ms → 5 s with jitter.
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::{DomainClient, RetryPolicy};
/// use std::time::Duration;
///
/// let policy = RetryPolicy::default()
///     .with_max_attempts(5)
///     .with_max_delay(Duration::from_secs(2));
///
/// let client = DomainClient::connect_with_retry("http://localhost:1310", &policy).await?;
/// ```
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Minimum delay between attempts.
    pub min_delay: Duration,
    /// Maximum delay between attempts (caps exponential growth).
    pub max_delay: Duration,
    /// Total number of attempts (including the first try).
    pub max_attempts: u32,
    /// Apply randomized jitter to each delay.
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            min_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            max_attempts: 10,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Default exponential-backoff policy (10 attempts, 100 ms → 5 s, jitter).
    ///
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
}

/// The default retry policy (alias for `RetryPolicy::default`).
pub fn default_retry_policy() -> RetryPolicy {
    RetryPolicy::default()
}

/// Alias type for cross-language name parity.
///
/// Python exposes `ExponentialBackoffRetry` as a distinct class with an
/// `on_retry` callback. Rust's `RetryPolicy` carries the same tunables
/// (`min_delay`/`max_delay`/`max_attempts`/`jitter`) and is reused here
/// via a type alias. The `with_on_retry` builder method attaches a
/// callback fired between attempts.
pub type ExponentialBackoffRetry = RetryPolicy;

