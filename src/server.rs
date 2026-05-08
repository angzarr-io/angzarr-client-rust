//! gRPC runner utilities for hosting aggregate, saga, process manager,
//! projector, and upcaster services.
//!
//! Each `run_*_server` function:
//!
//! 1. Resolves transport from env via [`get_transport_config`] →
//!    [`ServerConfig`] (TCP or UDS — UDS path lives in the parent dir we
//!    create on demand, with any stale socket file removed).
//! 2. Reads the runner's logical name from the router (`router.name()`),
//!    so callers don't pass a redundant `domain`/`name` argument that can
//!    drift from the metadata on the registered handlers.
//! 3. Adds `grpc.health.v1.Health` alongside the kind-specific service.
//! 4. Spawns a [`crate::readiness`] supervisor whose probes are:
//!    - a [`crate::readiness::TransportProbe`] flipped once the listener is
//!      bound and the server is accepting traffic, and
//!    - one [`crate::readiness::OutputDomainProbe`] per `target` declared in
//!      the router's saga / process-manager handler metadata.
//!
//! While any probe is failing, the per-kind health service name and the empty
//! `""` overall name both report `NOT_SERVING`. K8s liveness sees the gRPC
//! server respond regardless; readiness only flips green once all probes do.

use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use tonic::transport::Server;
use tonic_health::server::HealthReporter;
use tonic_health::ServingStatus;
use tracing::{info, warn};

use crate::error::{ClientError, Result};
use crate::error_codes::{codes, keys, messages};
use crate::handler::{
    CommandHandlerGrpc, ProcessManagerGrpc, ProjectorGrpc, SagaGrpc, UpcasterGrpc,
};
use crate::proto::command_handler_service_server::CommandHandlerServiceServer;
use crate::proto::process_manager_service_server::ProcessManagerServiceServer;
use crate::proto::projector_service_server::ProjectorServiceServer;
use crate::proto::saga_service_server::SagaServiceServer;
use crate::proto::upcaster_service_server::UpcasterServiceServer;
use crate::readiness::{
    probe_config_from_env, run_supervisor, BusProbe, OutputDomainProbe, Probe, TransportProbe,
};
use crate::router::runtime::{CommandHandlerRouter, ProcessManagerRouter, SagaRouter};

/// Fully-qualified gRPC service names — matched against `Health.Check` and
/// used as health-reporter keys.
///
/// The `angzarr_client.proto.angzarr.` prefix is the **proto package**
/// (declared in `angzarr-project/proto/angzarr_client/proto/angzarr/*.proto`),
/// not a Rust-language identifier. All six sibling clients
/// (Python / Go / Java / C# / C++ / Rust) emit the same package because
/// they all generate from the same `.proto` files; the name is part of
/// the wire-format spec. Renaming would require coordinated proto-package
/// rename across every client + every coordinator.
const HEALTH_NAME_COMMAND_HANDLER: &str = "angzarr_client.proto.angzarr.CommandHandlerService";
const HEALTH_NAME_SAGA: &str = "angzarr_client.proto.angzarr.SagaService";
const HEALTH_NAME_PROCESS_MANAGER: &str = "angzarr_client.proto.angzarr.ProcessManagerService";
const HEALTH_NAME_PROJECTOR: &str = "angzarr_client.proto.angzarr.ProjectorService";
const HEALTH_NAME_UPCASTER: &str = "angzarr_client.proto.angzarr.UpcasterService";

/// Initialize a JSON tracing subscriber filtered by `RUST_LOG` (default `info`).
///
/// Idempotent — `try_init` swallows the "already set" error so a second call
/// in the same process is a no-op.
pub fn configure_logging() {
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
}

/// Configuration for a gRPC runner.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// TCP port (used when `uds_path` is `None`).
    pub port: u16,
    /// Unix domain socket path. When `Some`, supersedes `port`.
    pub uds_path: Option<PathBuf>,
}

