//! Step definitions for `parity/client/connection.feature`.
//!
//! Every connection is a real tonic connection made by the library's client
//! constructors. Scenario endpoints name well-known places; the world maps
//! them onto the in-process test backend (`tests/common/backend.rs`):
//!
//! - port `1310` (the coordinator default) is the scenario's TCP backend;
//! - port `59999` is a port with nothing listening;
//! - a Unix socket path named by "a Unix socket at …" is a backend bound to
//!   a private temporary path standing in for it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use angzarr_client::error_codes::{codes, keys};
use angzarr_client::proto::{EventBook, SpeculateProjectorRequest};
use angzarr_client::traits::SpeculativeClient as SpeculativeOps;
use angzarr_client::{
    ClientError, CommandBuilderExt, CommandHandlerClient, DomainClient, QueryBuilderExt,
    QueryClient, SpeculativeClient,
};
use cucumber::{given, then, when, World};
use tonic::transport::Channel;
use uuid::Uuid;

use crate::common::backend::{
    env_lock, single_attempt, unavailable_endpoint, GenericCommand, Hidden, TestBackend,
    TYPE_PREFIX,
};

#[derive(Debug, Default, World)]
pub struct ConnectionWorld {
    backend: Option<TestBackend>,
    closed_port: Option<String>,
    sockets: HashMap<String, PathBuf>,
    query: Hidden<QueryClient>,
    command: Hidden<CommandHandlerClient>,
    speculative: Hidden<SpeculativeClient>,
    domain: Hidden<DomainClient>,
    channel: Hidden<Channel>,
    error: Option<ClientError>,
    env_guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    env_vars: Vec<String>,
    operation: Option<Result<EventBook, ClientError>>,
    first_port: Option<u16>,
}

