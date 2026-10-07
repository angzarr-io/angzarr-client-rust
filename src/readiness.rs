//! Readiness probes and health-status supervisor for runner servers.
//!
//! A runner exposes its readiness through `grpc.health.v1.Health`. While any
//! probe is failing, the per-kind service name reports `NOT_SERVING`; once
//! every probe is green, it flips to `SERVING`. Probes are evaluated on a
//! fixed cadence (default 30s, override via `ANGZARR_READINESS_PROBE_INTERVAL`)
//! with a per-probe timeout (default 2s, override via
//! `ANGZARR_READINESS_PROBE_TIMEOUT`).
//!
//! Aggregation is binary — `all up` is `SERVING`, anything else is `NOT_SERVING`.
//! The health server itself always responds, so liveness ("the process answers")
//! and readiness ("it's safe to send traffic") share one wire surface and are
//! distinguished by the response status.

use std::env;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{FuturesUnordered, StreamExt};
use futures::FutureExt;
use tonic_health::server::HealthReporter;
use tonic_health::ServingStatus;
use tracing::warn;

use crate::error::Result;

/// Parse a bare endpoint string (e.g. `host:port`, `/abs/path`,
/// `unix:/abs/path`, `unix:///abs/path`, `unix:relative/path`) into the
/// `Endpoint` enum used by both [`OutputDomainProbe`] and [`BusProbe`].
///
/// Centralizes the `unix:` prefix handling so both probes recognize
/// every form the client-side `detect_uds_path` does — they used to
/// diverge.
fn parse_probe_endpoint(raw: String) -> Endpoint {
    if let Some(rest) = raw.strip_prefix("unix://") {
        Endpoint::Uds(PathBuf::from(rest))
    } else if let Some(rest) = raw.strip_prefix("unix:") {
        Endpoint::Uds(PathBuf::from(rest))
    } else if raw.starts_with('/') || raw.starts_with("./") {
        Endpoint::Uds(PathBuf::from(raw))
    } else {
        Endpoint::Tcp(raw)
    }
}

/// Default cadence for re-evaluating output-domain probes.
pub const DEFAULT_PROBE_INTERVAL: Duration = Duration::from_secs(30);
/// Default per-probe timeout.
pub const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

const ENV_INTERVAL: &str = "ANGZARR_READINESS_PROBE_INTERVAL";
const ENV_TIMEOUT: &str = "ANGZARR_READINESS_PROBE_TIMEOUT";

/// Audit #74: optional async-bus endpoint (Kafka / RabbitMQ / SQS /
/// SNS / NATS / etc.). When set, a single [`BusProbe`] covers
/// reachability of the async path for every async-only saga / PM
/// target. When unset, no bus probe is added — async-only targets
/// are simply not part of readiness.
pub const ENV_BUS_ENDPOINT: &str = "ANGZARR_BUS_ENDPOINT";

/// Read the supervisor cadence + per-probe timeout from env, falling back to
/// the [`DEFAULT_PROBE_INTERVAL`] / [`DEFAULT_PROBE_TIMEOUT`] constants.
pub fn probe_config_from_env() -> (Duration, Duration) {
    let interval = env::var(ENV_INTERVAL)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_PROBE_INTERVAL);
    let timeout = env::var(ENV_TIMEOUT)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_PROBE_TIMEOUT);
    (interval, timeout)
}

/// Single readiness probe — evaluated once per supervisor tick.
#[async_trait]
pub trait Probe: Send + Sync + 'static {
    /// Stable identifier for log lines and (future) per-probe service names.
    fn name(&self) -> &str;
    /// `true` when the underlying dependency is currently healthy.
    async fn check(&self) -> bool;
}

/// One-shot transport probe — flipped `true` once the listener has bound and
/// the server is accepting traffic. From that point its result never changes.
/// Marking it bound also wakes the supervisor (see [`TransportProbe::wake`])
/// so readiness flips without waiting out the probe interval.
pub struct TransportProbe {
    state: Arc<TransportState>,
}

struct TransportState {
    bound: AtomicBool,
    wake: Arc<tokio::sync::Notify>,
}

impl TransportProbe {
    pub fn new() -> (Self, TransportSignal) {
        let state = Arc::new(TransportState {
            bound: AtomicBool::new(false),
            wake: Arc::new(tokio::sync::Notify::new()),
        });
        (
            Self {
                state: state.clone(),
            },
            TransportSignal { state },
        )
    }