impl ServerConfig {
    /// Resolve from env. UDS mode is selected when all three of `UDS_BASE_PATH`,
    /// `SERVICE_NAME`, and `DOMAIN` are set; otherwise TCP, with port read from
    /// `PORT` or `GRPC_PORT`, falling back to `default_port`.
    ///
    /// **Naming note**: server-side reads `UDS_BASE_PATH` (no
    /// `ANGZARR_` prefix) while client-side
    /// [`crate::transport::resolve_ch_endpoint`] reads `ANGZARR_UDS_BASE`
    /// — this asymmetry is the **established cross-language convention**:
    /// Python (`server.py` reads `UDS_BASE_PATH`, `client.py` reads
    /// `ANGZARR_UDS_BASE`), Go (`server.go` reads `UDS_BASE_PATH`,
    /// `client.go` reads `ANGZARR_UDS_BASE`), and so on. Aligning the
    /// names would require coordinating all six clients in lockstep
    /// with deployment manifests in the field. Do not "fix" this in
    /// isolation.
    ///
    /// This function is **pure** — no filesystem side effects. The runner is
    /// responsible for creating the parent directory and removing any stale
    /// socket file at the chosen path.
    pub fn from_env(default_port: u16) -> Self {
        if let (Ok(base_path), Ok(service_name), Ok(domain)) = (
            env::var("UDS_BASE_PATH"),
            env::var("SERVICE_NAME"),
            env::var("DOMAIN"),
        ) {
            let socket_name = format!("{}-{}.sock", service_name, domain);
            let uds_path = PathBuf::from(base_path).join(socket_name);
            return Self {
                port: default_port,
                uds_path: Some(uds_path),
            };
        }
        let port = env::var("PORT")
            .or_else(|_| env::var("GRPC_PORT"))
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default_port);
        Self {
            port,
            uds_path: None,
        }
    }
}

/// Resolve transport configuration from environment. Single canonical entry
/// point — the only env reader callers should use.
pub fn get_transport_config(default_port: u16) -> ServerConfig {
    ServerConfig::from_env(default_port)
}

/// Env var name for the full TCP bind address override (`host:port`).
///
/// Audit #77: when set, supersedes the default `[::]:{port}` composition
/// and the `PORT` / `GRPC_PORT` resolution. IPv6 hosts must include
/// brackets (e.g. `[::1]:50052`); IPv4 hosts are written bare.
pub const ENV_BIND_ADDRESS: &str = "ANGZARR_BIND_ADDRESS";

/// Default TCP bind host. `"[::]"` is the IPv6 wildcard, which on
/// Linux (`IPV6_V6ONLY=0` by default) accepts both IPv4 (via IPv4-mapped
/// IPv6) and IPv6 connections — matching Python's posture per audit #77.
pub const DEFAULT_BIND_HOST: &str = "[::]";

/// Compute the TCP bind address.
///
/// Returns `ANGZARR_BIND_ADDRESS` verbatim when set, otherwise composes
/// `[::]:{default_port}`. Pure read of env state; intended to be called
/// once per server start, immediately before `parse::<SocketAddr>()`.
pub fn resolve_bind_address(default_port: u16) -> String {
    env::var(ENV_BIND_ADDRESS).unwrap_or_else(|_| format!("{}:{}", DEFAULT_BIND_HOST, default_port))
}

/// Construct a fresh `tonic::transport::Server` builder.
pub fn create_server() -> Server {
    Server::builder()
}

/// Run a server for any [`crate::router::Built`] router kind.
///
/// Dispatches to the per-kind `run_*_server` function based on the variant.
pub async fn run_server(default_port: u16, built: crate::router::Built) -> Result<()> {
    match built {
        crate::router::Built::CommandHandler(router) => {
            run_command_handler_server(router, default_port).await
        }
        crate::router::Built::Saga(router) => run_saga_server(router, default_port).await,
        crate::router::Built::ProcessManager(router) => {
            run_process_manager_server(router, default_port).await
        }
        crate::router::Built::Projector(router) => run_projector_server(router, default_port).await,
        crate::router::Built::Upcaster(router) => run_upcaster_server(router, default_port).await,
    }
}

