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
use tonic_health::ServingStatus;
use tracing::info;

use crate::handler::{
    CommandHandlerGrpc, ProcessManagerGrpc, ProjectorGrpc, SagaGrpc, UpcasterGrpc,
};
use crate::proto::command_handler_service_server::CommandHandlerServiceServer;
use crate::proto::process_manager_service_server::ProcessManagerServiceServer;
use crate::proto::projector_service_server::ProjectorServiceServer;
use crate::proto::saga_service_server::SagaServiceServer;
use crate::proto::upcaster_service_server::UpcasterServiceServer;
use crate::readiness::{
    probe_config_from_env, run_supervisor, OutputDomainProbe, Probe, TransportProbe,
};
use crate::router::runtime::{CommandHandlerRouter, ProcessManagerRouter, SagaRouter};

/// Fully-qualified gRPC service names — matched against `Health.Check` and
/// used as health-reporter keys.
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

/// Construct a fresh `tonic::transport::Server` builder.
pub fn create_server() -> Server {
    Server::builder()
}

/// Run a server for any [`crate::router::Built`] router kind.
///
/// Dispatches to the per-kind `run_*_server` function based on the variant.
pub async fn run_server(
    default_port: u16,
    built: crate::router::Built,
) -> Result<(), tonic::transport::Error> {
    match built {
        crate::router::Built::CommandHandler(router) => {
            run_command_handler_server(router, default_port).await
        }
        crate::router::Built::Saga(router) => run_saga_server(router, default_port).await,
        crate::router::Built::ProcessManager(router) => {
            run_process_manager_server(router, default_port).await
        }
        crate::router::Built::Projector(router) => {
            run_projector_server(router, default_port).await
        }
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

// ---------------------------------------------------------------------------
// Per-kind runners
// ---------------------------------------------------------------------------

/// Run a command handler service. Domain is read from the router's
/// `#[command_handler(domain = ...)]` metadata.
pub async fn run_command_handler_server(
    router: CommandHandlerRouter,
    default_port: u16,
) -> Result<(), tonic::transport::Error> {
    let name = router.name();
    let outputs = router.output_domains();
    let svc = CommandHandlerServiceServer::new(CommandHandlerGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        outputs,
        HEALTH_NAME_COMMAND_HANDLER,
        |r| r.add_service(svc),
    )
    .await
}

/// Run a saga service. Saga name is read from the router's `#[saga(name = ...)]`
/// metadata. Output-domain probes are constructed from each handler's `target`.
pub async fn run_saga_server(
    router: SagaRouter,
    default_port: u16,
) -> Result<(), tonic::transport::Error> {
    let name = router.name();
    let outputs = router.output_domains();
    let svc = SagaServiceServer::new(SagaGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        outputs,
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
) -> Result<(), tonic::transport::Error> {
    let name = router.name();
    let svc = ProjectorServiceServer::new(ProjectorGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        Vec::new(),
        HEALTH_NAME_PROJECTOR,
        |r| r.add_service(svc),
    )
    .await
}

/// Run a process-manager service. PM name is read from
/// `#[process_manager(name = ...)]` metadata. Output-domain probes are built
/// from the union of `targets` declared across registered handlers.
pub async fn run_process_manager_server(
    router: ProcessManagerRouter,
    default_port: u16,
) -> Result<(), tonic::transport::Error> {
    let name = router.name();
    let outputs = router.output_domains();
    let svc = ProcessManagerServiceServer::new(ProcessManagerGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        outputs,
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
) -> Result<(), tonic::transport::Error> {
    let name = router.name();
    let svc = UpcasterServiceServer::new(UpcasterGrpc::new(router));
    run_kind(
        name,
        get_transport_config(default_port),
        Vec::new(),
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
/// then serves until the future resolves.
async fn run_kind<F>(
    instance_name: String,
    config: ServerConfig,
    output_domains: Vec<String>,
    health_service_name: &'static str,
    add_kind_service: F,
) -> Result<(), tonic::transport::Error>
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
    for domain in output_domains {
        probes.push(Box::new(OutputDomainProbe::for_domain(domain)));
    }

    let (interval, timeout) = probe_config_from_env();
    let supervisor = tokio::spawn(run_supervisor(
        probes,
        health_reporter,
        service_names,
        interval,
        timeout,
    ));

    let server = Server::builder().add_service(health_service);
    let router = add_kind_service(server);

    let result = match config.uds_path.as_ref() {
        Some(uds_path) => {
            if let Some(parent) = uds_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            cleanup_socket(uds_path);
            info!(
                name = %instance_name,
                path = %uds_path.display(),
                "Starting server (UDS)"
            );
            let listener =
                tokio::net::UnixListener::bind(uds_path).expect("Failed to bind UDS socket");
            let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
            transport_signal.mark_bound();
            router.serve_with_incoming(incoming).await
        }
        None => {
            let addr: SocketAddr = format!("0.0.0.0:{}", config.port)
                .parse()
                .expect("invalid TCP bind address");
            info!(
                name = %instance_name,
                port = config.port,
                "Starting server (TCP)"
            );
            transport_signal.mark_bound();
            router.serve(addr).await
        }
    };

    supervisor.abort();
    result
}
