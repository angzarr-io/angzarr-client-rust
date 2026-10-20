//! Step definitions for `features/client/aggregate_client.feature`.
//!
//! Drives a real [`CommandHandlerClient`] (through the fluent
//! [`CommandBuilderExt`] surface) against the in-process test backend
//! (`tests/common/backend.rs`), and reads history back with a real
//! [`QueryClient`].

use std::time::Duration;

use angzarr_client::proto::{CommandResponse, EventBook};
use angzarr_client::{
    full_type_url, ClientError, CommandBuilderExt, CommandHandlerClient, QueryBuilderExt,
    QueryClient,
};
use cucumber::{given, then, when, World};
use prost::Message;
use uuid::Uuid;

use crate::common::backend::{
    correlation_for, root_for, short_type, single_attempt, unavailable_endpoint, GenericCommand,
    Hidden, TestBackend, TYPE_PREFIX,
};
use crate::common::fixtures::CreateOrder;

/// A payload whose field 1 is a varint, which `CreateOrder` (field 1 =
/// string) cannot decode.
#[derive(Clone, PartialEq, Message)]
struct MalformedPayload {
    #[prost(uint64, tag = "1")]
    value: u64,
}

#[derive(Debug, Default, World)]
pub struct AggregateClientWorld {
    backend: Option<TestBackend>,
    client: Hidden<CommandHandlerClient>,
    endpoint: String,
    domain: String,
    root: Uuid,
    result: Option<Result<CommandResponse, ClientError>>,
    /// The answer of an ASYNC (fire-and-forget) send.
    accepted: Option<Result<(), ClientError>>,
    concurrent: Vec<Result<CommandResponse, ClientError>>,
    timed_out: bool,
    looked_up_sequence: Option<u64>,
    read_back: Option<EventBook>,
    last_sequence: u64,
}

impl AggregateClientWorld {
    fn backend(&self) -> &TestBackend {
        self.backend.as_ref().expect("test backend running")
    }

    fn client(&self) -> &CommandHandlerClient {
        self.client.get()
    }

    async fn query_client(&self) -> QueryClient {
        QueryClient::connect(self.backend().endpoint())
            .await
            .expect("query client connects to the test backend")
    }

    async fn send_generic(
        &self,
        name: &str,
        sequence: u64,
        count: u32,
        correlation: Option<&str>,
    ) -> Result<CommandResponse, ClientError> {
        let mut builder = self
            .client()
            .command(&self.domain, self.root)
            .with_command(
                format!("{TYPE_PREFIX}{name}"),
                &GenericCommand {
                    data: format!("{name}-data"),
                    count,
                },
            )
            .with_sequence(sequence);
        if let Some(label) = correlation {
            builder = builder.with_correlation_id(correlation_for(label));
        }
        builder.execute().await
    }

    async fn send_create(
        &self,
        customer_id: &str,
        sequence: u64,
    ) -> Result<CommandResponse, ClientError> {
        self.client()
            .command(&self.domain, self.root)
            .with_command(
                full_type_url::<CreateOrder>(),
                &CreateOrder {
                    order_id: "o-1".into(),
                    customer_id: customer_id.into(),
                    items: vec![],
                },
            )
            .with_sequence(sequence)
            .execute()
            .await
    }

    fn ok_response(&self) -> &CommandResponse {
        match self.result.as_ref().expect("a command was sent") {
            Ok(r) => r,
            Err(e) => panic!("command failed: {e:?}"),
        }
    }

    fn err(&self) -> &ClientError {
        match self.result.as_ref().expect("a command was sent") {
            Ok(r) => panic!("command unexpectedly accepted: {r:?}"),
            Err(e) => e,
        }
    }

    fn emitted(&self) -> &EventBook {
        self.ok_response()
            .events
            .as_ref()
            .expect("response carries the emitted events")
    }
}

fn seqs(book: &EventBook) -> Vec<u64> {
    book.pages.iter().map(|p| p.sequence).collect()
}

/// A refusal for a stale expected sequence: ABORTED (types.proto,
/// Handling; STRICT).
fn is_stale(err: &ClientError) -> bool {
    err.status()
        .is_some_and(|s| s.code() == tonic::Code::Aborted)
}

// --------------------------------------------------------------------------
// Background / arrangement
// --------------------------------------------------------------------------

#[given("a client connected to the test backend")]
async fn given_connected(world: &mut AggregateClientWorld) {
    let backend = TestBackend::start_tcp().await;
    backend.set_known_domains(&["orders", "inventory"]);
    backend.add_projector("orders");
    world.client.set(
        CommandHandlerClient::connect(backend.endpoint())
            .await
            .expect("client connects to the test backend"),
    );
    world.endpoint = backend.endpoint().to_string();
    world.backend = Some(backend);
}