    /// Notified when the transport is marked bound; pass it to
    /// [`run_supervisor_with_wake`].
    pub fn wake(&self) -> Arc<tokio::sync::Notify> {
        self.state.wake.clone()
    }
}

impl Default for TransportProbe {
    fn default() -> Self {
        Self::new().0
    }
}

#[async_trait]
impl Probe for TransportProbe {
    fn name(&self) -> &str {
        "transport"
    }
    async fn check(&self) -> bool {
        self.state.bound.load(Ordering::SeqCst)
    }
}

/// Side of the [`TransportProbe`] used by the runner to mark "bound and serving".
pub struct TransportSignal {
    state: Arc<TransportState>,
}

impl TransportSignal {
    /// Mark the transport as accepting traffic and wake the supervisor.
    pub fn mark_bound(&self) {
        self.state.bound.store(true, Ordering::SeqCst);
        self.state.wake.notify_one();
    }
}

/// Per-output-domain coordinator probe — attempts to open a connection to the
/// downstream domain's command handler coordinator endpoint.
pub struct OutputDomainProbe {
    domain: String,
    endpoint: Endpoint,
}

#[derive(Debug, Clone)]
enum Endpoint {
    /// `host:port` for TCP.
    Tcp(String),
    /// Filesystem path for UDS.
    Uds(PathBuf),
}

impl OutputDomainProbe {
    /// Resolve the coordinator endpoint for `domain` and build a probe.
    ///
    /// Returns a structured `ClientError` (rather than panicking) when
    /// `ANGZARR_MODE` / `ANGZARR_CH_PORT` are malformed — the runner
    /// surfaces this as a startup-time failure instead of unwinding the
    /// runtime mid-spawn.
    pub fn for_domain(domain: impl Into<String>) -> Result<Self> {
        let domain = domain.into();
        let raw = crate::transport::resolve_ch_endpoint(&domain, None, None, None, None)?;
        Ok(Self {
            domain,
            endpoint: parse_probe_endpoint(raw),
        })
    }
}

#[async_trait]
impl Probe for OutputDomainProbe {
    fn name(&self) -> &str {
        &self.domain
    }
    async fn check(&self) -> bool {
        match &self.endpoint {
            Endpoint::Tcp(addr) => tokio::net::TcpStream::connect(addr).await.is_ok(),
            Endpoint::Uds(path) => tokio::net::UnixStream::connect(path).await.is_ok(),
        }
    }
}

/// Audit #74: async-bus reachability probe — covers the path that
/// async-only saga / PM targets ride. The endpoint is operator-supplied
/// via [`ENV_BUS_ENDPOINT`] and points at whatever broker the
/// deployment uses (Kafka, RabbitMQ, SQS/SNS, NATS, etc.). The probe
/// is connection-only — it confirms the broker is reachable, not that
/// publishes will succeed end-to-end. Same contract as
/// [`OutputDomainProbe`] for sync targets.
pub struct BusProbe {
    endpoint: Endpoint,
}

impl BusProbe {
    fn from_endpoint(raw: String) -> Self {
        Self {
            endpoint: parse_probe_endpoint(raw),
        }
    }

    /// Build a [`BusProbe`] from [`ENV_BUS_ENDPOINT`], or `None` if the
    /// env var is unset / blank.
    pub fn from_env() -> Option<Self> {
        let raw = env::var(ENV_BUS_ENDPOINT).ok().filter(|s| !s.is_empty())?;
        Some(Self::from_endpoint(raw))
    }
}

#[async_trait]
impl Probe for BusProbe {
    fn name(&self) -> &str {
        "bus"
    }
    async fn check(&self) -> bool {
        match &self.endpoint {
            Endpoint::Tcp(addr) => tokio::net::TcpStream::connect(addr).await.is_ok(),
            Endpoint::Uds(path) => tokio::net::UnixStream::connect(path).await.is_ok(),
        }
    }
}

/// Run the readiness supervisor: poll every probe on each tick, aggregate
/// (`all_ok` → `SERVING`, else `NOT_SERVING`), and publish to every service
/// name registered with the [`HealthReporter`]. Loops until the task is
/// dropped — the runner spawns it alongside `Server::serve`.
pub async fn run_supervisor(
    probes: Vec<Box<dyn Probe>>,
    reporter: HealthReporter,
    service_names: Vec<String>,
    interval: Duration,
    timeout: Duration,
) {
    let never = Arc::new(tokio::sync::Notify::new());
    run_supervisor_with_wake(probes, reporter, service_names, interval, timeout, never).await
}

