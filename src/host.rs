//! Generic gRPC component host.
//!
//! A [`ComponentHost`] serves an application's components — handler types
//! declared with the kind macros, or routers already built from them — next
//! to the gRPC health service, plus any gRPC services the application
//! registers itself. It knows framework concepts only: component kinds,
//! transport, readiness and shutdown.
//!
//! Lifecycle:
//!
//! 1. [`ComponentHost::start`] builds one router per component kind, binds
//!    the listener (TCP, or a Unix socket, from [`get_transport_config`]
//!    unless [`ComponentHost::with_transport`] says otherwise) and serves.
//! 2. Every served service name, and the overall `""` name, reports
//!    `NOT_SERVING` until the readiness supervisor sees the listener bound
//!    and every sync output domain reachable; then `SERVING`.
//! 3. [`RunningHost::shutdown`] flips every name to `NOT_SERVING`, waits
//!    the drain period so load balancers stop routing to the host, then
//!    stops accepting and lets in-flight calls finish. A Unix socket file is
//!    removed afterwards.
//!
//! [`ComponentHost::serve`] runs that lifecycle until SIGINT / SIGTERM.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tonic::codegen::{http, Service};
use tonic::server::NamedService;
use tonic::service::RoutesBuilder;
use tonic::transport::Server;
use tonic_health::server::HealthReporter;
use tonic_health::ServingStatus;
use tracing::info;

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
    probe_config_from_env, run_supervisor_with_wake, BusProbe, OutputDomainProbe, Probe,
    TransportProbe,
};
use crate::router::{Built, Handler, HandlerKind, Kind, Router};
use crate::server::{
    bind_tcp_listener, bind_uds_listener, cleanup_socket, ensure_uds_parent_dir,
    get_transport_config, parse_bind_address, resolve_bind_address, shutdown_signal, ServerConfig,
};

/// Default TCP port when neither the environment nor
/// [`ComponentHost::with_default_port`] names one.
pub const DEFAULT_HOST_PORT: u16 = 50051;

/// gRPC service name of the health service the host always serves.
pub const HEALTH_SERVICE_NAME: &str = "grpc.health.v1.Health";

type AddService = Box<dyn FnOnce(&mut RoutesBuilder) + Send>;

/// A gRPC service the application registers on the host.
struct AppService {
    name: &'static str,
    add: AddService,
}

/// Builder for a host serving components and application services.
pub struct ComponentHost {
    builders: Vec<(Kind, Router)>,
    routers: Vec<Built>,
    services: Vec<AppService>,
    transport: Option<ServerConfig>,
    default_port: u16,
    drain_period: Duration,
}

impl Default for ComponentHost {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ComponentHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentHost")
            .field(
                "kinds",
                &self.builders.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            )
            .field("routers", &self.routers.len())
            .field(
                "services",
                &self.services.iter().map(|s| s.name).collect::<Vec<_>>(),
            )
            .field("transport", &self.transport)
            .field("default_port", &self.default_port)
            .field("drain_period", &self.drain_period)
            .finish()
    }
}

impl ComponentHost {
    /// An empty host: no components, transport from the environment, no
    /// drain period.
    pub fn new() -> Self {
        Self {
            builders: Vec::new(),
            routers: Vec::new(),
            services: Vec::new(),
            transport: None,
            default_port: DEFAULT_HOST_PORT,
            drain_period: Duration::ZERO,
        }
    }

    /// Register a component: a handler type declared with a kind macro.
    /// Components of one kind are served by one router, which routes among
    /// them (e.g. aggregates by domain).
    pub fn with_handler<H, F>(mut self, factory: F) -> Self
    where
        H: Handler + HandlerKind,
        F: Fn() -> H + Send + Sync + 'static,
    {
        let at = self.builders.iter().position(|(k, _)| *k == H::KIND);
        let router = match at {
            Some(i) => self.builders.remove(i).1,
            None => Router::new(H::KIND.as_str()),
        };
        self.builders.push((H::KIND, router.with_handler(factory)));
        self
    }

    /// Register an already-built router. A host serves one router per
    /// component kind.
    pub fn with_router(mut self, built: Built) -> Self {
        self.routers.push(built);
        self
    }

