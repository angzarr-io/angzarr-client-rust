//! `ComponentHost` and the `run_*_server` runners, beyond the hosting
//! scenarios: prebuilt routers, the default port, bus readiness, serving
//! until a signal, and dropping a running host.
//!
//! These tests set transport environment variables; each holds [`ENV`] for
//! its whole run.

use std::time::Duration;

use angzarr_client::error_codes::codes;
use angzarr_client::proto::{EventBook, ProcessManagerHandleResponse, SagaResponse};
use angzarr_client::router::{
    command_handler, process_manager, projector, saga, upcaster, Built, Router,
};
use angzarr_client::server::{
    run_command_handler_server, run_process_manager_server, run_projector_server, run_saga_server,
    run_server, run_upcaster_server,
};
use angzarr_client::{CommandResult, ComponentHost, HostAddress, ServerConfig};
use tonic_health::pb::health_check_response::ServingStatus;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Tick {
    #[prost(uint32, tag = "1")]
    n: u32,
}

impl ::prost::Name for Tick {
    const NAME: &'static str = "Tick";
    const PACKAGE: &'static str = "host";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Tock {
    #[prost(uint32, tag = "1")]
    n: u32,
}

impl ::prost::Name for Tock {
    const NAME: &'static str = "Tock";
    const PACKAGE: &'static str = "host";
}

#[derive(Default)]
struct S;

struct Ledger;

#[command_handler(domain = "ledger", state = S)]
impl Ledger {
    #[handles(Tick)]
    fn on_tick(&self, _cmd: Tick, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

struct Relay;

#[saga(name = "relay", source = "ledger", target = "audit")]
impl Relay {
    #[handles(Tick)]
    fn on_tick(&self, _evt: Tick) -> CommandResult<SagaResponse> {
        Ok(SagaResponse::default())
    }
}

struct Flow;

#[process_manager(
    name = "flow",
    pm_domain = "flow",
    sources = ["ledger"],
    targets = ["audit"],
    state = S
)]
impl Flow {
    #[handles(Tick)]
    fn on_tick(&self, _evt: Tick, _state: &S) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
}

struct Feed;

#[projector(name = "feed", domains = ["ledger"])]
impl Feed {
    #[handles(Tick)]
    fn on_tick(&self, _evt: Tick) -> CommandResult<()> {
        Ok(())
    }
}

struct Lift;

#[upcaster(name = "lift", domain = "ledger")]
impl Lift {
    #[upcasts(from = Tick, to = Tock)]
    fn upgrade(old: Tick) -> Tock {
        Tock { n: old.n }
    }
}

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
    "ANGZARR_BUS_ENDPOINT",
];

