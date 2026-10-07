//! Step definitions for parity/client/hosting.feature.
//!
//! Every scenario runs a real [`ComponentHost`] and talks to it over gRPC.
//! Scenarios hold the process-wide environment lock for their whole run,
//! since the host reads its transport from the environment.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use angzarr_client::proto::command_handler_service_server::CommandHandlerServiceServer;
use angzarr_client::{ClientError, ComponentHost, HostAddress, RunningHost, ServerConfig};
use cucumber::{given, then, when, World};
use tokio::sync::OwnedMutexGuard;
use tokio::task::JoinHandle;
use tonic::server::NamedService;
use tonic_health::pb::health_check_response::ServingStatus;

use crate::common::backend::env_lock;
use crate::common::host_fixtures::{
    await_health, connect, echo, handled_by, health, send_record, Gate, OrderComponent,
    OrderReportService, PaymentComponent, ORDER_REPORT_SERVICE,
};

const COMMAND_HANDLER_SERVICE: &str = <CommandHandlerServiceServer<
    angzarr_client::handler::CommandHandlerGrpc,
> as NamedService>::NAME;

/// Transport variables the host reads.
const TRANSPORT_VARS: &[&str] = &[
    "TRANSPORT_TYPE",
    "UDS_BASE_PATH",
    "SERVICE_NAME",
    "DOMAIN",
    "SAGA_NAME",
    "PROJECTOR_NAME",
    "PORT",
    "GRPC_PORT",
    "ANGZARR_BIND_ADDRESS",
];

/// Drain period of a started host, long enough to observe NOT_SERVING.
const DRAIN: Duration = Duration::from_millis(300);

#[derive(Default, World)]
#[world(init = Self::default)]
pub struct HostingWorld {
    env: Option<OwnedMutexGuard<()>>,
    transport_from_env: bool,
    gate: Arc<Gate>,
    host: Option<ComponentHost>,
    running: Option<RunningHost>,
    draining: Option<HostAddress>,
    start_error: Option<ClientError>,
    in_flight: Option<JoinHandle<Result<String, tonic::Status>>>,
    shutting_down: Option<JoinHandle<angzarr_client::Result<()>>>,
}

impl std::fmt::Debug for HostingWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostingWorld")
            .field("transport_from_env", &self.transport_from_env)
            .field("running", &self.running)
            .field("start_error", &self.start_error)
            .finish()
    }
}

impl HostingWorld {
    /// Take the environment lock and clear the transport variables.
    async fn lock_env(&mut self) {
        if self.env.is_none() {
            self.env = Some(env_lock().lock_owned().await);
            for var in TRANSPORT_VARS {
                std::env::remove_var(var);
            }
        }
    }

    fn running(&self) -> &RunningHost {
        self.running
            .as_ref()
            .unwrap_or_else(|| panic!("host is not running; start error {:?}", self.start_error))
    }

    async fn start(&mut self, transport: Option<ServerConfig>) {
        let mut host = self.host.take().expect("a host was configured");
        if let Some(config) = transport {
            host = host.with_transport(config);
        }
        match host.start().await {
            Ok(running) => self.running = Some(running),
            Err(e) => self.start_error = Some(e),
        }
    }

    /// Start on an ephemeral TCP port unless the environment selects the
    /// transport.
    async fn start_default(&mut self) {
        let transport = (!self.transport_from_env).then_some(ServerConfig {
            port: 0,
            uds_path: None,
        });
        self.start(transport).await;
    }
}

impl Drop for HostingWorld {
    fn drop(&mut self) {
        if self.env.is_some() {
            for var in TRANSPORT_VARS {
                std::env::remove_var(var);
            }
        }
    }
}

// --- Given -----------------------------------------------------------------

#[given(expr = "a component host with an aggregate component for domain {string} registered")]
async fn given_host_one(world: &mut HostingWorld, domain: String) {
    world.lock_env().await;
    assert_eq!(domain, "order");
    let gate = Arc::clone(&world.gate);
    world.host = Some(
        ComponentHost::new()
            .with_handler(move || OrderComponent(Arc::clone(&gate)))
            .with_drain_period(DRAIN),
    );
}

#[given(
    expr = "a component host with aggregate components for domains {string} and {string} registered"
)]
async fn given_host_two(world: &mut HostingWorld, first: String, second: String) {
    world.lock_env().await;
    assert_eq!((first.as_str(), second.as_str()), ("order", "payment"));
    let order_gate = Arc::clone(&world.gate);
    let payment_gate = Arc::clone(&world.gate);
    world.host = Some(
        ComponentHost::new()
            .with_handler(move || OrderComponent(Arc::clone(&order_gate)))
            .with_handler(move || PaymentComponent(Arc::clone(&payment_gate))),
    );
}

