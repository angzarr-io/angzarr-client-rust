//! gRPC server utilities for running aggregate and saga services.
//!
//! This module provides helpers for starting gRPC servers with TCP or UDS transport.

use std::env;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use tonic::transport::Server;
use tracing::info;

/// Initialize default tracing/logging for a server.
///
/// Cross-language alias for Python's `configure_logging`. Installs a
/// `tracing_subscriber` with `RUST_LOG` env filtering and ISO-style output
/// if no subscriber is already set. Safe to call multiple times (a second
/// call is a no-op).
pub fn configure_logging() {
    // Use `try_init` so re-entry doesn't panic.
    let _ = tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default());
}

/// Resolve transport configuration from environment. Mirrors Python's
/// `get_transport_config` — returns `(transport_type, address)`:
/// - `("tcp", "[::]:{port}")` by default
/// - `("uds", "unix:{path}")` when `TRANSPORT_TYPE=uds`
pub fn get_transport_config() -> (String, String) {
    let transport = env::var("TRANSPORT_TYPE").unwrap_or_else(|_| "tcp".into());
    if transport.eq_ignore_ascii_case("uds") {
        let base = env::var("UDS_BASE_PATH").unwrap_or_else(|_| "/tmp/angzarr".into());
        let service = env::var("SERVICE_NAME").unwrap_or_else(|_| "business".into());
        let qualifier = env::var("DOMAIN")
            .or_else(|_| env::var("SAGA_NAME"))
            .or_else(|_| env::var("PROJECTOR_NAME"))
            .unwrap_or_default();
        let socket_name = if qualifier.is_empty() {
            format!("{}.sock", service)
        } else {
            format!("{}-{}.sock", service, qualifier)
        };
        let path = format!("{}/{}", base.trim_end_matches('/'), socket_name);
        ("uds".into(), format!("unix:{}", path))
    } else {
        let port = env::var("PORT").unwrap_or_else(|_| "50052".into());
        ("tcp".into(), format!("[::]:{}", port))
    }
}

/// Construct a fresh tonic `Server` builder. Cross-language alias for
/// Python's `create_server` — returns a builder the caller attaches
/// services to via `.add_service(...)` before `.serve(...)`.
pub fn create_server() -> Server {
    Server::builder()
}

/// Run a server for any [`crate::router::Built`] router kind.
///
/// Dispatches to the per-kind `run_*_server` function based on the
/// `Built` variant. Honors the env-var-driven TCP/UDS transport selection.
/// Cross-language alias for Python's generic `run_server(...)`.
pub async fn run_server(
    name: &str,
    default_port: u16,
    built: crate::router::Built,
) -> Result<(), tonic::transport::Error> {
    match built {
        crate::router::Built::CommandHandler(router) => {
            run_command_handler_server(name, default_port, router).await
        }
        crate::router::Built::Saga(router) => run_saga_server(name, default_port, router).await,
        crate::router::Built::ProcessManager(router) => {
            run_process_manager_server(name, default_port, router).await
        }
        crate::router::Built::Projector(router) => {
            let handler = crate::handler::ProjectorGrpc::new(router);
            run_projector_server(name, default_port, handler).await
        }
        crate::router::Built::Upcaster(router) => {
            run_upcaster_server(name, default_port, router).await
        }
    }
}

/// Remove a stale UDS socket file at `path`. Mirrors Python's
/// `cleanup_socket`. No-op if the path does not exist.
pub fn cleanup_socket(path: impl AsRef<Path>) {
    let p = path.as_ref();
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
}

use crate::handler::{
    CommandHandlerGrpc, ProcessManagerGrpc, ProjectorGrpc, SagaGrpc, UpcasterGrpc,
};
use crate::proto::command_handler_service_server::CommandHandlerServiceServer;
use crate::proto::process_manager_service_server::ProcessManagerServiceServer;
use crate::proto::projector_service_server::ProjectorServiceServer;
use crate::proto::saga_service_server::SagaServiceServer;
use crate::proto::upcaster_service_server::UpcasterServiceServer;
use crate::router::runtime::{CommandHandlerRouter, ProcessManagerRouter, SagaRouter};