#[given(expr = "a new aggregate root in domain {string}")]
async fn given_new_root(world: &mut AggregateClientWorld, domain: String) {
    world.domain = domain;
    world.root = Uuid::new_v4();
    world.last_sequence = 0;
}

#[given(expr = "an aggregate {string} with root {string} at sequence {int}")]
async fn given_aggregate_at(
    world: &mut AggregateClientWorld,
    domain: String,
    root: String,
    seq: u64,
) {
    let cover = crate::common::backend::cover(&domain, &root, None);
    world.backend().seed(&cover, "ItemAdded", seq);
    world.domain = domain;
    world.root = root_for(&root);
    world.last_sequence = seq;
}

#[given(expr = "an aggregate {string} with root {string}")]
async fn given_aggregate(world: &mut AggregateClientWorld, domain: String, root: String) {
    world.domain = domain;
    world.root = root_for(&root);
}

#[given(expr = "no aggregate exists for domain {string} root {string}")]
async fn given_no_aggregate(world: &mut AggregateClientWorld, domain: String, root: String) {
    let cover = crate::common::backend::cover(&domain, &root, None);
    assert!(world.backend().stored_pages(&cover).is_empty());
    world.domain = domain;
    world.root = root_for(&root);
}

#[given(expr = "projectors are configured for {string} domain")]
async fn given_projectors(world: &mut AggregateClientWorld, domain: String) {
    world.backend().add_projector(&domain);
}

#[given("the aggregate service is unavailable")]
async fn given_unavailable(world: &mut AggregateClientWorld) {
    world.endpoint = unavailable_endpoint().await;
    world.domain = "orders".into();
    world.root = Uuid::new_v4();
}

#[given("the aggregate service does not respond in time")]
async fn given_slow(world: &mut AggregateClientWorld) {
    world.backend().set_response_delay(Duration::from_secs(3));
    world.domain = "orders".into();
    world.root = Uuid::new_v4();
}

// --------------------------------------------------------------------------
// Actions
// --------------------------------------------------------------------------

#[when(expr = "I send a {string} command with data {string}")]
async fn when_send_named_with_data(world: &mut AggregateClientWorld, name: String, data: String) {
    assert_eq!(
        name, "CreateOrder",
        "scripted aggregate creates via CreateOrder"
    );
    world.result = Some(world.send_create(&data, world.last_sequence).await);
}

#[when(expr = "I send an {string} command at sequence {int}")]
async fn when_send_named_at(world: &mut AggregateClientWorld, name: String, seq: u64) {
    world.result = Some(world.send_generic(&name, seq, 1, None).await);
}

#[when(expr = "I send a command tagged with correlation ID {string}")]
async fn when_send_correlated(world: &mut AggregateClientWorld, id: String) {
    world.result = Some(
        world
            .send_generic("AddItem", world.last_sequence, 1, Some(&id))
            .await,
    );
}

#[when(expr = "I send a command at sequence {int}")]
async fn when_send_at(world: &mut AggregateClientWorld, seq: u64) {
    world.result = Some(world.send_generic("AddItem", seq, 1, None).await);
}

#[when(expr = "two commands are sent concurrently at sequence {int}")]
async fn when_concurrent(world: &mut AggregateClientWorld, seq: u64) {
    let (a, b) = tokio::join!(
        world.send_generic("AddItem", seq, 1, None),
        world.send_generic("AddItem", seq, 1, None),
    );
    world.concurrent = vec![a, b];
}

#[when(expr = "I look up the current sequence for {string} root {string}")]
async fn when_lookup(world: &mut AggregateClientWorld, domain: String, root: String) {
    let qc = world.query_client().await;
    let book = qc
        .query(domain, root_for(&root))
        .get_event_book()
        .await
        .expect("query succeeds");
    world.looked_up_sequence = Some(book.next_sequence);
}

#[when("I retry the command at that sequence")]
async fn when_retry(world: &mut AggregateClientWorld) {
    let seq = world.looked_up_sequence.expect("sequence looked up");
    world.result = Some(world.send_generic("AddItem", seq, 1, None).await);
}

#[when("I send a command without waiting for downstream work")]
async fn when_send_async(world: &mut AggregateClientWorld) {
    let accepted = world
        .client()
        .command(&world.domain, world.root)
        .with_command(
            format!("{TYPE_PREFIX}AddItem"),
            &GenericCommand {
                data: "AddItem-data".into(),
                count: 1,
            },
        )
        .execute_async()
        .await;
    world.accepted = Some(accepted);
}