    /// Register an application-defined gRPC service, served next to the
    /// components and reported by the health service under its name.
    pub fn with_service<S>(mut self, service: S) -> Self
    where
        S: Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<tonic::body::Body>,
                Error = Infallible,
            > + NamedService
            + Clone
            + Send
            + Sync
            + 'static,
        S::Future: Send + 'static,
    {
        self.services.push(AppService {
            name: S::NAME,
            add: Box::new(move |routes: &mut RoutesBuilder| {
                routes.add_service(service);
            }),
        });
        self
    }

    /// Serve on this transport instead of the one the environment selects.
    pub fn with_transport(mut self, config: ServerConfig) -> Self {
        self.transport = Some(config);
        self
    }

    /// TCP port used when the environment names none.
    pub fn with_default_port(mut self, port: u16) -> Self {
        self.default_port = port;
        self
    }

    /// Time between reporting `NOT_SERVING` and closing the listener on
    /// shutdown, so callers routed by health checks move away first.
    pub fn with_drain_period(mut self, period: Duration) -> Self {
        self.drain_period = period;
        self
    }

    /// Build the routers, bind the listener and start serving in the
    /// background. Fails with a configuration error when nothing is
    /// registered, a router does not build, or two routers share a kind.
    pub async fn start(self) -> Result<RunningHost> {
        let ComponentHost {
            builders,
            mut routers,
            services,
            transport,
            default_port,
            drain_period,
        } = self;
        if builders.is_empty() && routers.is_empty() {
            return Err(ClientError::invalid_argument(
                codes::HOST_HAS_NO_COMPONENTS,
                messages::HOST_HAS_NO_COMPONENTS,
                std::iter::empty::<(String, String)>(),
            ));
        }
        for (_, router) in builders {
            routers.push(router.build().map_err(|e| {
                ClientError::invalid_argument(e.code(), e.message(), e.details().clone())
            })?);
        }
        let mut kinds: Vec<Kind> = Vec::new();
        for built in &routers {
            let kind = built_kind(built);
            if kinds.contains(&kind) {
                return Err(ClientError::invalid_argument(
                    codes::HOST_DUPLICATE_KIND,
                    messages::HOST_DUPLICATE_KIND,
                    [(keys::HANDLER_KIND, kind.as_str())],
                ));
            }
            kinds.push(kind);
        }

        let mut hosted = Vec::new();
        for built in routers {
            hosted.push(Hosted::from(built));
        }
        let names: Vec<String> = hosted
            .iter()
            .map(|h| h.name.to_string())
            .chain(services.iter().map(|s| s.name.to_string()))
            .collect();
        let instance = hosted
            .iter()
            .map(|h| h.instance.as_str())
            .collect::<Vec<_>>()
            .join(",");

        let config = transport.unwrap_or_else(|| get_transport_config(default_port));
        let (listener, address) = bind(&config).await?;
        info!(
            name = %instance,
            address = %address,
            services = ?names,
            "server_started",
        );

        let (reporter, health_service) = tonic_health::server::health_reporter();
        let mut health_names: Vec<String> = vec![String::new()];
        health_names.extend(names.iter().cloned());
        for name in &health_names {
            reporter
                .set_service_status(name, ServingStatus::NotServing)
                .await;
        }

        let (transport_probe, transport_signal) = TransportProbe::new();
        let wake = transport_probe.wake();
        let mut probes: Vec<Box<dyn Probe>> = vec![Box::new(transport_probe)];
        let mut any_async = false;
        for h in &hosted {
            for domain in &h.sync_outputs {
                probes.push(Box::new(OutputDomainProbe::for_domain(domain.clone())?));
            }
            any_async |= h.has_async_outputs;
        }
        if any_async {
            if let Some(bus) = BusProbe::from_env() {
                probes.push(Box::new(bus));
            }
        }

        let mut routes = RoutesBuilder::default();
        routes.add_service(health_service);
        for h in hosted {
            (h.add)(&mut routes);
        }
        for s in services {
            (s.add)(&mut routes);
        }
        let mut served = vec![HEALTH_SERVICE_NAME.to_string()];
        served.extend(names);

        let (interval, timeout) = probe_config_from_env();
        let supervisor = tokio::spawn(run_supervisor_with_wake(
            probes,
            reporter.clone(),
            health_names.clone(),
            interval,
            timeout,
            wake,
        ));

        let (stop, stopped) = oneshot::channel::<()>();
        let router = Server::builder().add_routes(routes.routes());
        let signal = async {
            let _ = stopped.await;
        };
        let server = match listener {
            Listener::Tcp(l) => tokio::spawn(router.serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(l),
                signal,
            )),
            Listener::Uds(l) => tokio::spawn(router.serve_with_incoming_shutdown(
                tokio_stream::wrappers::UnixListenerStream::new(l),
                signal,
            )),
        };
        transport_signal.mark_bound();