/// Configuration for a gRPC server.
pub struct ServerConfig {
    /// Port to listen on (TCP mode).
    pub port: u16,
    /// Unix domain socket path (UDS mode).
    pub uds_path: Option<PathBuf>,
}

impl ServerConfig {
    /// Create config from environment variables.
    ///
    /// UDS mode (standalone):
    /// - `UDS_BASE_PATH`: Base directory for UDS sockets
    /// - `SERVICE_NAME`: Service name (e.g., "business")
    /// - `DOMAIN`: Domain name (e.g., "player")
    ///   => Socket path: `{UDS_BASE_PATH}/{SERVICE_NAME}-{DOMAIN}.sock`
    ///
    /// TCP mode (distributed):
    /// - `PORT` or `GRPC_PORT`: TCP port (default: `default_port`)
    pub fn from_env(default_port: u16) -> Self {
        // Check for UDS mode first
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

        // Fall back to TCP mode
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

/// Run a command handler service with the given router.
///
/// Supports both TCP and Unix domain socket (UDS) transport.
/// UDS is used when `UDS_BASE_PATH`, `SERVICE_NAME`, and `DOMAIN` env vars are set.
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::{run_command_handler_server, CommandHandlerRouter};
///
/// #[tokio::main]
/// async fn main() {
///     let router = CommandHandlerRouter::new("player", "player", PlayerHandler::new());
///
///     run_command_handler_server("player", 50001, router).await;
/// }
/// ```
pub async fn run_command_handler_server(
    domain: &str,
    default_port: u16,
    router: CommandHandlerRouter,
) -> Result<(), tonic::transport::Error> {
    let config = ServerConfig::from_env(default_port);
    let handler = CommandHandlerGrpc::new(router);
    let service = CommandHandlerServiceServer::new(handler);

    if let Some(uds_path) = &config.uds_path {
        // UDS mode (standalone)
        info!(
            domain = domain,
            path = %uds_path.display(),
            "Starting command handler server (UDS)"
        );

        // Remove existing socket file if present
        let _ = std::fs::remove_file(uds_path);

        let uds = tokio::net::UnixListener::bind(uds_path).expect("Failed to bind UDS socket");
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds);

        Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await
    } else {
        // TCP mode (distributed)
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse().unwrap();

        info!(
            domain = domain,
            port = config.port,
            "Starting command handler server"
        );

        Server::builder().add_service(service).serve(addr).await
    }
}

/// Run a saga service with the given router.
///
/// Supports both TCP and Unix domain socket (UDS) transport.
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::{run_saga_server, SagaRouter};
///
/// #[tokio::main]
/// async fn main() {
///     let router = SagaRouter::new("saga-order-fulfillment", "order", "fulfillment", OrderHandler::new());
///
///     run_saga_server("saga-order-fulfillment", 50010, router).await;
/// }
/// ```
pub async fn run_saga_server(
    name: &str,
    default_port: u16,
    router: SagaRouter,
) -> Result<(), tonic::transport::Error> {
    let config = ServerConfig::from_env(default_port);
    let handler = SagaGrpc::new(router);
    let service = SagaServiceServer::new(handler);

    if let Some(uds_path) = &config.uds_path {
        // UDS mode
        info!(
            name = name,
            path = %uds_path.display(),
            "Starting saga server (UDS)"
        );

        let _ = std::fs::remove_file(uds_path);

        let uds = tokio::net::UnixListener::bind(uds_path).expect("Failed to bind UDS socket");
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds);

        Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await
    } else {
        // TCP mode
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse().unwrap();

        info!(name = name, port = config.port, "Starting saga server");

        Server::builder().add_service(service).serve(addr).await
    }
}

