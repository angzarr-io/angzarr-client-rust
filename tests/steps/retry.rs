//! Step defs for features/client/retry.feature.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cucumber::{given, then, when, World};

use angzarr_client::error_codes::{codes, keys};
use angzarr_client::{default_retry_policy, CommandRejectedError, ExponentialBackoffRetry};

#[derive(Default, World)]
#[world(init = Self::new)]
pub struct RetryWorld {
    policy: Option<ExponentialBackoffRetry>,
    op: Option<Arc<dyn Fn() -> Result<i64, String> + Send + Sync>>,
    call_count: Arc<AtomicU32>,
    on_retry_calls: Arc<AtomicU32>,
    result: Option<Result<i64, String>>,
    rejected: Option<CommandRejectedError>,
}

impl std::fmt::Debug for RetryWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryWorld")
            .field("has_policy", &self.policy.is_some())
            .field("call_count", &self.call_count.load(Ordering::SeqCst))
            .field(
                "on_retry_calls",
                &self.on_retry_calls.load(Ordering::SeqCst),
            )
            .field("has_result", &self.result.is_some())
            .field("has_rejected", &self.rejected.is_some())
            .finish()
    }
}

impl RetryWorld {
    fn new() -> Self {
        Self::default()
    }
}

#[when("I obtain the default retry policy")]
async fn when_default_policy(world: &mut RetryWorld) {
    world.policy = Some(default_retry_policy());
}

#[then(regex = r"^the policy has min_delay (\d+) ms$")]
async fn then_min_delay(world: &mut RetryWorld, ms: u64) {
    let p = world.policy.as_ref().expect("no policy");
    assert_eq!(p.min_delay, Duration::from_millis(ms));
}

#[then(regex = r"^the policy has max_delay (\d+) ms$")]
async fn then_max_delay(world: &mut RetryWorld, ms: u64) {
    let p = world.policy.as_ref().expect("no policy");
    assert_eq!(p.max_delay, Duration::from_millis(ms));
}

#[then(regex = r"^the policy has max_attempts (\d+)$")]
async fn then_max_attempts(world: &mut RetryWorld, n: u32) {
    let p = world.policy.as_ref().expect("no policy");
    assert_eq!(p.max_attempts, n);
}

#[then("the policy has jitter enabled")]
async fn then_jitter_enabled(world: &mut RetryWorld) {
    let p = world.policy.as_ref().expect("no policy");
    assert!(p.jitter);
}

#[given(regex = r"^an ExponentialBackoffRetry with max_attempts (\d+) and jitter disabled$")]
async fn given_policy(world: &mut RetryWorld, n: u32) {
    world.policy = Some(
        ExponentialBackoffRetry::default()
            .with_min_delay(Duration::from_nanos(1))
            .with_max_delay(Duration::from_nanos(1))
            .with_max_attempts(n)
            .with_jitter(false),
    );
}

#[given(regex = r"^an operation that fails (\d+) times then returns (-?\d+)$")]
async fn given_op_fails_then(world: &mut RetryWorld, fail_count: u32, value: i64) {
    let count = world.call_count.clone();
    world.op = Some(Arc::new(move || {
        let n = count.fetch_add(1, Ordering::SeqCst) + 1;
        if n <= fail_count {
            Err(format!("fail-{n}"))
        } else {
            Ok(value)
        }
    }));
}

#[given("an operation that always fails")]
async fn given_op_always_fails(world: &mut RetryWorld) {
    let count = world.call_count.clone();
    world.op = Some(Arc::new(move || {
        let n = count.fetch_add(1, Ordering::SeqCst) + 1;
        Err(format!("fail-{n}"))
    }));
}

#[given("an on_retry callback that counts invocations")]
async fn given_on_retry(world: &mut RetryWorld) {
    let counter = world.on_retry_calls.clone();
    let policy = world.policy.take().expect("no policy");
    world.policy = Some(policy.with_on_retry(move |_attempt, _msg| {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
}

#[when("I execute the operation through the retry policy")]
async fn when_execute(world: &mut RetryWorld) {
    let policy = world.policy.as_ref().expect("no policy");
    let op = world.op.as_ref().expect("no op").clone();
    let result = policy.execute(move || op());
    world.result = Some(result);
}

#[then(regex = r"^the returned value is (-?\d+)$")]
async fn then_returned_value(world: &mut RetryWorld, value: i64) {
    match world.result.as_ref().expect("no result") {
        Ok(v) => assert_eq!(*v, value),
        Err(e) => panic!("expected Ok({value}), got Err({e})"),
    }
}

#[then(regex = r"^the operation was called (\d+) times$")]
async fn then_call_count(world: &mut RetryWorld, n: u32) {
    assert_eq!(world.call_count.load(Ordering::SeqCst), n);
}

#[then("the result is an error")]
async fn then_result_is_error(world: &mut RetryWorld) {
    assert!(world.result.as_ref().expect("no result").is_err());
}

#[then(regex = r"^the on_retry callback was invoked (\d+) times$")]
async fn then_on_retry_count(world: &mut RetryWorld, n: u32) {
    assert_eq!(world.on_retry_calls.load(Ordering::SeqCst), n);
}

#[when(
    regex = r#"^I construct a CommandRejectedError via precondition_failed with reason "([^"]*)"$"#
)]
async fn when_precondition_failed(world: &mut RetryWorld, reason: String) {
    // Audit #59: static message + structured detail. The cucumber-supplied
    // `reason` rides as a detail value rather than being interpolated into
    // the message string.
    world.rejected = Some(CommandRejectedError::precondition_failed(
        codes::STATUS_MISMATCH,
        "precondition failed",
        [(keys::CONTEXT, reason)],
    ));
}

#[then("the error's is_precondition_failed predicate is true")]
async fn then_pf_true(world: &mut RetryWorld) {
    assert!(world
        .rejected
        .as_ref()
        .expect("no rejected")
        .is_precondition_failed());
}

#[then("the error's is_invalid_argument predicate is false")]
async fn then_ia_false(world: &mut RetryWorld) {
    assert!(!world
        .rejected
        .as_ref()
        .expect("no rejected")
        .is_invalid_argument());
}

#[then("the error's is_not_found predicate is false")]
async fn then_nf_false(world: &mut RetryWorld) {
    assert!(!world.rejected.as_ref().expect("no rejected").is_not_found());
}