/// Remove a stale UDS socket file at `path`. No-op if the path does not exist.
pub fn cleanup_socket(path: impl AsRef<Path>) {
    let p = path.as_ref();
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
}

/// Parse a `host:port` string into a `SocketAddr`, surfacing a structured
/// `INVALID_BIND_ADDRESS` error instead of panicking on operator typos.
pub(crate) fn parse_bind_address(addr_str: &str) -> Result<SocketAddr> {
    addr_str.parse::<SocketAddr>().map_err(|e| {
        ClientError::invalid_argument(
            codes::INVALID_BIND_ADDRESS,
            messages::INVALID_BIND_ADDRESS,
            [
                (keys::INPUT, addr_str.to_string()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })
}

/// Ensure the parent directory exists for a UDS path, surfacing a
/// structured error if the create fails (read-only fs, EACCES, etc.).
pub(crate) fn ensure_uds_parent_dir(uds_path: &Path) -> Result<()> {
    let Some(parent) = uds_path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|e| {
        ClientError::connection(
            codes::UDS_DIRECTORY_CREATE_FAILED,
            messages::UDS_DIRECTORY_CREATE_FAILED,
            [
                (keys::INPUT, parent.display().to_string()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })
}

/// Bind a Unix domain socket, surfacing a structured `UDS_BIND_FAILED`
/// error rather than panicking.
pub(crate) fn bind_uds_listener(uds_path: &Path) -> Result<tokio::net::UnixListener> {
    tokio::net::UnixListener::bind(uds_path).map_err(|e| {
        ClientError::connection(
            codes::UDS_BIND_FAILED,
            messages::UDS_BIND_FAILED,
            [
                (keys::INPUT, uds_path.display().to_string()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })
}

/// Future that resolves on the first SIGINT (Ctrl+C) or, on Unix, SIGTERM.
///
/// Wired into `Server::serve_with_shutdown` so the server drains in-flight
/// requests on a signal instead of being killed mid-stream by the runtime.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!(error = %e, "failed to install ctrl_c handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                warn!(error = %e, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

// ---------------------------------------------------------------------------
// Per-kind runners
// ---------------------------------------------------------------------------

/// Run a command handler service. Domain is read from the router's
/// `#[command_handler(domain = ...)]` metadata.
pub async fn run_command_handler_server(
    router: CommandHandlerRouter,
    default_port: u16,
) -> Result<()> {
    let name = router.name();
    // CH never emits cross-domain commands at the framework level —
    // events flow back through the response. No probes.
    let svc = CommandHandlerServiceServer::new(CommandHandlerGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        Vec::new(),
        false,
        HEALTH_NAME_COMMAND_HANDLER,
        |r| r.add_service(svc),
    )
    .await
}

/// Run a saga service. Saga name is read from the router's `#[saga(name = ...)]`
/// metadata. Audit #74: only `target`s declared with `#[saga(sync = true)]`
/// get an `OutputDomainProbe`; async-only sagas rely on the `BusProbe`
/// (configured via `ANGZARR_BUS_ENDPOINT`).
pub async fn run_saga_server(router: SagaRouter, default_port: u16) -> Result<()> {
    let name = router.name();
    let sync_outputs = router.sync_output_domains();
    let has_async_outputs = router.has_async_outputs();
    let svc = SagaServiceServer::new(SagaGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        sync_outputs,
        has_async_outputs,
        HEALTH_NAME_SAGA,
        |r| r.add_service(svc),
    )
    .await
}

/// Run a projector service. Projector name is read from the
/// `#[projector(name = ...)]` metadata. Projectors are read-side and have no
/// output-domain probes.
pub async fn run_projector_server(
    router: crate::router::ProjectorRouter,
    default_port: u16,
) -> Result<()> {
    let name = router.name();
    let svc = ProjectorServiceServer::new(ProjectorGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        Vec::new(),
        false,
        HEALTH_NAME_PROJECTOR,
        |r| r.add_service(svc),
    )
    .await
}

/// Run a process-manager service. PM name is read from
/// `#[process_manager(name = ...)]` metadata. Audit #74: only targets
/// listed in `sync_targets` get an `OutputDomainProbe`; async-only
/// targets ride the bus probe.
pub async fn run_process_manager_server(
    router: ProcessManagerRouter,
    default_port: u16,
) -> Result<()> {
    let name = router.name();
    let sync_outputs = router.sync_output_domains();
    let has_async_outputs = router.has_async_outputs();
    let svc = ProcessManagerServiceServer::new(ProcessManagerGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        sync_outputs,
        has_async_outputs,
        HEALTH_NAME_PROCESS_MANAGER,
        |r| r.add_service(svc),
    )
    .await
}

/// Run an upcaster service. Upcaster name is read from
/// `#[upcaster(name = ...)]` metadata. Upcasters have no output-domain probes.
pub async fn run_upcaster_server(
    router: crate::router::upcaster::UpcasterRouter,
    default_port: u16,
) -> Result<()> {
    let name = router.name();
    let svc = UpcasterServiceServer::new(UpcasterGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        Vec::new(),
        false,
        HEALTH_NAME_UPCASTER,
        |r| r.add_service(svc),
    )
    .await
}

// ---------------------------------------------------------------------------
// Shared runner core
// ---------------------------------------------------------------------------

/// Common runner body shared by every per-kind `run_*_server`:
/// builds probes + health, marks transport bound after the listener succeeds,
/// then serves until either the server future resolves or a SIGINT/SIGTERM
/// signal triggers graceful shutdown.
async fn run_kind<F>(
    instance_name: String,
    config: ServerConfig,
    sync_output_domains: Vec<String>,
    has_async_outputs: bool,
    health_service_name: &'static str,
    add_kind_service: F,
) -> Result<()>
where
    F: FnOnce(tonic::transport::server::Router) -> tonic::transport::server::Router,
{
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    let service_names: Vec<String> = vec![String::new(), health_service_name.to_string()];
    for name in &service_names {
        health_reporter
            .set_service_status(name, ServingStatus::NotServing)
            .await;
    }

    let (transport_probe, transport_signal) = TransportProbe::new();
    let mut probes: Vec<Box<dyn Probe>> = vec![Box::new(transport_probe)];
    // Audit #74: probe sync targets directly; if any handler emits async,
    // add a single BusProbe iff the operator configured `ANGZARR_BUS_ENDPOINT`.
    for domain in sync_output_domains {
        probes.push(Box::new(OutputDomainProbe::for_domain(domain)?));
    }
    if has_async_outputs {
        if let Some(bus) = BusProbe::from_env() {
            probes.push(Box::new(bus));
        }
    }

    let (interval, timeout) = probe_config_from_env();
    // Audit #83: clones held for the shutdown flip. `HealthReporter` is
    // `Clone`; `service_names` is owned by the supervisor task.
    let shutdown_reporter = health_reporter.clone();
    let shutdown_service_names = service_names.clone();
    let supervisor = tokio::spawn(run_supervisor(
        probes,
        health_reporter,
        service_names,
        interval,
        timeout,
    ));

    let server = Server::builder().add_service(health_service);
    let router = add_kind_service(server);

    // Resolve listener / address up front so binding errors surface
    // immediately as structured `ClientError`s instead of unwinding
    // the runtime from inside `run_kind`. Track the UDS path so we can
    // remove it after `serve` resolves.
    let uds_to_cleanup = config.uds_path.clone();
    let result = match config.uds_path.as_ref() {
        Some(uds_path) => {
            ensure_uds_parent_dir(uds_path)?;
            cleanup_socket(uds_path);
            // Audit #89: cross-language log shape. Same event name +
            // field set as Python `_run_server_async` so operators
            // querying logs by `service` / `name` / `transport` /
            // `address` see equivalent records from pods of either
            // language.
            info!(
                service = health_service_name,
                name = %instance_name,
                transport = "uds",
                address = %uds_path.display(),
                "server_started",
            );
            let listener = bind_uds_listener(uds_path)?;
            let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
            transport_signal.mark_bound();
            router
                .serve_with_incoming_shutdown(incoming, shutdown_signal())
                .await
        }
        None => {
            let addr_str = resolve_bind_address(config.port);
            let addr = parse_bind_address(&addr_str)?;
            info!(
                service = health_service_name,
                name = %instance_name,
                transport = "tcp",
                address = %addr_str,
                "server_started",
            );
            transport_signal.mark_bound();
            router.serve_with_shutdown(addr, shutdown_signal()).await
        }
    };

    // Audit #83: shut down in two phases.
    // 1. Cancel the supervisor and wait for it to actually exit so any
    //    in-flight `set_service_status` finishes before we publish the
    //    final state. `JoinError` from the abort path is expected and
    //    swallowed; any other panic from the supervisor was already
    //    caught at the probe level (audit #82).
    // 2. Flip every registered health name to `NOT_SERVING` so K8s
    //    readiness goes red and the load balancer drains the pod.
    supervisor.abort();
    let _ = supervisor.await;
    publish_shutdown_status(&shutdown_reporter, &shutdown_service_names).await;

    // Always remove the UDS socket file on shutdown — leaving it
    // behind makes the next start fail with EADDRINUSE if the runner
    // is restarted before kubelet cleans up the volume.
    if let Some(path) = uds_to_cleanup {
        cleanup_socket(&path);
    }

    // Audit #89: same event name + field set on shutdown.
    info!(
        service = health_service_name,
        name = %instance_name,
        "server_shutdown",
    );
    result.map_err(ClientError::from)
}

/// Audit #83: flip every registered health name to `NOT_SERVING` so the
/// load balancer drains the pod. Extracted so the shutdown publish can
/// be unit-tested in isolation from the runner's transport plumbing.
async fn publish_shutdown_status(reporter: &HealthReporter, service_names: &[String]) {
    for name in service_names {
        reporter
            .set_service_status(name, ServingStatus::NotServing)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_bind_env() {
        env::remove_var(ENV_BIND_ADDRESS);
    }

    // Audit #77: ANGZARR_BIND_ADDRESS overrides the default
    // dual-stack `[::]:{port}` composition.

    #[test]
    fn resolve_bind_address_default_is_dual_stack() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_bind_env();
        let addr = resolve_bind_address(50052);
        assert_eq!(addr, "[::]:50052");
        // Sanity: parses as a real SocketAddr.
        let _: SocketAddr = addr.parse().expect("default must parse");
    }

    #[test]
    fn resolve_bind_address_env_override_ipv4() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_bind_env();
        env::set_var(ENV_BIND_ADDRESS, "127.0.0.1:9090");
        let addr = resolve_bind_address(50052);
        assert_eq!(addr, "127.0.0.1:9090");
        let _: SocketAddr = addr.parse().expect("override must parse");
        clear_bind_env();
    }

    #[test]
    fn resolve_bind_address_env_override_ipv6_loopback() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_bind_env();
        env::set_var(ENV_BIND_ADDRESS, "[::1]:8080");
        let addr = resolve_bind_address(50052);
        assert_eq!(addr, "[::1]:8080");
        let _: SocketAddr = addr.parse().expect("ipv6 override must parse");
        clear_bind_env();
    }

    #[test]
    fn resolve_bind_address_env_override_ignores_default_port() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_bind_env();
        env::set_var(ENV_BIND_ADDRESS, "0.0.0.0:1234");
        let addr = resolve_bind_address(50052);
        // The default_port arg is irrelevant when the override is set.
        assert_eq!(addr, "0.0.0.0:1234");
        clear_bind_env();
    }

    // Audit #83: shutdown flips every registered health name to
    // NOT_SERVING so the K8s load balancer drains the pod.

    /// Read the current `ServingStatus` from the reporter's shared
    /// state by wiring a fresh `HealthService` over a clone of the
    /// reporter and calling its gRPC `check` method.
    async fn read_health_status(
        reporter: &HealthReporter,
        name: &str,
    ) -> tonic_health::pb::health_check_response::ServingStatus {
        use tonic::Request;
        use tonic_health::pb::HealthCheckRequest;
        use tonic_health::server::HealthService;
        let service = HealthService::from_health_reporter(reporter.clone());
        let req = Request::new(HealthCheckRequest {
            service: name.to_string(),
        });
        let resp = tonic_health::pb::health_server::Health::check(&service, req)
            .await
            .expect("check must succeed for a registered service");
        resp.into_inner().status()
    }

    #[tokio::test]
    async fn publish_shutdown_status_flips_every_name_to_not_serving() {
        let (reporter, _service) = tonic_health::server::health_reporter();
        let names: Vec<String> = vec![
            String::new(), // empty/overall name
            "svc.A".to_string(),
            "svc.B".to_string(),
        ];

        // Start every name at SERVING — this is the steady state once
        // the readiness supervisor has flipped them green.
        for name in &names {
            reporter
                .set_service_status(name, ServingStatus::Serving)
                .await;
        }
        for name in &names {
            assert_eq!(
                read_health_status(&reporter, name).await,
                tonic_health::pb::health_check_response::ServingStatus::Serving,
            );
        }

        publish_shutdown_status(&reporter, &names).await;

        for name in &names {
            assert_eq!(
                read_health_status(&reporter, name).await,
                tonic_health::pb::health_check_response::ServingStatus::NotServing,
                "shutdown must flip {name:?} to NOT_SERVING",
            );
        }
    }

    #[test]
    fn parse_bind_address_accepts_ipv4_and_ipv6() {
        assert!(parse_bind_address("127.0.0.1:8080").is_ok());
        assert!(parse_bind_address("[::1]:9090").is_ok());
        assert!(parse_bind_address("[::]:50052").is_ok());
    }

    #[test]
    fn parse_bind_address_rejects_garbage_with_invalid_bind_address_code() {
        let err = parse_bind_address("not-a-real-address").unwrap_err();
        assert_eq!(err.code(), codes::INVALID_BIND_ADDRESS);
        // Operator should see the original input echoed back.
        if let ClientError::InvalidArgument(d) = err {
            assert_eq!(d.details[keys::INPUT], "not-a-real-address");
        } else {
            panic!("expected InvalidArgument");
        }
    }

    #[test]
    fn parse_bind_address_rejects_empty_string() {
        let err = parse_bind_address("").unwrap_err();
        assert_eq!(err.code(), codes::INVALID_BIND_ADDRESS);
    }

    #[test]
    fn ensure_uds_parent_dir_creates_missing_parent() {
        let tmpdir = std::env::temp_dir().join(format!(
            "angzarr-uds-{}",
            std::process::id(),
        ));
        // Make sure we start clean.
        let _ = std::fs::remove_dir_all(&tmpdir);
        let socket_path = tmpdir.join("nested/dir/foo.sock");
        ensure_uds_parent_dir(&socket_path).expect("must create parent");
        assert!(tmpdir.join("nested/dir").is_dir());
        let _ = std::fs::remove_dir_all(&tmpdir);
    }

    #[test]
    fn ensure_uds_parent_dir_surfaces_failure_with_structured_code() {
        // /proc is read-only on Linux; creating a directory under it
        // surfaces as UDS_DIRECTORY_CREATE_FAILED rather than panicking
        // or being silently swallowed (the previous `let _ = ...` did
        // the latter).
        let bogus = PathBuf::from("/proc/this-cannot-be-created/foo.sock");
        let result = ensure_uds_parent_dir(&bogus);
        match result {
            Err(e) => assert_eq!(e.code(), codes::UDS_DIRECTORY_CREATE_FAILED),
            Ok(()) => {
                // /proc allows directory creation in some unprivileged
                // sandboxes — skip rather than fail spuriously.
                let _ = std::fs::remove_dir_all("/proc/this-cannot-be-created");
            }
        }
    }

    #[tokio::test]
    async fn publish_shutdown_status_no_names_is_noop() {
        let (reporter, _service) = tonic_health::server::health_reporter();
        // Should not panic, should not register a name we never asked
        // for. Empty input is the legal "no services" case (won't
        // happen in run_kind today but the helper is general-purpose).
        publish_shutdown_status(&reporter, &[]).await;
    }
}