/// [`run_supervisor`] that also re-ticks as soon as `wake` is notified
/// (e.g. by [`TransportSignal::mark_bound`]) instead of only every
/// `interval`.
pub async fn run_supervisor_with_wake(
    probes: Vec<Box<dyn Probe>>,
    reporter: HealthReporter,
    service_names: Vec<String>,
    interval: Duration,
    timeout: Duration,
    wake: Arc<tokio::sync::Notify>,
) {
    loop {
        let all_ok = supervisor_tick(&probes, timeout).await;
        let status = if all_ok {
            ServingStatus::Serving
        } else {
            ServingStatus::NotServing
        };
        for name in &service_names {
            reporter.set_service_status(name, status).await;
        }
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = wake.notified() => {}
        }
    }
}

/// One iteration of the supervisor loop: poll every probe in parallel
/// with the configured timeout, return `true` iff all probes report
/// healthy.
///
/// Audit #82: wraps each probe future in `catch_unwind` so a panicking
/// probe doesn't unwind the spawned supervisor task. Each cause
/// (panic / timeout / probe-returned-false) emits its own `warn!`;
/// there is no aggregate "failed" log on top — the cause warning is
/// sufficient.
///
/// Probes evaluate concurrently via `FuturesUnordered` so a single
/// hung target only stalls its own slot up to `timeout`, not the whole
/// tick — the previous serial loop made the worst-case tick latency
/// `N * timeout`.
async fn supervisor_tick(probes: &[Box<dyn Probe>], timeout: Duration) -> bool {
    let mut futs: FuturesUnordered<_> = probes
        .iter()
        .map(|probe| {
            let name = probe.name().to_string();
            let probe_future = AssertUnwindSafe(probe.check()).catch_unwind();
            async move {
                let outcome = tokio::time::timeout(timeout, probe_future).await;
                evaluate_probe_outcome(&name, outcome)
            }
        })
        .collect();

    let mut all_ok = true;
    while let Some(ok) = futs.next().await {
        if !ok {
            all_ok = false;
        }
    }
    all_ok
}