/// Serialises the tests that touch the transport environment.
static ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn clear_env() {
    for var in TRANSPORT_VARS {
        std::env::remove_var(var);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn ephemeral() -> ServerConfig {
    ServerConfig {
        port: 0,
        uds_path: None,
    }
}

fn built<H, F>(factory: F) -> Built
where
    H: angzarr_client::router::Handler + angzarr_client::router::HandlerKind,
    F: Fn() -> H + Send + Sync + 'static,
{
    Router::new("host-test")
        .with_handler(factory)
        .build()
        .expect("router builds")
}

async fn health_at(port: u16, service: &str) -> Option<ServingStatus> {
    let channel = tonic::transport::Endpoint::from_shared(format!("http://127.0.0.1:{port}"))
        .ok()?
        .connect()
        .await
        .ok()?;
    tonic_health::pb::health_client::HealthClient::new(channel)
        .check(tonic_health::pb::HealthCheckRequest {
            service: service.into(),
        })
        .await
        .ok()
        .map(|r| r.into_inner().status())
}

async fn await_status(port: u16, want: ServingStatus, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if health_at(port, "").await == Some(want) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

fn tcp_port(address: &HostAddress) -> u16 {
    match address {
        HostAddress::Tcp(a) => a.port(),
        other => panic!("expected tcp, got {other}"),
    }
}

#[tokio::test]
async fn a_prebuilt_router_is_served() {
    let _env = ENV.lock().await;
    clear_env();
    let running = ComponentHost::new()
        .with_router(built(|| Ledger))
        .with_transport(ephemeral())
        .start()
        .await
        .expect("host starts");
    assert_eq!(
        running.services(),
        &[
            "grpc.health.v1.Health".to_string(),
            "io.angzarr.v1.CommandHandlerService".to_string(),
        ]
    );
    let port = tcp_port(running.address());
    assert!(await_status(port, ServingStatus::Serving, Duration::from_secs(2)).await);
    running.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn two_routers_of_one_kind_are_refused() {
    let _env = ENV.lock().await;
    clear_env();
    let err = ComponentHost::new()
        .with_router(built(|| Ledger))
        .with_router(built(|| Ledger))
        .with_transport(ephemeral())
        .start()
        .await
        .expect_err("duplicate kind");
    assert_eq!(err.code(), codes::HOST_DUPLICATE_KIND);
}

#[tokio::test]
async fn without_a_port_in_the_environment_the_default_port_is_used() {
    let _env = ENV.lock().await;
    clear_env();
    let port = free_port();
    std::env::set_var("ANGZARR_BIND_ADDRESS", format!("127.0.0.1:{port}"));
    let from_env = ComponentHost::new()
        .with_handler(|| Ledger)
        .start()
        .await
        .expect("host starts");
    clear_env();
    assert_eq!(tcp_port(from_env.address()), port);
    from_env.shutdown().await.expect("clean shutdown");

    let default_port = free_port();
    let running = ComponentHost::new()
        .with_handler(|| Ledger)
        .with_default_port(default_port)
        .start()
        .await
        .expect("host starts");
    assert_eq!(tcp_port(running.address()), default_port);
    running.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn an_unreachable_bus_holds_an_async_saga_host_not_serving() {
    let _env = ENV.lock().await;
    clear_env();
    let dead = free_port();
    std::env::set_var("ANGZARR_BUS_ENDPOINT", format!("127.0.0.1:{dead}"));
    let saga = ComponentHost::new()
        .with_handler(|| Relay)
        .with_handler(|| Ledger)
        .with_transport(ephemeral())
        .start()
        .await
        .expect("host starts");
    let port = tcp_port(saga.address());
    assert!(
        !await_status(port, ServingStatus::Serving, Duration::from_millis(500)).await,
        "the bus probe holds readiness"
    );
    assert_eq!(health_at(port, "").await, Some(ServingStatus::NotServing));
    saga.shutdown().await.expect("clean shutdown");

    let plain = ComponentHost::new()
        .with_handler(|| Ledger)
        .with_transport(ephemeral())
        .start()
        .await
        .expect("host starts");
    let port = tcp_port(plain.address());
    assert!(
        await_status(port, ServingStatus::Serving, Duration::from_secs(2)).await,
        "no async outputs, no bus probe"
    );
    plain.shutdown().await.expect("clean shutdown");
    clear_env();
}

#[tokio::test]
async fn a_dropped_host_stops_accepting() {
    let _env = ENV.lock().await;
    clear_env();
    let running = ComponentHost::new()
        .with_handler(|| Ledger)
        .with_transport(ephemeral())
        .start()
        .await
        .expect("host starts");
    let port = tcp_port(running.address());
    assert!(await_status(port, ServingStatus::Serving, Duration::from_secs(2)).await);
    drop(running);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a dropped host still accepts connections"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_dropped_host_stops_probing_readiness() {
    let _env = ENV.lock().await;
    clear_env();
    let bus = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::env::set_var(
        "ANGZARR_BUS_ENDPOINT",
        bus.local_addr().unwrap().to_string(),
    );
    std::env::set_var("ANGZARR_READINESS_PROBE_INTERVAL", "1");
    let running = ComponentHost::new()
        .with_handler(|| Relay)
        .with_transport(ephemeral())
        .start()
        .await
        .expect("host starts");
    clear_env();
    std::env::remove_var("ANGZARR_READINESS_PROBE_INTERVAL");
    // The supervisor probes the bus once per second while the host runs.
    tokio::time::timeout(Duration::from_secs(3), bus.accept())
        .await
        .expect("the running host probes the bus")
        .unwrap();
    drop(running);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let probed = tokio::time::timeout(Duration::from_millis(2500), bus.accept()).await;
    assert!(probed.is_err(), "a dropped host still probes readiness");
}

#[test]
fn host_and_address_describe_themselves() {
    let host = ComponentHost::new()
        .with_handler(|| Ledger)
        .with_default_port(4242)
        .with_drain_period(Duration::from_millis(5));
    let debug = format!("{host:?}");
    assert!(debug.starts_with("ComponentHost {"), "{debug}");
    assert!(debug.contains("CommandHandler"), "{debug}");
    assert!(debug.contains("default_port: 4242"), "{debug}");
    assert!(debug.contains("drain_period: 5ms"), "{debug}");

    assert_eq!(
        HostAddress::Tcp("127.0.0.1:7".parse().unwrap()).to_string(),
        "127.0.0.1:7"
    );
    assert_eq!(
        HostAddress::Uds("/tmp/x.sock".into()).to_string(),
        "unix:/tmp/x.sock"
    );
}

/// Each runner keeps serving until a signal: still running, and reporting
/// SERVING, well after it starts.
async fn assert_serves_until_signal<F>(serve: F)
where
    F: std::future::Future<Output = angzarr_client::Result<()>> + Send + 'static,
{
    let port = free_port();
    std::env::set_var("ANGZARR_BIND_ADDRESS", format!("127.0.0.1:{port}"));
    let task = tokio::spawn(serve);
    let serving = await_status(port, ServingStatus::Serving, Duration::from_secs(2)).await;
    clear_env();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let finished = task.is_finished();
    task.abort();
    let _ = task.await;
    assert!(serving, "runner never reported SERVING");
    assert!(!finished, "runner returned without a signal");
}

#[tokio::test]
async fn the_runners_serve_until_a_signal() {
    let _env = ENV.lock().await;
    clear_env();
    assert_serves_until_signal(run_server(0, built(|| Ledger))).await;
    let Built::CommandHandler(r) = built(|| Ledger) else {
        unreachable!()
    };
    assert_serves_until_signal(run_command_handler_server(r, 0)).await;
    let Built::Saga(r) = built(|| Relay) else {
        unreachable!()
    };
    assert_serves_until_signal(run_saga_server(r, 0)).await;
    let Built::ProcessManager(r) = built(|| Flow) else {
        unreachable!()
    };
    assert_serves_until_signal(run_process_manager_server(r, 0)).await;
    let Built::Projector(r) = built(|| Feed) else {
        unreachable!()
    };
    assert_serves_until_signal(run_projector_server(r, 0)).await;
    let Built::Upcaster(r) = built(|| Lift) else {
        unreachable!()
    };
    assert_serves_until_signal(run_upcaster_server(r, 0)).await;
    assert_serves_until_signal(ComponentHost::new().with_handler(|| Ledger).serve()).await;
}