#[given("a component host with no components registered")]
async fn given_host_empty(world: &mut HostingWorld) {
    world.lock_env().await;
    world.host = Some(ComponentHost::new());
}

#[given(
    expr = "a started component host with an aggregate component for domain {string} registered"
)]
async fn given_started_host(world: &mut HostingWorld, domain: String) {
    given_host_one(world, domain).await;
    world.start_default().await;
    let channel = connect(world.running().address()).await;
    assert!(
        await_health(channel, "", ServingStatus::Serving).await,
        "started host never reported SERVING"
    );
}

#[given(regex = r#"^the transport environment selects (TCP|Unix socket) at "([^"]*)"$"#)]
async fn given_transport_env(world: &mut HostingWorld, transport: String, address: String) {
    world.lock_env().await;
    world.transport_from_env = true;
    match transport.as_str() {
        "TCP" => {
            std::env::set_var("TRANSPORT_TYPE", "tcp");
            std::env::set_var("ANGZARR_BIND_ADDRESS", &address);
        }
        _ => {
            let path = PathBuf::from(&address);
            let base = path.parent().expect("socket directory");
            let service = path
                .file_stem()
                .expect("socket file name")
                .to_string_lossy()
                .into_owned();
            std::env::set_var("TRANSPORT_TYPE", "uds");
            std::env::set_var("UDS_BASE_PATH", base);
            std::env::set_var("SERVICE_NAME", service);
        }
    }
}

#[given(expr = "an application-defined gRPC service {word} registered on the host")]
async fn given_app_service(world: &mut HostingWorld, name: String) {
    assert_eq!(name, "OrderReportService");
    let host = world.host.take().expect("a host was configured");
    world.host = Some(host.with_service(OrderReportService));
}

// --- When ------------------------------------------------------------------

#[when("the host starts on a TCP port")]
async fn when_starts_tcp(world: &mut HostingWorld) {
    world
        .start(Some(ServerConfig {
            port: 0,
            uds_path: None,
        }))
        .await;
}

#[when("the host starts")]
async fn when_starts(world: &mut HostingWorld) {
    world.start(None).await;
}

#[when("shutdown begins")]
async fn when_shutdown_begins(world: &mut HostingWorld) {
    let channel = connect(world.running().address()).await;
    world.in_flight = Some(tokio::spawn(async move {
        send_record(channel, "order", true)
            .await
            .map(|r| handled_by(&r))
    }));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while world.gate.held() == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the held call never reached the component"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let running = world.running.take().expect("running host");
    world.draining = Some(running.address().clone());
    world.shutting_down = Some(tokio::spawn(running.shutdown()));
}

#[when("the host shuts down")]
async fn when_shuts_down(world: &mut HostingWorld) {
    let running = world.running.take().expect("running host");
    running.shutdown().await.expect("clean shutdown");
}

// --- Then ------------------------------------------------------------------

#[then("CommandHandlerService is served on that port")]
async fn then_ch_served(world: &mut HostingWorld) {
    let running = world.running();
    assert!(matches!(running.address(), HostAddress::Tcp(a) if a.port() != 0));
    assert!(running
        .services()
        .contains(&COMMAND_HANDLER_SERVICE.to_string()));
    let channel = connect(running.address()).await;
    assert!(health(channel, COMMAND_HANDLER_SERVICE).await.is_ok());
}

#[then(
    expr = "a ContextualCommand for domain {string} sent to CommandHandlerService.Handle reaches the order component"
)]
async fn then_reaches_order_via_handle(world: &mut HostingWorld, domain: String) {
    let channel = connect(world.running().address()).await;
    let response = send_record(channel, &domain, false)
        .await
        .expect("Handle succeeds");
    assert_eq!(handled_by(&response), "order");
}

#[then(expr = "a ContextualCommand for domain {string} reaches the {word} component")]
async fn then_reaches(world: &mut HostingWorld, domain: String, component: String) {
    let channel = connect(world.running().address()).await;
    let response = send_record(channel, &domain, false)
        .await
        .expect("Handle succeeds");
    assert_eq!(handled_by(&response), component);
}