impl Drop for ConnectionWorld {
    fn drop(&mut self) {
        for name in &self.env_vars {
            std::env::remove_var(name);
        }
        for path in self.sockets.values() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

impl ConnectionWorld {
    async fn backend(&mut self) -> &TestBackend {
        if self.backend.is_none() {
            self.backend = Some(TestBackend::start_tcp().await);
        }
        self.backend.as_ref().expect("backend")
    }

    /// Map a scenario endpoint onto the test environment.
    async fn resolve(&mut self, endpoint: &str) -> String {
        for (scenario_path, real) in &self.sockets {
            if endpoint.ends_with(scenario_path.as_str()) {
                let prefix = &endpoint[..endpoint.len() - scenario_path.len()];
                return format!("{prefix}{}", real.display());
            }
        }
        if endpoint.contains(":1310") {
            let port = self.backend().await.port().expect("tcp backend");
            return endpoint.replace(":1310", &format!(":{port}"));
        }
        if endpoint.contains(":59999") {
            if self.closed_port.is_none() {
                let ep = unavailable_endpoint().await;
                self.closed_port = Some(ep.rsplit(':').next().expect("port").to_string());
            }
            let port = self.closed_port.clone().expect("closed port");
            return endpoint.replace(":59999", &format!(":{port}"));
        }
        endpoint.to_string()
    }

    async fn lock_env(&mut self) {
        if self.env_guard.is_none() {
            self.env_guard = Some(env_lock().lock_owned().await);
        }
    }

    fn connected_query(&self) -> &QueryClient {
        if let Some(e) = &self.error {
            panic!("connection failed: {e:?}");
        }
        self.query.get()
    }

    fn err(&self) -> &ClientError {
        self.error.as_ref().expect("connection failed")
    }

    fn cause(&self) -> String {
        match self.err() {
            ClientError::Connection(d) => d.details.get(keys::CAUSE).cloned().unwrap_or_default(),
            other => format!("{other:?}"),
        }
    }

    async fn connect_query(&mut self, scenario_endpoint: &str) {
        let endpoint = self.resolve(scenario_endpoint).await;
        match QueryClient::connect_with_retry(&endpoint, &single_attempt()).await {
            Ok(c) => self.query.set(c),
            Err(e) => self.error = Some(e),
        }
    }
}

async fn query_ok(client: &QueryClient) -> EventBook {
    client
        .query("orders", Uuid::new_v4())
        .get_event_book()
        .await
        .expect("query RPC succeeds")
}

async fn command_ok(client: &CommandHandlerClient) {
    let resp = client
        .command("orders", Uuid::new_v4())
        .with_command(
            format!("{TYPE_PREFIX}AddItem"),
            &GenericCommand {
                data: "conn".into(),
                count: 1,
            },
        )
        .with_sequence(0)
        .execute()
        .await
        .expect("command RPC succeeds");
    assert_eq!(resp.events.expect("events").pages.len(), 1);
}

// --------------------------------------------------------------------------
// TCP
// --------------------------------------------------------------------------

#[when(expr = "I connect to {string}")]
async fn when_connect(world: &mut ConnectionWorld, endpoint: String) {
    world.connect_query(&endpoint).await;
}

#[then("the connection should succeed")]
async fn then_succeeds(world: &mut ConnectionWorld) {
    let book = query_ok(world.connected_query()).await;
    assert!(book.pages.is_empty());
}

#[then("the client should be ready for operations")]
async fn then_ready(world: &mut ConnectionWorld) {
    let before = world.backend().await.rpcs().len();
    query_ok(world.connected_query()).await;
    let rpcs = world.backend().await.rpcs();
    assert_eq!(rpcs.len(), before + 1);
    assert_eq!(rpcs.last(), Some(&"GetEventBook"));
}

/// The backend speaks plaintext HTTP/2; a successful RPC proves the
/// `http://` scheme was used without TLS.
#[then("the scheme should be treated as insecure")]
async fn then_insecure(world: &mut ConnectionWorld) {
    query_ok(world.connected_query()).await;
}

/// With `https://` the client negotiates TLS, which the plaintext backend
/// on the same port cannot complete, while the plain scheme succeeds.
#[then("the connection should use TLS")]
async fn then_tls(world: &mut ConnectionWorld) {
    let tls_failed = match world.error.as_ref() {
        Some(_) => true,
        None => world
            .query
            .get()
            .query("orders", Uuid::new_v4())
            .get_event_book()
            .await
            .is_err(),
    };
    assert!(tls_failed, "https endpoint spoke plaintext to the backend");
    let plain = world.resolve("http://localhost:1310").await;
    let client = QueryClient::connect_with_retry(&plain, &single_attempt())
        .await
        .expect("plaintext connection to the same port succeeds");
    query_ok(&client).await;
}

#[then("the connection should fail")]
async fn then_fails(world: &mut ConnectionWorld) {
    assert!(world.error.is_some(), "connection unexpectedly succeeded");
}

#[then("the error should indicate DNS or connection failure")]
async fn then_dns(world: &mut ConnectionWorld) {
    let err = world.err();
    assert!(err.is_connection_error(), "got {err:?}");
    assert_eq!(err.code(), codes::CONNECTION_FAILED);
}

#[then("the error should indicate connection refused")]
async fn then_refused(world: &mut ConnectionWorld) {
    let err = world.err();
    assert_eq!(err.code(), codes::CONNECTION_FAILED, "got {err:?}");
    let cause = world.cause().to_lowercase();
    assert!(cause.contains("refused"), "cause: {cause}");
}

// --------------------------------------------------------------------------
// Unix domain sockets
// --------------------------------------------------------------------------

#[given(expr = "a Unix socket at {string}")]
async fn given_socket(world: &mut ConnectionWorld, path: String) {
    let real = std::env::temp_dir()
        .join(format!("angzarr-conn-{}", Uuid::new_v4()))
        .join("angzarr.sock");
    world.backend = Some(TestBackend::start_uds(&real).await);
    world.sockets.insert(path, real);
}

#[then("the client should use UDS transport")]
async fn then_uds(world: &mut ConnectionWorld) {
    query_ok(world.connected_query()).await;
    let backend = world.backend.as_ref().expect("uds backend");
    assert!(
        backend.port().is_none(),
        "backend listens only on a Unix socket"
    );
    assert_eq!(backend.accepted_connections(), 1);
}

#[then("the error should indicate socket not found")]
async fn then_socket_missing(world: &mut ConnectionWorld) {
    let err = world.err();
    assert_eq!(err.code(), codes::CONNECTION_FAILED, "got {err:?}");
    let cause = world.cause().to_lowercase();
    assert!(cause.contains("no such file"), "cause: {cause}");
}

// --------------------------------------------------------------------------
// Environment variables
// --------------------------------------------------------------------------

#[given(expr = "environment variable {string} set to {string}")]
async fn given_env_set(world: &mut ConnectionWorld, name: String, value: String) {
    world.lock_env().await;
    let value = world.resolve(&value).await;
    std::env::set_var(&name, value);
    world.env_vars.push(name);
}

#[given(expr = "environment variable {string} is not set")]
async fn given_env_unset(world: &mut ConnectionWorld, name: String) {
    world.lock_env().await;
    std::env::remove_var(&name);
}

#[when(expr = "I call from_env\\({string}, {string}\\)")]
async fn when_from_env(world: &mut ConnectionWorld, name: String, default: String) {
    let default = world.resolve(&default).await;
    match QueryClient::from_env(&name, &default).await {
        Ok(c) => world.query.set(c),
        Err(e) => world.error = Some(e),
    }
}

#[then(expr = "the connection should use {string}")]
async fn then_uses(world: &mut ConnectionWorld, endpoint: String) {
    assert!(
        endpoint.ends_with(":1310"),
        "scenario names the backend port"
    );
    let before = world.backend().await.accepted_connections();
    query_ok(world.connected_query()).await;
    assert!(
        world.backend().await.accepted_connections() >= before.max(1),
        "connection did not reach {endpoint}"
    );
}

// --------------------------------------------------------------------------
// Channel reuse
// --------------------------------------------------------------------------

#[given("an existing gRPC channel")]
async fn given_channel(world: &mut ConnectionWorld) {
    let endpoint = world.backend().await.endpoint().to_string();
    let channel = tonic::transport::Endpoint::from_shared(endpoint)
        .expect("valid endpoint")
        .connect()
        .await
        .expect("channel connects");
    world.channel.set(channel);
}

#[when(regex = r"^I call from_channel\(channel\)$")]
async fn when_from_channel(world: &mut ConnectionWorld) {
    world
        .query
        .set(QueryClient::from_channel(world.channel.get().clone()));
}

#[then("the client should reuse that channel")]
async fn then_reuses(world: &mut ConnectionWorld) {
    query_ok(world.query.get()).await;
    assert_eq!(world.backend().await.rpcs(), vec!["GetEventBook"]);
}

#[then("no new connection should be created")]
async fn then_no_new(world: &mut ConnectionWorld) {
    query_ok(world.query.get()).await;
    assert_eq!(world.backend().await.accepted_connections(), 1);
}

#[when("I create QueryClient from the channel")]
async fn when_query_from_channel(world: &mut ConnectionWorld) {
    world
        .query
        .set(QueryClient::from_channel(world.channel.get().clone()));
}

#[when("I create CommandHandlerClient from the same channel")]
async fn when_command_from_channel(world: &mut ConnectionWorld) {
    world.command.set(CommandHandlerClient::from_channel(
        world.channel.get().clone(),
    ));
}

#[then("both clients should share the connection")]
async fn then_share(world: &mut ConnectionWorld) {
    query_ok(world.query.get()).await;
    command_ok(world.command.get()).await;
    assert_eq!(
        world.backend().await.rpcs(),
        vec!["GetEventBook", "HandleCommand"]
    );
}

#[then("the connection should only be established once")]
async fn then_once(world: &mut ConnectionWorld) {
    assert_eq!(world.backend().await.accepted_connections(), 1);
}

// --------------------------------------------------------------------------
// Client types
// --------------------------------------------------------------------------

#[when(expr = "I create a QueryClient connected to {string}")]
async fn when_query_client(world: &mut ConnectionWorld, endpoint: String) {
    world.connect_query(&endpoint).await;
}

#[then("the client should be able to query events")]
async fn then_can_query(world: &mut ConnectionWorld) {
    query_ok(world.connected_query()).await;
}

#[when(expr = "I create a CommandHandlerClient connected to {string}")]
async fn when_command_client(world: &mut ConnectionWorld, endpoint: String) {
    let endpoint = world.resolve(&endpoint).await;
    let client = CommandHandlerClient::connect_with_retry(&endpoint, &single_attempt())
        .await
        .expect("command handler client connects");
    world.command.set(client);
}

#[then("the client should be able to execute commands")]
async fn then_can_execute(world: &mut ConnectionWorld) {
    command_ok(world.command.get()).await;
}

#[when(expr = "I create a SpeculativeClient connected to {string}")]
async fn when_speculative_client(world: &mut ConnectionWorld, endpoint: String) {
    let endpoint = world.resolve(&endpoint).await;
    let client = SpeculativeClient::connect_with_retry(&endpoint, &single_attempt())
        .await
        .expect("speculative client connects");
    world.speculative.set(client);
}

#[then("the client should be able to perform speculative operations")]
async fn then_can_speculate(world: &mut ConnectionWorld) {
    let projection = world
        .speculative
        .get()
        .projector(SpeculateProjectorRequest {
            events: Some(EventBook::default()),
        })
        .await
        .expect("speculative RPC succeeds");
    assert_eq!(projection.projector, "order-summary");
}

#[when(expr = "I create a DomainClient connected to {string}")]
async fn when_domain_client(world: &mut ConnectionWorld, endpoint: String) {
    let endpoint = world.resolve(&endpoint).await;
    let client = DomainClient::connect_with_retry(&endpoint, &single_attempt())
        .await
        .expect("domain client connects");
    world.domain.set(client);
}

#[then("the client should have aggregate and query sub-clients")]
async fn then_sub_clients(world: &mut ConnectionWorld) {
    let dc = world.domain.get();
    command_ok(&dc.command_handler).await;
    query_ok(&dc.query).await;
}

#[then("both should share the same connection")]
async fn then_domain_shares(world: &mut ConnectionWorld) {
    let rpcs = world.backend().await.rpcs();
    assert_eq!(rpcs, vec!["HandleCommand", "GetEventBook"]);
    assert_eq!(world.backend().await.accepted_connections(), 1);
}

// --------------------------------------------------------------------------
// Connection options
// --------------------------------------------------------------------------

#[when(expr = "I connect with timeout of {int} seconds")]
async fn when_connect_timeout(_world: &mut ConnectionWorld, seconds: u64) {
    panic!(
        "angzarr_client has no connect-timeout option: connect/connect_with_retry take only an \
         endpoint and a RetryPolicy, and create_channel applies a fixed 30s per-RPC timeout \
         (requested {seconds}s)"
    );
}

#[then("the connection should respect the timeout")]
async fn then_respects_timeout(_world: &mut ConnectionWorld) {
    panic!("no connect-timeout option to observe");
}

#[then("slow connections should fail after timeout")]
async fn then_slow_fail(_world: &mut ConnectionWorld) {
    panic!("no connect-timeout option to observe");
}

#[when("I connect with keep-alive enabled")]
async fn when_keepalive(world: &mut ConnectionWorld) {
    world.connect_query("localhost:1310").await;
}

#[then("the connection should send keep-alive probes")]
async fn then_keepalive_probes(_world: &mut ConnectionWorld) {
    panic!(
        "keep-alive is fixed inside create_channel (30s HTTP/2 ping); the client exposes no \
         option to enable or tune it and the test backend cannot observe HTTP/2 PING frames"
    );
}

#[then("idle connections should remain open")]
async fn then_idle_open(world: &mut ConnectionWorld) {
    query_ok(world.connected_query()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    query_ok(world.connected_query()).await;
    assert_eq!(world.backend().await.accepted_connections(), 1);
    assert_eq!(world.backend().await.active_connections(), 1);
}

// --------------------------------------------------------------------------
// Errors
// --------------------------------------------------------------------------

#[then("the error should indicate invalid format")]
async fn then_invalid_format(world: &mut ConnectionWorld) {
    assert_eq!(world.err().code(), codes::ENDPOINT_INVALID_URI);
}

#[given("an established connection")]
async fn given_established(world: &mut ConnectionWorld) {
    world.connect_query("localhost:1310").await;
    query_ok(world.connected_query()).await;
}

#[when("the server disconnects")]
async fn when_server_disconnects(world: &mut ConnectionWorld) {
    world.backend.as_mut().expect("backend").stop().await;
}

#[when("I attempt an operation")]
async fn when_attempt(world: &mut ConnectionWorld) {
    let result = world
        .query
        .get()
        .query("orders", Uuid::new_v4())
        .get_event_book()
        .await;
    world.operation = Some(result);
}

#[then("the operation should fail")]
async fn then_operation_fails(world: &mut ConnectionWorld) {
    let op = world.operation.as_ref().expect("operation attempted");
    assert!(op.is_err(), "operation succeeded: {op:?}");
}

#[then("the error should indicate connection lost")]
async fn then_connection_lost(world: &mut ConnectionWorld) {
    let err = world
        .operation
        .as_ref()
        .expect("operation attempted")
        .as_ref()
        .expect_err("operation failed");
    assert!(err.is_connection_error(), "got {err:?}");
}

#[given("a connection that failed")]
async fn given_failed(world: &mut ConnectionWorld) {
    let port = world.backend().await.port().expect("tcp backend");
    world.first_port = Some(port);
    world.connect_query("localhost:1310").await;
    world.backend.as_mut().expect("backend").stop().await;
    let failed = world
        .query
        .get()
        .query("orders", Uuid::new_v4())
        .get_event_book()
        .await;
    assert!(failed.is_err(), "connection did not fail");
}

#[when("I create a new client with the same endpoint")]
async fn when_new_client(world: &mut ConnectionWorld) {
    let port = world.first_port.expect("first port");
    world.backend = Some(TestBackend::start_tcp_on(port).await);
    world.error = None;
    let fresh =
        QueryClient::connect_with_retry(&format!("http://localhost:{port}"), &single_attempt())
            .await;
    match fresh {
        Ok(c) => world.query.set(c),
        Err(e) => world.error = Some(e),
    }
}

#[then("the new connection should be independent")]
async fn then_independent(world: &mut ConnectionWorld) {
    query_ok(world.connected_query()).await;
    assert_eq!(world.backend().await.accepted_connections(), 1);
}

#[then("the new connection should succeed if server is available")]
async fn then_new_succeeds(world: &mut ConnectionWorld) {
    let book = query_ok(world.connected_query()).await;
    assert!(book.pages.is_empty());
    let rpcs = world.backend().await.rpcs();
    assert!(
        !rpcs.is_empty() && rpcs.iter().all(|r| *r == "GetEventBook"),
        "rpcs: {rpcs:?}"
    );
}
