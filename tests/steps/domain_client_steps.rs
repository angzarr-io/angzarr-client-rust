//! Step definitions for `features/client/domain-client.feature`.
//!
//! The coordinator for domain "test" is the in-process test backend
//! (`tests/common/backend.rs`) listening on the Unix socket that
//! `resolve_ch_endpoint("test", Standalone)` names
//! (`$ANGZARR_UDS_BASE/ch-test.sock`), so `DomainClient::for_domain`,
//! `DomainClient::connect` and `DomainClient::from_env` all reach the
//! same backend over a real socket.

use std::path::PathBuf;
use std::time::Duration;

use angzarr_client::proto::{CommandResponse, EventPage};
use angzarr_client::{
    ClientError, CommandBuilderExt, DomainClient, QueryBuilderExt, TransportMode,
};
use cucumber::{given, then, when, World};
use uuid::Uuid;

use crate::common::backend::{
    cover, env_lock, root_for, GenericCommand, Hidden, TestBackend, TYPE_PREFIX,
};

#[derive(Debug, Default, World)]
pub struct DomainClientWorld {
    backend: Option<TestBackend>,
    uds_base: Option<PathBuf>,
    client: Hidden<DomainClient>,
    domain: String,
    root: Option<Uuid>,
    command: Option<Result<CommandResponse, ClientError>>,
    pages: Option<Result<Vec<EventPage>, ClientError>>,
    env_guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    env_vars: Vec<String>,
    accepted_before_close: usize,
}

