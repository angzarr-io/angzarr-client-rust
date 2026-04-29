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
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tonic_health::server::HealthReporter;
use tonic_health::ServingStatus;
use tracing::warn;

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
pub struct TransportProbe {
    bound: Arc<AtomicBool>,
}

impl TransportProbe {
    pub fn new() -> (Self, TransportSignal) {
        let bound = Arc::new(AtomicBool::new(false));
        (
            Self {
                bound: bound.clone(),
            },
            TransportSignal { bound },
        )
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
        self.bound.load(Ordering::SeqCst)
    }
}

/// Side of the [`TransportProbe`] used by the runner to mark "bound and serving".
pub struct TransportSignal {
    bound: Arc<AtomicBool>,
}

impl TransportSignal {
    /// Mark the transport as accepting traffic.
    pub fn mark_bound(&self) {
        self.bound.store(true, Ordering::SeqCst);
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
    /// Audit #40: a malformed `ANGZARR_MODE` / `ANGZARR_CH_PORT` env
    /// var here aborts startup with a clear panic message — readiness
    /// probes are configured once at startup and cannot run with a bad
    /// transport config. The underlying `resolve_ch_endpoint` returns
    /// `Result`; we surface the error via `expect` so operators see the
    /// typo before the server starts serving traffic.
    pub fn for_domain(domain: impl Into<String>) -> Self {
        let domain = domain.into();
        let raw = crate::transport::resolve_ch_endpoint(&domain, None, None, None, None)
            .expect("readiness probe: ANGZARR_MODE / ANGZARR_CH_PORT env config invalid");
        let endpoint = if let Some(path) = raw.strip_prefix("unix:") {
            Endpoint::Uds(PathBuf::from(path))
        } else if raw.starts_with('/') {
            Endpoint::Uds(PathBuf::from(raw))
        } else {
            Endpoint::Tcp(raw)
        };
        Self { domain, endpoint }
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
        let endpoint = if let Some(path) = raw.strip_prefix("unix:") {
            Endpoint::Uds(PathBuf::from(path))
        } else if raw.starts_with('/') {
            Endpoint::Uds(PathBuf::from(raw))
        } else {
            Endpoint::Tcp(raw)
        };
        Self { endpoint }
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
    loop {
        let mut all_ok = true;
        for probe in &probes {
            let ok: bool = tokio::time::timeout(timeout, probe.check())
                .await
                .unwrap_or_default();
            if !ok {
                all_ok = false;
                warn!(probe = probe.name(), "readiness probe failed");
            }
        }
        let status = if all_ok {
            ServingStatus::Serving
        } else {
            ServingStatus::NotServing
        };
        for name in &service_names {
            reporter.set_service_status(name, status).await;
        }
        tokio::time::sleep(interval).await;
    }
}