/// Translate a probe's `(timeout × catch_unwind)` outcome into a
/// boolean, emitting a structured warning for each non-OK cause.
fn evaluate_probe_outcome(
    name: &str,
    outcome: std::result::Result<
        std::result::Result<bool, Box<dyn std::any::Any + Send>>,
        tokio::time::error::Elapsed,
    >,
) -> bool {
    match outcome {
        Ok(Ok(b)) => {
            if !b {
                warn!(probe = name, "readiness probe failed");
            }
            b
        }
        Ok(Err(panic_payload)) => {
            let msg = panic_payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<non-string panic>".into());
            warn!(
                probe = name,
                error = %msg,
                "readiness probe panicked",
            );
            false
        }
        Err(_elapsed) => {
            warn!(probe = name, "readiness probe timed out");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Audit #82: the supervisor must survive a panicking probe, distinguish
    // timeout from probe-returned-false in logs, and emit a panic message
    // rich enough to triage from log search.

    struct OkProbe;
    #[async_trait]
    impl Probe for OkProbe {
        fn name(&self) -> &str {
            "ok"
        }
        async fn check(&self) -> bool {
            true
        }
    }

    struct BadProbe;
    #[async_trait]
    impl Probe for BadProbe {
        fn name(&self) -> &str {
            "bad"
        }
        async fn check(&self) -> bool {
            false
        }
    }

    struct PanickingProbe {
        msg: &'static str,
    }
    #[async_trait]
    impl Probe for PanickingProbe {
        fn name(&self) -> &str {
            "panicker"
        }
        async fn check(&self) -> bool {
            panic!("{}", self.msg);
        }
    }

    struct PanickingStringProbe;
    #[async_trait]
    impl Probe for PanickingStringProbe {
        fn name(&self) -> &str {
            "string-panicker"
        }
        async fn check(&self) -> bool {
            // String (not &'static str) panic payload — exercises the
            // second downcast branch in `supervisor_tick`.
            let owned: String = format!("dynamic message {}", 42);
            panic!("{}", owned);
        }
    }

    struct SlowProbe {
        delay: Duration,
    }
    #[async_trait]
    impl Probe for SlowProbe {
        fn name(&self) -> &str {
            "slow"
        }
        async fn check(&self) -> bool {
            tokio::time::sleep(self.delay).await;
            true
        }
    }

    fn boxed(p: impl Probe + 'static) -> Box<dyn Probe> {
        Box::new(p)
    }

    #[tokio::test]
    async fn tick_returns_true_when_all_probes_ok() {
        let probes: Vec<Box<dyn Probe>> = vec![boxed(OkProbe), boxed(OkProbe)];
        let ok = supervisor_tick(&probes, Duration::from_millis(100)).await;
        assert!(ok);
    }

    #[tokio::test]
    async fn tick_returns_false_when_any_probe_returns_false() {
        let probes: Vec<Box<dyn Probe>> = vec![boxed(OkProbe), boxed(BadProbe)];
        let ok = supervisor_tick(&probes, Duration::from_millis(100)).await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn tick_survives_panicking_probe_and_returns_false() {
        // The critical fix: a panic inside `probe.check()` must not unwind
        // the supervisor task. With `catch_unwind`, the tick returns false
        // for the panicking probe and the supervisor lives.
        let probes: Vec<Box<dyn Probe>> = vec![
            boxed(OkProbe),
            boxed(PanickingProbe {
                msg: "boom &'static str",
            }),
            boxed(OkProbe),
        ];
        let ok = supervisor_tick(&probes, Duration::from_millis(100)).await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn tick_survives_string_panic_payload() {
        let probes: Vec<Box<dyn Probe>> = vec![boxed(PanickingStringProbe)];
        let ok = supervisor_tick(&probes, Duration::from_millis(100)).await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn tick_treats_slow_probe_as_failure_via_timeout() {
        let probes: Vec<Box<dyn Probe>> = vec![boxed(SlowProbe {
            delay: Duration::from_millis(200),
        })];
        let ok = supervisor_tick(&probes, Duration::from_millis(20)).await;
        assert!(!ok);
    }

    async fn status(svc: &tonic_health::server::HealthService) -> Option<i32> {
        use tonic_health::pb::health_server::Health;
        let req = tonic::Request::new(tonic_health::pb::HealthCheckRequest {
            service: "svc".into(),
        });
        svc.check(req).await.ok().map(|r| r.into_inner().status)
    }

    /// Marking the transport bound re-evaluates readiness immediately
    /// instead of waiting out the supervisor interval.
    #[tokio::test]
    async fn mark_bound_publishes_serving_without_waiting_for_the_interval() {
        let (reporter, _server) = tonic_health::server::health_reporter();
        let service = tonic_health::server::HealthService::from_health_reporter(reporter.clone());
        let (probe, signal) = TransportProbe::new();
        let wake = probe.wake();
        let handle = tokio::spawn(run_supervisor_with_wake(
            vec![boxed(probe)],
            reporter,
            vec!["svc".to_string()],
            Duration::from_secs(3600),
            Duration::from_millis(100),
            wake,
        ));
        let not_serving = tonic_health::pb::health_check_response::ServingStatus::NotServing as i32;
        let serving = tonic_health::pb::health_check_response::ServingStatus::Serving as i32;
        for _ in 0..100 {
            if status(&service).await == Some(not_serving) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(status(&service).await, Some(not_serving));
        signal.mark_bound();
        let mut last = None;
        for _ in 0..200 {
            last = status(&service).await;
            if last == Some(serving) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        handle.abort();
        assert_eq!(last, Some(serving));
    }

    #[tokio::test]
    async fn run_supervisor_lives_across_panicking_probes() {
        // End-to-end: spawn the supervisor with a panicking probe and a
        // very short interval; assert the task is still running after
        // several ticks. Pre-fix this would unwind the spawned task.
        let (reporter, _service) = tonic_health::server::health_reporter();
        let probes: Vec<Box<dyn Probe>> = vec![boxed(PanickingProbe {
            msg: "supervisor must survive this",
        })];

        let handle = tokio::spawn(run_supervisor(
            probes,
            reporter,
            vec!["svc".to_string()],
            Duration::from_millis(10),
            Duration::from_millis(50),
        ));

        // Give it a few ticks worth of wall-clock time.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            !handle.is_finished(),
            "supervisor exited early — panic catch broke",
        );
        handle.abort();
    }
}