#[when("I send a command and wait for projectors")]
async fn when_send_sync(world: &mut AggregateClientWorld) {
    let result = world
        .client()
        .command(&world.domain, world.root)
        .with_command(
            format!("{TYPE_PREFIX}AddItem"),
            &GenericCommand {
                data: "AddItem-data".into(),
                count: 1,
            },
        )
        .execute_sync()
        .await;
    world.result = Some(result);
}

#[when("I send a command with a malformed payload")]
async fn when_send_malformed(world: &mut AggregateClientWorld) {
    let result = world
        .client()
        .command(&world.domain, world.root)
        .with_command(
            full_type_url::<CreateOrder>(),
            &MalformedPayload { value: 7 },
        )
        .with_sequence(0)
        .execute()
        .await;
    world.result = Some(result);
}

#[when("I send a command missing required fields")]
async fn when_send_missing_fields(world: &mut AggregateClientWorld) {
    world.result = Some(world.send_create("", 0).await);
}

#[when(expr = "I send a command to domain {string}")]
async fn when_send_to_domain(world: &mut AggregateClientWorld, domain: String) {
    world.domain = domain;
    world.root = Uuid::new_v4();
    world.result = Some(world.send_generic("AddItem", 0, 1, None).await);
}

#[when(expr = "I send a command that produces {int} events")]
async fn when_send_multi(world: &mut AggregateClientWorld, n: u32) {
    world.result = Some(
        world
            .send_generic("AddItem", world.last_sequence, n, None)
            .await,
    );
}

#[when(expr = "I read back the events for {string} root {string}")]
async fn when_read_back(world: &mut AggregateClientWorld, domain: String, root: String) {
    let qc = world.query_client().await;
    world.read_back = Some(
        qc.query(domain, root_for(&root))
            .get_event_book()
            .await
            .expect("read back succeeds"),
    );
}

#[when("I attempt to send a command")]
async fn when_attempt(world: &mut AggregateClientWorld) {
    let result =
        match CommandHandlerClient::connect_with_retry(&world.endpoint, &single_attempt()).await {
            Ok(client) => {
                world.client.set(client);
                world.send_generic("AddItem", 0, 1, None).await
            }
            Err(e) => Err(e),
        };
    world.result = Some(result);
}

#[when("I send a command with a short timeout")]
async fn when_short_timeout(world: &mut AggregateClientWorld) {
    let outcome = tokio::time::timeout(
        Duration::from_millis(200),
        world.send_generic("AddItem", 0, 1, None),
    )
    .await;
    match outcome {
        Err(_elapsed) => world.timed_out = true,
        Ok(r) => world.result = Some(r),
    }
}

#[when(expr = "I send a {string} command for root {string} at sequence {int}")]
async fn when_send_named_for_root(
    world: &mut AggregateClientWorld,
    name: String,
    root: String,
    seq: u64,
) {
    assert_eq!(
        name, "CreateOrder",
        "scripted aggregate creates via CreateOrder"
    );
    world.root = root_for(&root);
    world.result = Some(world.send_create("customer-1", seq).await);
}

// --------------------------------------------------------------------------
// Outcomes
// --------------------------------------------------------------------------

#[then("the command is accepted")]
async fn then_accepted(world: &mut AggregateClientWorld) {
    let book = world.emitted();
    assert!(!book.pages.is_empty(), "accepted command emitted no events");
}

#[then(expr = "a single {string} event is recorded")]
async fn then_single_event(world: &mut AggregateClientWorld, name: String) {
    let book = world.emitted();
    assert_eq!(book.pages.len(), 1);
    let any = match &book.pages[0].payload {
        Some(a) => a,
        other => panic!("expected event payload, got {other:?}"),
    };
    assert_eq!(short_type(any), name);
    let cover = book.cover.clone().expect("cover");
    assert_eq!(world.backend().stored_pages(&cover).len(), 1);
}

#[then(expr = "the new events continue the history from sequence {int}")]
async fn then_continue_from(world: &mut AggregateClientWorld, seq: u64) {
    assert_eq!(seqs(world.emitted()).first().copied(), Some(seq));
}

#[then(expr = "the resulting events carry correlation ID {string}")]
async fn then_correlation(world: &mut AggregateClientWorld, id: String) {
    let want = angzarr_client::uuid_to_proto(correlation_for(&id));
    let pages = &world.emitted().pages;
    assert!(!pages.is_empty());
    for page in pages {
        let got = page.meta.as_ref().and_then(|m| m.correlation_id.as_ref());
        assert_eq!(got, Some(&want));
    }
}