#[then(regex = r#"^the host listens on (TCP|Unix socket) at "([^"]*)"$"#)]
async fn then_listens(world: &mut HostingWorld, transport: String, address: String) {
    let running = world.running();
    match (transport.as_str(), running.address()) {
        ("TCP", HostAddress::Tcp(bound)) => {
            let requested: std::net::SocketAddr = address.parse().expect("socket address");
            assert_eq!(bound.ip(), requested.ip());
            if requested.port() == 0 {
                assert_ne!(bound.port(), 0, "an ephemeral port is assigned");
            } else {
                assert_eq!(bound.port(), requested.port());
            }
        }
        ("Unix socket", HostAddress::Uds(path)) => {
            assert_eq!(path, &PathBuf::from(&address));
            assert!(path.exists(), "socket file exists while serving");
        }
        (want, got) => panic!("expected {want} at {address}, host listens on {got}"),
    }
    let channel = connect(running.address()).await;
    assert!(await_health(channel, "", ServingStatus::Serving).await);
}

#[then(
    "the overall server's health status is SERVING only after every registered component's service is listening"
)]
async fn then_serving_after_listening(world: &mut HostingWorld) {
    let channel = connect(world.running().address()).await;
    let first = health(channel.clone(), "").await;
    if first != Ok(ServingStatus::Serving) {
        assert_eq!(first, Ok(ServingStatus::NotServing), "before readiness");
        assert!(await_health(channel.clone(), "", ServingStatus::Serving).await);
    }
    // Once the overall status is SERVING, the component answers.
    let response = send_record(channel, "order", false)
        .await
        .expect("component listening once SERVING");
    assert_eq!(handled_by(&response), "order");
}

#[then(expr = "the gRPC health service reports SERVING for {word}")]
async fn then_health_serving_for(world: &mut HostingWorld, service: String) {
    let name = match service.as_str() {
        "CommandHandlerService" => COMMAND_HANDLER_SERVICE,
        "OrderReportService" => ORDER_REPORT_SERVICE,
        other => panic!("unknown service {other}"),
    };
    let channel = connect(world.running().address()).await;
    assert!(
        await_health(channel, name, ServingStatus::Serving).await,
        "{name} never reported SERVING"
    );
}

#[then("the gRPC health service reports NOT_SERVING for the overall server")]
async fn then_not_serving(world: &mut HostingWorld) {
    let address = world.draining.clone().expect("host is shutting down");
    let channel = connect(&address).await;
    assert!(
        await_health(channel, "", ServingStatus::NotServing).await,
        "draining host still reports SERVING"
    );
    assert!(
        !world
            .in_flight
            .as_ref()
            .expect("in-flight call")
            .is_finished(),
        "the in-flight call is still held"
    );
}

#[then("in-flight calls complete before the server stops")]
async fn then_in_flight_complete(world: &mut HostingWorld) {
    tokio::time::sleep(DRAIN * 2).await;
    let shutting_down = world.shutting_down.take().expect("shutdown started");
    assert!(
        !shutting_down.is_finished(),
        "the server stopped while a call was in flight"
    );
    world.gate.open();
    let handled = world
        .in_flight
        .take()
        .expect("in-flight call")
        .await
        .expect("call task")
        .expect("in-flight call completes");
    assert_eq!(handled, "order");
    tokio::time::timeout(Duration::from_secs(5), shutting_down)
        .await
        .expect("server stops once the call completes")
        .expect("shutdown task")
        .expect("clean shutdown");
}

#[then(expr = "{string} no longer exists")]
async fn then_socket_removed(_world: &mut HostingWorld, path: String) {
    assert!(!PathBuf::from(&path).exists(), "{path} still exists");
}

#[then("OrderReportService is served on that port alongside CommandHandlerService")]
async fn then_app_service_served(world: &mut HostingWorld) {
    let running = world.running();
    let services = running.services();
    assert!(services.contains(&ORDER_REPORT_SERVICE.to_string()));
    assert!(services.contains(&COMMAND_HANDLER_SERVICE.to_string()));
    let channel = connect(running.address()).await;
    let echoed = echo(channel.clone(), "order").await.expect("Echo succeeds");
    assert_eq!(echoed.domain, "report:order");
    let response = send_record(channel, "order", false)
        .await
        .expect("Handle succeeds");
    assert_eq!(handled_by(&response), "order");
}

#[then("starting fails with a configuration error")]
async fn then_start_fails(world: &mut HostingWorld) {
    assert!(world.running.is_none(), "host started");
    let err = world.start_error.as_ref().expect("start error");
    assert!(err.is_invalid_argument(), "got {err:?}");
    assert_eq!(
        err.code(),
        angzarr_client::error_codes::codes::HOST_HAS_NO_COMPONENTS
    );
}