/// Run a projector service with the given handler.
///
/// Supports both TCP and Unix domain socket (UDS) transport.
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::{run_projector_server, ProjectorGrpc};
///
/// #[tokio::main]
/// async fn main() {
///     let handler = ProjectorGrpc::new("output").with_handle(handle_events);
///
///     run_projector_server("output", 9090, handler).await;
/// }
/// ```
pub async fn run_projector_server(
    name: &str,
    default_port: u16,
    handler: ProjectorGrpc,
) -> Result<(), tonic::transport::Error> {
    let config = ServerConfig::from_env(default_port);
    let service = ProjectorServiceServer::new(handler);

    if let Some(uds_path) = &config.uds_path {
        // UDS mode
        info!(
            name = name,
            path = %uds_path.display(),
            "Starting projector server (UDS)"
        );

        let _ = std::fs::remove_file(uds_path);

        let uds = tokio::net::UnixListener::bind(uds_path).expect("Failed to bind UDS socket");
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds);

        Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await
    } else {
        // TCP mode
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse().unwrap();

        info!(name = name, port = config.port, "Starting projector server");

        Server::builder().add_service(service).serve(addr).await
    }
}

/// Run a process manager service with the given router.
///
/// Supports both TCP and Unix domain socket (UDS) transport.
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::{run_process_manager_server, ProcessManagerRouter};
///
/// #[tokio::main]
/// async fn main() {
///     let router = ProcessManagerRouter::new("hand-flow", "hand-flow", rebuild_state)
///         .domain("table", TablePmHandler::new());
///
///     run_process_manager_server("hand-flow", 9091, router).await;
/// }
/// ```
pub async fn run_process_manager_server(
    name: &str,
    default_port: u16,
    router: ProcessManagerRouter,
) -> Result<(), tonic::transport::Error> {
    let config = ServerConfig::from_env(default_port);
    let handler = ProcessManagerGrpc::new(router);
    let service = ProcessManagerServiceServer::new(handler);

    if let Some(uds_path) = &config.uds_path {
        // UDS mode
        info!(
            name = name,
            path = %uds_path.display(),
            "Starting process manager server (UDS)"
        );

        let _ = std::fs::remove_file(uds_path);

        let uds = tokio::net::UnixListener::bind(uds_path).expect("Failed to bind UDS socket");
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds);

        Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await
    } else {
        // TCP mode
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse().unwrap();

        info!(
            name = name,
            port = config.port,
            "Starting process manager server"
        );

        Server::builder().add_service(service).serve(addr).await
    }
}

/// Run an upcaster service with the given router.
///
/// Supports both TCP and Unix domain socket (UDS) transport.
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::{run_upcaster_server, Router};
///
/// #[tokio::main]
/// async fn main() {
///     let router = Router::new("upcaster-player")
///         .with_handler(|| PlayerUpcaster::new())
///         .build()
///         .unwrap()
///         .into_upcaster()   // or match on Built::Upcaster(r) => r
///         .unwrap();
///
///     run_upcaster_server("upcaster-player", 50401, router).await;
/// }
/// ```
pub async fn run_upcaster_server(
    name: &str,
    default_port: u16,
    router: crate::router::upcaster::UpcasterRouter,
) -> Result<(), tonic::transport::Error> {
    let config = ServerConfig::from_env(default_port);
    let handler = UpcasterGrpc::new(router);
    let service = UpcasterServiceServer::new(handler);

    if let Some(uds_path) = &config.uds_path {
        // UDS mode
        info!(
            name = name,
            path = %uds_path.display(),
            "Starting upcaster server (UDS)"
        );

        let _ = std::fs::remove_file(uds_path);

        let uds = tokio::net::UnixListener::bind(uds_path).expect("Failed to bind UDS socket");
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(uds);

        Server::builder()
            .add_service(service)
            .serve_with_incoming(incoming)
            .await
    } else {
        // TCP mode
        let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse().unwrap();

        info!(name = name, port = config.port, "Starting upcaster server");

        Server::builder().add_service(service).serve(addr).await
    }
}