#[then("the command is refused because the aggregate has moved on")]
async fn then_refused_moved_on(world: &mut AggregateClientWorld) {
    let err = world.err();
    assert!(is_stale(err), "expected ABORTED, got {err:?}");
}

#[then("one command is accepted")]
async fn then_one_accepted(world: &mut AggregateClientWorld) {
    assert_eq!(world.concurrent.iter().filter(|r| r.is_ok()).count(), 1);
}

#[then("the other is refused because the aggregate has moved on")]
async fn then_other_refused(world: &mut AggregateClientWorld) {
    let refused: Vec<&ClientError> = world
        .concurrent
        .iter()
        .filter_map(|r| r.as_ref().err())
        .collect();
    assert_eq!(refused.len(), 1);
    assert!(is_stale(refused[0]), "got {:?}", refused[0]);
}

#[then("the response returns before any projectors have caught up")]
async fn then_returns_before_projectors(world: &mut AggregateClientWorld) {
    let accepted = world.accepted.as_ref().expect("an async send");
    assert!(accepted.is_ok(), "async send failed: {accepted:?}");
    assert!(
        world.backend().projector_runs().is_empty(),
        "projector already ran"
    );
    assert_eq!(world.backend().rpcs().last(), Some(&"HandleAsync"));
}

#[then("the response reflects the projectors having processed the event")]
async fn then_projectors_processed(world: &mut AggregateClientWorld) {
    let resp = world.ok_response();
    let emitted = seqs(resp.events.as_ref().expect("events"));
    let projected: Vec<u64> = resp
        .outcome
        .as_ref()
        .expect("outcome")
        .projections
        .iter()
        .map(|p| p.next_sequence - 1)
        .collect();
    assert!(!projected.is_empty(), "no projections in the response");
    assert_eq!(projected, emitted);
    assert_eq!(world.backend().rpcs().last(), Some(&"HandleSync"));
}

#[then("the command is refused as invalid")]
async fn then_refused_invalid(world: &mut AggregateClientWorld) {
    let err = world.err();
    assert!(
        err.is_invalid_argument(),
        "expected INVALID_ARGUMENT, got {err:?}"
    );
}

#[then("the refusal names the missing field")]
async fn then_names_field(world: &mut AggregateClientWorld) {
    let status = world.err().status().expect("server status");
    assert!(
        status.message().contains("customer_id"),
        "message: {}",
        status.message()
    );
}

#[then("the command is refused because the domain is unknown")]
async fn then_unknown_domain(world: &mut AggregateClientWorld) {
    let err = world.err();
    assert!(err.is_not_found(), "expected NOT_FOUND, got {err:?}");
}

#[then(expr = "{int} events are recorded")]
async fn then_n_recorded(world: &mut AggregateClientWorld, n: usize) {
    assert_eq!(world.emitted().pages.len(), n);
    let cover = world.emitted().cover.clone().expect("cover");
    assert_eq!(world.backend().stored_pages(&cover).len(), n);
}

#[then(expr = "the events occupy consecutive sequences starting at {int}")]
async fn then_consecutive(world: &mut AggregateClientWorld, start: u64) {
    let s = seqs(world.emitted());
    let expected: Vec<u64> = (start..start + s.len() as u64).collect();
    assert_eq!(s, expected);
}

#[then(expr = "either all {int} events are present or none of them are")]
async fn then_atomic(world: &mut AggregateClientWorld, n: usize) {
    let read = world.read_back.as_ref().expect("read back").pages.len();
    assert!(read == n || read == 0, "partial write: {read} of {n}");
}

#[then("the call fails because the service cannot be reached")]
async fn then_unreachable(world: &mut AggregateClientWorld) {
    let err = world.err();
    assert!(
        err.is_connection_error(),
        "expected connection error, got {err:?}"
    );
}

#[then("the call fails because the deadline was exceeded")]
async fn then_deadline(world: &mut AggregateClientWorld) {
    assert!(world.timed_out, "call completed: {:?}", world.result);
}

#[then("the aggregate now exists with one event")]
async fn then_exists_one(world: &mut AggregateClientWorld) {
    let qc = world.query_client().await;
    let book = qc
        .query(&world.domain, world.root)
        .get_event_book()
        .await
        .expect("query succeeds");
    assert_eq!(book.pages.len(), 1);
    assert_eq!(book.next_sequence, 1);
}