        Ok(RunningHost {
            address,
            instance,
            served,
            health_names,
            reporter,
            supervisor: Some(supervisor),
            server: Some(server),
            stop: Some(stop),
            drain_period,
        })
    }

    /// Start, then serve until SIGINT / SIGTERM and shut down gracefully.
    pub async fn serve(self) -> Result<()> {
        self.start().await?.serve_until_signal().await
    }
}

/// Where a running host listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAddress {
    /// A TCP socket address (the bound port when port 0 was requested).
    Tcp(SocketAddr),
    /// A Unix domain socket path.
    Uds(PathBuf),
}

impl std::fmt::Display for HostAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostAddress::Tcp(a) => write!(f, "{a}"),
            HostAddress::Uds(p) => write!(f, "unix:{}", p.display()),
        }
    }
}

type ServerTask = JoinHandle<std::result::Result<(), tonic::transport::Error>>;

/// A started [`ComponentHost`]. Dropping it without
/// [`shutdown`](RunningHost::shutdown) stops accepting and lets in-flight
/// calls finish in the background, without the drain period or socket
/// cleanup.
#[derive(Debug)]
pub struct RunningHost {
    address: HostAddress,
    instance: String,
    served: Vec<String>,
    health_names: Vec<String>,
    reporter: HealthReporter,
    supervisor: Option<JoinHandle<()>>,
    server: Option<ServerTask>,
    stop: Option<oneshot::Sender<()>>,
    drain_period: Duration,
}