impl Drop for DomainClientWorld {
    fn drop(&mut self) {
        for name in &self.env_vars {
            std::env::remove_var(name);
        }
        if let Some(base) = &self.uds_base {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

impl DomainClientWorld {
    fn backend(&self) -> &TestBackend {
        self.backend.as_ref().expect("coordinator running")
    }

    async fn lock_env(&mut self) {
        if self.env_guard.is_none() {
            self.env_guard = Some(env_lock().lock_owned().await);
        }
    }

    async fn send(&self) -> Result<CommandResponse, ClientError> {
        let root = self.root.unwrap_or_else(Uuid::new_v4);
        self.client
            .get()
            .command(&self.domain, root)
            .with_command(
                format!("{TYPE_PREFIX}AddItem"),
                &GenericCommand {
                    data: "via-domain-client".into(),
                    count: 1,
                },
            )
            .with_sequence(0)
            .execute()
            .await
    }

    fn command_ok(&self) -> &CommandResponse {
        match self.command.as_ref().expect("a command was sent") {
            Ok(r) => r,
            Err(e) => panic!("command failed: {e:?}"),
        }
    }
}

// --------------------------------------------------------------------------
// Arrangement
// --------------------------------------------------------------------------

#[given(expr = "a running aggregate coordinator for domain {string}")]
async fn given_coordinator(world: &mut DomainClientWorld, domain: String) {
    let base = std::env::temp_dir().join(format!("angzarr-dc-{}", Uuid::new_v4()));
    let socket = base.join(format!("ch-{domain}.sock"));
    world.backend = Some(TestBackend::start_uds(&socket).await);
    world.uds_base = Some(base);
    world.domain = domain;
}

#[given(expr = "a registered aggregate handler for domain {string}")]
async fn given_handler(world: &mut DomainClientWorld, domain: String) {
    world.backend().set_known_domains(&[domain.as_str()]);
}

#[given(expr = "an aggregate {string} with root {string} has {int} events")]
async fn given_events(world: &mut DomainClientWorld, domain: String, root: String, n: u32) {
    world
        .backend()
        .seed(&cover(&domain, &root, None, ""), "ItemAdded", n);
    world.root = Some(root_for(&root));
}

#[given("a connected domain client")]
async fn given_connected(world: &mut DomainClientWorld) {
    let client = DomainClient::connect(world.backend().endpoint())
        .await
        .expect("domain client connects");
    world.client.set(client);
}

#[given(expr = "environment variable {string} is set to the coordinator endpoint")]
async fn given_env(world: &mut DomainClientWorld, name: String) {
    world.lock_env().await;
    std::env::set_var(&name, world.backend().endpoint());
    world.env_vars.push(name);
}

// --------------------------------------------------------------------------
// Actions
// --------------------------------------------------------------------------

#[when("I create a domain client for the coordinator endpoint")]
async fn when_create_endpoint(world: &mut DomainClientWorld) {
    given_connected(world).await;
}

#[when(expr = "I create a domain client for domain {string}")]
async fn when_create_for_domain(world: &mut DomainClientWorld, domain: String) {
    world.lock_env().await;
    let base = world.uds_base.clone().expect("coordinator socket base");
    std::env::set_var(angzarr_client::transport::ENV_UDS_BASE, &base);
    world
        .env_vars
        .push(angzarr_client::transport::ENV_UDS_BASE.to_string());
    let client = DomainClient::for_domain(&domain, Some(TransportMode::Standalone))
        .await
        .expect("for_domain resolves and connects to the coordinator");
    world.client.set(client);
    world.domain = domain;
}

#[when("I use the command builder to send a command")]
async fn when_builder_send(world: &mut DomainClientWorld) {
    world.command = Some(world.send().await);
}

#[when("I use the query builder to fetch events for that root")]
async fn when_builder_query(world: &mut DomainClientWorld) {
    let root = world.root.expect("root seeded");
    world.pages = Some(
        world
            .client
            .get()
            .query(&world.domain, root)
            .get_pages()
            .await,
    );
}

#[when("I send a command")]
async fn when_send(world: &mut DomainClientWorld) {
    world.root = Some(Uuid::new_v4());
    world.command = Some(world.send().await);
}

#[when("I query for the resulting events")]
async fn when_query_resulting(world: &mut DomainClientWorld) {
    let root = world.root.expect("command sent");
    world.pages = Some(
        world
            .client
            .get()
            .query(&world.domain, root)
            .get_pages()
            .await,
    );
}

#[when("I close the domain client")]
async fn when_close(world: &mut DomainClientWorld) {
    world.accepted_before_close = world.backend().accepted_connections();
    assert_eq!(world.backend().active_connections(), 1);
    world.client.take().expect("client connected").close();
}

#[when(expr = "I create a domain client from environment variable {string}")]
async fn when_from_env(world: &mut DomainClientWorld, name: String) {
    let client = DomainClient::from_env(&name, "unix:///nonexistent/default.sock")
        .await
        .expect("from_env connects to the endpoint in the variable");
    world.client.set(client);
}

// --------------------------------------------------------------------------
// Outcomes
// --------------------------------------------------------------------------

#[then("I should be able to query events")]
async fn then_can_query(world: &mut DomainClientWorld) {
    let book = world
        .client
        .get()
        .query(&world.domain, Uuid::new_v4())
        .get_event_book()
        .await
        .expect("query succeeds");
    assert!(book.pages.is_empty());
    assert!(world.backend().rpcs().contains(&"GetEventBook"));
}

#[then("I should be able to send commands")]
async fn then_can_send(world: &mut DomainClientWorld) {
    let resp = world.send().await.expect("command succeeds");
    assert_eq!(resp.events.expect("events").pages.len(), 1);
}

#[then("I should receive a command response")]
async fn then_command_response(world: &mut DomainClientWorld) {
    let events = world.command_ok().events.as_ref().expect("events");
    assert_eq!(events.pages.len(), 1);
    assert_eq!(events.cover.as_ref().expect("cover").domain, world.domain);
}

#[then(expr = "I should receive {int} event pages")]
async fn then_n_pages(world: &mut DomainClientWorld, n: usize) {
    let pages = world
        .pages
        .as_ref()
        .expect("query made")
        .as_ref()
        .expect("query succeeds");
    assert_eq!(pages.len(), n);
}

#[then("both operations should succeed on the same connection")]
async fn then_same_connection(world: &mut DomainClientWorld) {
    world.command_ok();
    let pages = world
        .pages
        .as_ref()
        .expect("query made")
        .as_ref()
        .expect("query succeeds");
    assert_eq!(pages.len(), 1, "query sees the command's event");
    assert_eq!(world.backend().accepted_connections(), 1);
}

async fn assert_severed(world: &DomainClientWorld) {
    let backend = world.backend();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while backend.active_connections() > 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "connection still open after close"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!world.client.is_some(), "closed client is consumed");
    assert_eq!(backend.accepted_connections(), world.accepted_before_close);
}

#[then("subsequent commands should fail with a connection error")]
async fn then_commands_fail(world: &mut DomainClientWorld) {
    assert_severed(world).await;
}

#[then("subsequent queries should fail with a connection error")]
async fn then_queries_fail(world: &mut DomainClientWorld) {
    assert_severed(world).await;
}

#[then("the domain client should be connected")]
async fn then_connected(world: &mut DomainClientWorld) {
    let book = world
        .client
        .get()
        .query(&world.domain, Uuid::new_v4())
        .get_event_book()
        .await
        .expect("query over the env-resolved endpoint succeeds");
    assert!(book.pages.is_empty());
    assert_eq!(world.backend().accepted_connections(), 1);
}