impl Drop for RunningHost {
    fn drop(&mut self) {
        if let Some(supervisor) = self.supervisor.take() {
            supervisor.abort();
        }
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl RunningHost {
    /// Where the host listens.
    pub fn address(&self) -> &HostAddress {
        &self.address
    }

    /// Fully-qualified names of the gRPC services served: the health
    /// service, one framework service per component kind, then the
    /// application's services.
    pub fn services(&self) -> &[String] {
        &self.served
    }

    /// Report `NOT_SERVING`, wait the drain period, stop accepting, let
    /// in-flight calls finish, and remove a Unix socket file.
    pub async fn shutdown(mut self) -> Result<()> {
        if let Some(supervisor) = self.supervisor.take() {
            supervisor.abort();
            let _ = supervisor.await;
        }
        publish_shutdown_status(&self.reporter, &self.health_names).await;
        tokio::time::sleep(self.drain_period).await;
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let ended = match self.server.take() {
            Some(server) => server.await,
            None => Ok(Ok(())),
        };
        self.finish(ended)
    }

    /// Serve until SIGINT / SIGTERM, then [`shutdown`](Self::shutdown). A
    /// server that stops on its own ends the wait with its result.
    pub async fn serve_until_signal(mut self) -> Result<()> {
        let Some(mut server) = self.server.take() else {
            return self.shutdown().await;
        };
        tokio::select! {
            _ = shutdown_signal() => {
                self.server = Some(server);
                self.shutdown().await
            }
            ended = &mut server => {
                if let Some(supervisor) = self.supervisor.take() {
                    supervisor.abort();
                }
                publish_shutdown_status(&self.reporter, &self.health_names).await;
                self.finish(ended)
            }
        }
    }

    /// Remove a Unix socket file, log the shutdown and report how the
    /// server task ended.
    fn finish(
        &self,
        ended: std::result::Result<
            std::result::Result<(), tonic::transport::Error>,
            tokio::task::JoinError,
        >,
    ) -> Result<()> {
        if let HostAddress::Uds(path) = &self.address {
            cleanup_socket(path);
        }
        info!(name = %self.instance, address = %self.address, "server_shutdown");
        match ended {
            Ok(served) => served.map_err(ClientError::from),
            Err(join) => Err(server_task_error(join)),
        }
    }
}

fn server_task_error(join: tokio::task::JoinError) -> ClientError {
    ClientError::invalid_argument(
        codes::HOST_SERVER_TASK_FAILED,
        messages::HOST_SERVER_TASK_FAILED,
        [(keys::CAUSE, join.to_string())],
    )
}

/// Flip every health name to `NOT_SERVING` so load balancers drain the
/// host.
pub(crate) async fn publish_shutdown_status(reporter: &HealthReporter, names: &[String]) {
    for name in names {
        reporter
            .set_service_status(name, ServingStatus::NotServing)
            .await;
    }
}

fn built_kind(built: &Built) -> Kind {
    match built {
        Built::CommandHandler(_) => Kind::CommandHandler,
        Built::Saga(_) => Kind::Saga,
        Built::ProcessManager(_) => Kind::ProcessManager,
        Built::Projector(_) => Kind::Projector,
        Built::Upcaster(_) => Kind::Upcaster,
    }
}

/// One component kind's framework service, ready to add to the routes.
struct Hosted {
    name: &'static str,
    instance: String,
    sync_outputs: Vec<String>,
    has_async_outputs: bool,
    add: AddService,
}

impl From<Built> for Hosted {
    fn from(built: Built) -> Self {
        match built {
            Built::CommandHandler(r) => Hosted {
                name: <CommandHandlerServiceServer<CommandHandlerGrpc> as NamedService>::NAME,
                instance: r.name(),
                sync_outputs: Vec::new(),
                has_async_outputs: false,
                add: Box::new(move |routes| {
                    routes
                        .add_service(CommandHandlerServiceServer::new(CommandHandlerGrpc::new(r)));
                }),
            },
            Built::Saga(r) => Hosted {
                name: <SagaServiceServer<SagaGrpc> as NamedService>::NAME,
                instance: r.name(),
                sync_outputs: r.sync_output_domains(),
                has_async_outputs: r.has_async_outputs(),
                add: Box::new(move |routes| {
                    routes.add_service(SagaServiceServer::new(SagaGrpc::new(r)));
                }),
            },
            Built::ProcessManager(r) => Hosted {
                name: <ProcessManagerServiceServer<ProcessManagerGrpc> as NamedService>::NAME,
                instance: r.name(),
                sync_outputs: r.sync_output_domains(),
                has_async_outputs: r.has_async_outputs(),
                add: Box::new(move |routes| {
                    routes
                        .add_service(ProcessManagerServiceServer::new(ProcessManagerGrpc::new(r)));
                }),
            },
            Built::Projector(r) => Hosted {
                name: <ProjectorServiceServer<ProjectorGrpc> as NamedService>::NAME,
                instance: r.name(),
                sync_outputs: Vec::new(),
                has_async_outputs: false,
                add: Box::new(move |routes| {
                    routes.add_service(ProjectorServiceServer::new(ProjectorGrpc::new(r)));
                }),
            },
            Built::Upcaster(r) => Hosted {
                name: <UpcasterServiceServer<UpcasterGrpc> as NamedService>::NAME,
                instance: r.name(),
                sync_outputs: Vec::new(),
                has_async_outputs: false,
                add: Box::new(move |routes| {
                    routes.add_service(UpcasterServiceServer::new(UpcasterGrpc::new(r)));
                }),
            },
        }
    }
}

enum Listener {
    Tcp(tokio::net::TcpListener),
    Uds(tokio::net::UnixListener),
}

/// Bind the configured transport. A Unix socket's parent directory is
/// created and a stale socket file removed first.
async fn bind(config: &ServerConfig) -> Result<(Listener, HostAddress)> {
    match config.uds_path.as_ref() {
        Some(path) => {
            ensure_uds_parent_dir(path)?;
            cleanup_socket(path);
            let listener = bind_uds_listener(path)?;
            Ok((Listener::Uds(listener), HostAddress::Uds(path.clone())))
        }
        None => {
            let addr = parse_bind_address(&resolve_bind_address(config.port))?;
            let listener = bind_tcp_listener(addr).await?;
            let bound = listener.local_addr().unwrap_or(addr);
            Ok((Listener::Tcp(listener), HostAddress::Tcp(bound)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn publish_shutdown_status_no_names_is_noop() {
        let (reporter, _service) = tonic_health::server::health_reporter();
        // Should not panic, should not register a name we never asked
        // for. Empty input is the legal "no services" case (won't
        // happen in a host, which always has the overall name).
        publish_shutdown_status(&reporter, &[]).await;
    }
}
