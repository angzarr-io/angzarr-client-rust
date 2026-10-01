//! Step definitions for `features/client/speculative_client.feature`.
//!
//! Drives a real [`SpeculativeClient`] against the in-process test backend
//! (`tests/common/backend.rs`) and checks the real state with a real
//! [`QueryClient`].

use angzarr_client::proto::{
    command_page, event_page, page_header::SequenceType, temporal_query::PointInTime, CommandBook,
    CommandPage, CommandResponse, Cover, EventBook, EventPage, PageHeader,
    ProcessManagerHandleRequest, ProcessManagerHandleResponse, Projection, SagaHandleRequest,
    SagaResponse, SpeculateCommandHandlerRequest, SpeculatePmRequest, SpeculateProjectorRequest,
    SpeculateSagaRequest, TemporalQuery,
};
use angzarr_client::traits::SpeculativeClient as SpeculativeOps;
use angzarr_client::{ClientError, QueryBuilderExt, QueryClient, SpeculativeClient};
use cucumber::{given, then, when, World};
use prost::Message;

use crate::common::backend::{
    command_any, cover, event_any, page_seq, root_for, single_attempt, unavailable_endpoint,
    GenericEvent, Hidden, TestBackend,
};

#[derive(Debug, Default, World)]
pub struct SpeculativeClientWorld {
    backend: Option<TestBackend>,
    client: Hidden<SpeculativeClient>,
    endpoint: String,
    cover: Option<Cover>,
    events: Option<EventBook>,
    stored_before: usize,
    command: Option<Result<CommandResponse, ClientError>>,
    commands_ab: Vec<CommandResponse>,
    projection: Option<Result<Projection, ClientError>>,
    saga: Option<Result<SagaResponse, ClientError>>,
    pm: Option<Result<ProcessManagerHandleResponse, ClientError>>,
    real: Option<EventBook>,
    error: Option<ClientError>,
}

impl SpeculativeClientWorld {
    fn backend(&self) -> &TestBackend {
        self.backend.as_ref().expect("test backend running")
    }

    fn cover(&self) -> Cover {
        self.cover.clone().expect("aggregate chosen")
    }

    fn command_book(&self, name: &str, data: &str, count: u32) -> CommandBook {
        CommandBook {
            cover: Some(self.cover()),
            pages: vec![CommandPage {
                header: Some(PageHeader {
                    sequence_type: Some(SequenceType::Sequence(0)),
                    sync_mode: None,
                }),
                payload: Some(command_page::Payload::Command(command_any(
                    name, data, count,
                ))),
                ..Default::default()
            }],
        }
    }

    async fn speculate(
        &self,
        book: CommandBook,
        as_of: Option<u32>,
    ) -> Result<CommandResponse, ClientError> {
        self.client
            .get()
            .command_handler(SpeculateCommandHandlerRequest {
                command: Some(book),
                point_in_time: as_of.map(|s| TemporalQuery {
                    point_in_time: Some(PointInTime::AsOfSequence(s)),
                }),
            })
            .await
    }

    fn command_ok(&self) -> &CommandResponse {
        match self.command.as_ref().expect("speculative command executed") {
            Ok(r) => r,
            Err(e) => panic!("speculative command failed: {e:?}"),
        }
    }

    fn command_err(&self) -> &ClientError {
        match self.command.as_ref().expect("speculative command executed") {
            Ok(r) => panic!("speculative command unexpectedly succeeded: {r:?}"),
            Err(e) => e,
        }
    }

    fn emitted_seqs(&self) -> Vec<u32> {
        self.command_ok()
            .events
            .as_ref()
            .expect("events")
            .pages
            .iter()
            .filter_map(page_seq)
            .collect()
    }

    fn event_book(&self, cover: Cover, n: u32) -> EventBook {
        EventBook {
            cover: Some(cover),
            pages: (0..n)
                .map(|i| EventPage {
                    header: Some(PageHeader {
                        sequence_type: Some(SequenceType::Sequence(i)),
                        sync_mode: None,
                    }),
                    payload: Some(event_page::Payload::Event(event_any(
                        "OrderCreated",
                        &format!("e{i}"),
                    ))),
                    ..Default::default()
                })
                .collect(),
            next_sequence: n,
            ..Default::default()
        }
    }
}

fn data_of(book: &EventBook) -> Vec<String> {
    book.pages
        .iter()
        .map(|p| match &p.payload {
            Some(event_page::Payload::Event(a)) => {
                GenericEvent::decode(a.value.as_slice())
                    .expect("decodes")
                    .data
            }
            other => panic!("expected event payload, got {other:?}"),
        })
        .collect()
}

// --------------------------------------------------------------------------
// Arrangement
// --------------------------------------------------------------------------

#[given("a what-if execution surface available")]
async fn given_surface(world: &mut SpeculativeClientWorld) {
    let backend = TestBackend::start_tcp().await;
    world.client.set(
        SpeculativeClient::connect(backend.endpoint())
            .await
            .expect("speculative client connects to the test backend"),
    );
    world.endpoint = backend.endpoint().to_string();
    world.backend = Some(backend);
}

#[given(expr = "an aggregate {string} with root {string} has {int} events")]
async fn given_n_events(world: &mut SpeculativeClientWorld, domain: String, root: String, n: u32) {
    let c = cover(&domain, &root, None, "");
    world.backend().seed(&c, "ItemAdded", n);
    world.stored_before = world.backend().total_events();
    world.cover = Some(c);
}

#[given(expr = "a speculative aggregate {string} with root {string} has {int} events")]
async fn given_spec_aggregate(
    world: &mut SpeculativeClientWorld,
    domain: String,
    root: String,
    n: u32,
) {
    given_n_events(world, domain, root, n).await;
}

#[given(expr = "an aggregate {string} with root {string} in state {string}")]
async fn given_in_state(
    world: &mut SpeculativeClientWorld,
    domain: String,
    root: String,
    state: String,
) {
    assert_eq!(state, "shipped", "only the shipped state is scripted");
    let c = cover(&domain, &root, None, "");
    world.backend().seed(&c, "OrderCreated", 1);
    world.backend().seed(&c, "OrderShipped", 1);
    world.stored_before = world.backend().total_events();
    world.cover = Some(c);
}

#[given(expr = "an aggregate {string} with root {string}")]
async fn given_aggregate(world: &mut SpeculativeClientWorld, domain: String, root: String) {
    world.cover = Some(cover(&domain, &root, None, ""));
    world.stored_before = world.backend().total_events();
}

#[given(expr = "events for {string} root {string}")]
async fn given_events_for(world: &mut SpeculativeClientWorld, domain: String, root: String) {
    let book = world.event_book(cover(&domain, &root, None, "corr-1"), 3);
    world.events = Some(book);
    world.stored_before = world.backend().total_events();
}

#[given(expr = "{int} events for {string} root {string}")]
async fn given_n_events_for(
    world: &mut SpeculativeClientWorld,
    n: u32,
    domain: String,
    root: String,
) {
    let book = world.event_book(cover(&domain, &root, None, "corr-1"), n);
    world.events = Some(book);
    world.stored_before = world.backend().total_events();
}

#[given(expr = "events with saga origin from {string} aggregate")]
async fn given_saga_origin(world: &mut SpeculativeClientWorld, domain: String) {
    let c = cover(&domain, "origin-root", None, "corr-1");
    world.cover = Some(c.clone());
    world.events = Some(world.event_book(c, 2));
}

#[given("correlated events from multiple domains")]
async fn given_correlated(world: &mut SpeculativeClientWorld) {
    let mut book = world.event_book(cover("orders", "wf-order", None, "workflow-9"), 1);
    book.pages.push(EventPage {
        header: Some(PageHeader {
            sequence_type: Some(SequenceType::Sequence(1)),
            sync_mode: None,
        }),
        payload: Some(event_page::Payload::Event(event_any(
            "StockReserved",
            "inventory",
        ))),
        ..Default::default()
    });
    world.events = Some(book);
    world.stored_before = world.backend().total_events();
}

#[given("events without correlation ID")]
async fn given_uncorrelated(world: &mut SpeculativeClientWorld) {
    world.events = Some(world.event_book(cover("orders", "wf-order", None, ""), 1));
}

#[given("the speculative service is unavailable")]
async fn given_unavailable(world: &mut SpeculativeClientWorld) {
    world.endpoint = unavailable_endpoint().await;
}

// --------------------------------------------------------------------------
// Actions
// --------------------------------------------------------------------------

#[when(expr = "I speculatively execute a command against {string} root {string}")]
async fn when_spec_against(world: &mut SpeculativeClientWorld, domain: String, root: String) {
    assert_eq!(world.cover().domain, domain);
    assert_eq!(
        world.cover().root.expect("root").value,
        root_for(&root).as_bytes().to_vec()
    );
    let book = world.command_book("AddItem", "what-if", 1);
    world.command = Some(world.speculate(book, None).await);
}

#[when(expr = "I speculatively execute a command as of sequence {int}")]
async fn when_spec_as_of(world: &mut SpeculativeClientWorld, seq: u32) {
    let book = world.command_book("AddItem", "historical", 1);
    world.command = Some(world.speculate(book, Some(seq)).await);
}

#[when(expr = "I speculatively execute a {string} command")]
async fn when_spec_named(world: &mut SpeculativeClientWorld, name: String) {
    let book = world.command_book(&name, "what-if", 1);
    world.command = Some(world.speculate(book, None).await);
}

#[when("I speculatively execute a command with invalid payload")]
async fn when_spec_invalid(world: &mut SpeculativeClientWorld) {
    let mut book = world.command_book("AddItem", "", 1);
    if let Some(command_page::Payload::Command(any)) = book.pages[0].payload.as_mut() {
        any.value = vec![0x08, 0x01, 0xff];
    }
    world.command = Some(world.speculate(book, None).await);
}

#[when("I speculatively execute a command")]
async fn when_spec_plain(world: &mut SpeculativeClientWorld) {
    let book = world.command_book("AddItem", "what-if", 1);
    world.command = Some(world.speculate(book, None).await);
}

#[when(expr = "I speculatively execute a command producing {int} events")]
async fn when_spec_n(world: &mut SpeculativeClientWorld, n: u32) {
    let book = world.command_book("AddItem", "speculative", n);
    world.command = Some(world.speculate(book, None).await);
}

#[when(expr = "I speculatively execute command {word}")]
async fn when_spec_ab(world: &mut SpeculativeClientWorld, label: String) {
    let book = world.command_book("AddItem", &label, 1);
    let resp = world
        .speculate(book, None)
        .await
        .expect("speculation succeeds");
    world.commands_ab.push(resp);
}

#[when(expr = "I verify the real events for {string} root {string}")]
async fn when_verify_real(world: &mut SpeculativeClientWorld, domain: String, root: String) {
    let qc = QueryClient::connect(world.backend().endpoint())
        .await
        .expect("query client connects");
    world.real = Some(
        qc.query(domain, root_for(&root))
            .get_event_book()
            .await
            .expect("query succeeds"),
    );
}

#[when(expr = "I speculatively execute projector {string} against those events")]
async fn when_spec_projector_against(world: &mut SpeculativeClientWorld, _name: String) {
    let events = world.events.clone().expect("events");
    world.projection = Some(
        world
            .client
            .get()
            .projector(SpeculateProjectorRequest {
                events: Some(events),
            })
            .await,
    );
}

#[when(expr = "I speculatively execute projector {string}")]
async fn when_spec_projector(world: &mut SpeculativeClientWorld, name: String) {
    when_spec_projector_against(world, name).await;
}

#[when(expr = "I speculatively execute saga {string}")]
async fn when_spec_saga(world: &mut SpeculativeClientWorld, _name: String) {
    let source = world.events.clone().expect("events");
    world.saga = Some(
        world
            .client
            .get()
            .saga(SpeculateSagaRequest {
                request: Some(SagaHandleRequest {
                    source: Some(source),
                    ..Default::default()
                }),
            })
            .await,
    );
}

#[when(expr = "I speculatively execute process manager {string}")]
async fn when_spec_pm(world: &mut SpeculativeClientWorld, _name: String) {
    let trigger = world.events.clone().expect("events");
    world.pm = Some(
        world
            .client
            .get()
            .process_manager(SpeculatePmRequest {
                request: Some(ProcessManagerHandleRequest {
                    trigger: Some(trigger),
                    ..Default::default()
                }),
            })
            .await,
    );
}

#[when("I attempt speculative execution")]
async fn when_attempt(world: &mut SpeculativeClientWorld) {
    let result =
        match SpeculativeClient::connect_with_retry(&world.endpoint, &single_attempt()).await {
            Ok(client) => client
                .command_handler(SpeculateCommandHandlerRequest::default())
                .await
                .map(|_| ()),
            Err(e) => Err(e),
        };
    world.error = result.err();
}

#[when("I attempt speculative execution with missing parameters")]
async fn when_attempt_missing(world: &mut SpeculativeClientWorld) {
    world.error = world
        .client
        .get()
        .command_handler(SpeculateCommandHandlerRequest::default())
        .await
        .err();
}

// --------------------------------------------------------------------------
// Outcomes
// --------------------------------------------------------------------------

#[then("the response should contain the projected events")]
async fn then_projected(world: &mut SpeculativeClientWorld) {
    assert_eq!(world.emitted_seqs(), vec![3]);
}

#[then("the events should NOT be persisted")]
async fn then_not_persisted(world: &mut SpeculativeClientWorld) {
    let qc = QueryClient::connect(world.backend().endpoint())
        .await
        .expect("query client connects");
    let c = world.cover();
    let real = qc
        .query(
            &c.domain,
            uuid::Uuid::from_slice(&c.root.expect("root").value).expect("uuid"),
        )
        .get_event_book()
        .await
        .expect("query succeeds");
    assert_eq!(real.pages.len(), 3);
}

#[then("the command should execute against the historical state")]
async fn then_historical(world: &mut SpeculativeClientWorld) {
    assert_eq!(
        world.emitted_seqs(),
        vec![6],
        "must continue from sequence 5, not 9"
    );
}

#[then(expr = "the response should reflect state at sequence {int}")]
async fn then_reflect_state(world: &mut SpeculativeClientWorld, seq: u32) {
    let next = world
        .command_ok()
        .events
        .as_ref()
        .expect("events")
        .next_sequence;
    assert_eq!(
        next,
        seq + 2,
        "history through {seq} plus one projected event"
    );
}

#[then("the response should indicate rejection")]
async fn then_rejection(world: &mut SpeculativeClientWorld) {
    let err = world.command_err();
    assert!(
        err.is_precondition_failed(),
        "expected FAILED_PRECONDITION, got {err:?}"
    );
}

#[then(expr = "the rejection reason should be {string}")]
async fn then_reason(world: &mut SpeculativeClientWorld, reason: String) {
    let status = world.command_err().status().expect("server status");
    assert_eq!(status.message(), reason);
}

#[then("the operation should fail with validation error")]
async fn then_validation(world: &mut SpeculativeClientWorld) {
    let err = world.command_err();
    assert!(
        err.is_invalid_argument(),
        "expected INVALID_ARGUMENT, got {err:?}"
    );
}

#[then("no events should be produced")]
async fn then_none_produced(world: &mut SpeculativeClientWorld) {
    assert!(world.command.as_ref().expect("executed").is_err());
    assert_eq!(world.backend().total_events(), world.stored_before);
}

#[then("the projected execution leaves no trace")]
async fn then_no_trace(world: &mut SpeculativeClientWorld) {
    assert_eq!(world.emitted_seqs(), vec![5]);
    assert_eq!(world.backend().stored_pages(&world.cover()).len(), 5);
    assert_eq!(world.backend().total_events(), world.stored_before);
}

#[then("the response should contain the projection")]
async fn then_projection(world: &mut SpeculativeClientWorld) {
    let p = world
        .projection
        .as_ref()
        .expect("projector executed")
        .as_ref()
        .expect("projection succeeds");
    assert_eq!(p.projector, "order-summary");
    assert!(p.projection.is_some());
    assert_eq!(p.cover, world.events.as_ref().expect("events").cover);
}

#[then("no external systems should be updated")]
async fn then_no_external(world: &mut SpeculativeClientWorld) {
    assert!(world.backend().projector_runs().is_empty());
    assert_eq!(world.backend().total_events(), world.stored_before);
}

#[then(expr = "the projector should process all {int} events in order")]
async fn then_projector_order(world: &mut SpeculativeClientWorld, n: u32) {
    let p = world
        .projection
        .as_ref()
        .expect("projector executed")
        .as_ref()
        .expect("projection succeeds");
    let any = p.projection.as_ref().expect("projection payload");
    let seen = GenericEvent::decode(any.value.as_slice())
        .expect("decodes")
        .data;
    let expected: Vec<String> = (0..n).map(|i| i.to_string()).collect();
    assert_eq!(seen, expected.join(","));
}

#[then("the final projection state should be returned")]
async fn then_final_state(world: &mut SpeculativeClientWorld) {
    let p = world
        .projection
        .as_ref()
        .expect("projector executed")
        .as_ref()
        .expect("projection succeeds");
    let last = world
        .events
        .as_ref()
        .expect("events")
        .pages
        .last()
        .and_then(page_seq);
    assert_eq!(Some(p.sequence), last);
}

#[then("the response should contain the commands the saga would emit")]
async fn then_saga_commands(world: &mut SpeculativeClientWorld) {
    let r = world
        .saga
        .as_ref()
        .expect("saga executed")
        .as_ref()
        .expect("saga succeeds");
    let n = world.events.as_ref().expect("events").pages.len();
    assert_eq!(r.commands.len(), n);
    for c in &r.commands {
        assert_eq!(c.cover.as_ref().expect("cover").domain, "inventory");
    }
}

#[then("the commands should NOT be sent to the target domain")]
async fn then_saga_not_sent(world: &mut SpeculativeClientWorld) {
    assert_eq!(world.backend().total_events(), world.stored_before);
    assert!(!world.backend().rpcs().contains(&"HandleCommand"));
}

#[then("the response should preserve the saga origin chain")]
async fn then_origin(world: &mut SpeculativeClientWorld) {
    let r = world
        .saga
        .as_ref()
        .expect("saga executed")
        .as_ref()
        .expect("saga succeeds");
    let origin = world.cover();
    assert!(!r.commands.is_empty());
    for (i, c) in r.commands.iter().enumerate() {
        match c.pages[0]
            .header
            .as_ref()
            .and_then(|h| h.sequence_type.as_ref())
        {
            Some(SequenceType::AngzarrDeferred(d)) => {
                assert_eq!(d.source.as_ref(), Some(&origin));
                assert_eq!(d.source_seq, i as u32);
            }
            other => panic!("expected angzarr_deferred provenance, got {other:?}"),
        }
    }
}

#[then("the response should contain the PM's command decisions")]
async fn then_pm_commands(world: &mut SpeculativeClientWorld) {
    let r = world
        .pm
        .as_ref()
        .expect("pm executed")
        .as_ref()
        .expect("pm succeeds");
    assert_eq!(
        r.commands.len(),
        world.events.as_ref().expect("events").pages.len()
    );
}

#[then("the commands should NOT be executed")]
async fn then_pm_not_executed(world: &mut SpeculativeClientWorld) {
    assert_eq!(world.backend().total_events(), world.stored_before);
    assert!(!world.backend().rpcs().contains(&"HandleCommand"));
}

#[then("the speculative PM operation should fail")]
async fn then_pm_fails(world: &mut SpeculativeClientWorld) {
    let err = world
        .pm
        .as_ref()
        .expect("pm executed")
        .as_ref()
        .expect_err("pm must fail");
    assert!(err.is_invalid_argument(), "got {err:?}");
}

#[then("the error should indicate missing correlation ID")]
async fn then_missing_correlation(world: &mut SpeculativeClientWorld) {
    let err = world
        .pm
        .as_ref()
        .expect("pm executed")
        .as_ref()
        .expect_err("pm must fail");
    let msg = err.status().expect("server status").message();
    assert!(msg.contains("correlation_id"), "message: {msg}");
}

#[then(expr = "I should receive only {int} events")]
async fn then_only_n(world: &mut SpeculativeClientWorld, n: usize) {
    assert_eq!(
        world.real.as_ref().expect("real events read").pages.len(),
        n
    );
}

#[then("the speculative events should not be present")]
async fn then_spec_absent(world: &mut SpeculativeClientWorld) {
    let speculative = data_of(world.command_ok().events.as_ref().expect("events"));
    assert_eq!(speculative.len(), 2);
    let real = data_of(world.real.as_ref().expect("real events read"));
    assert!(
        real.iter().all(|d| !speculative.contains(d)),
        "real: {real:?}"
    );
}

#[then("each speculation should start from the same base state")]
async fn then_same_base(world: &mut SpeculativeClientWorld) {
    let firsts: Vec<Option<u32>> = world
        .commands_ab
        .iter()
        .map(|r| {
            r.events
                .as_ref()
                .and_then(|b| b.pages.first())
                .and_then(page_seq)
        })
        .collect();
    assert_eq!(firsts, vec![Some(3), Some(3)]);
}

#[then("results should be independent")]
async fn then_independent(world: &mut SpeculativeClientWorld) {
    let data: Vec<Vec<String>> = world
        .commands_ab
        .iter()
        .map(|r| data_of(r.events.as_ref().expect("events")))
        .collect();
    assert_eq!(data, vec![vec!["A".to_string()], vec!["B".to_string()]]);
    assert_eq!(world.backend().total_events(), world.stored_before);
}

#[then("the speculative operation should fail with connection error")]
async fn then_connection_error(world: &mut SpeculativeClientWorld) {
    let err = world.error.as_ref().expect("operation failed");
    assert!(
        err.is_connection_error(),
        "expected connection error, got {err:?}"
    );
}

#[then("the speculative operation should fail with invalid argument error")]
async fn then_invalid_argument(world: &mut SpeculativeClientWorld) {
    let err = world.error.as_ref().expect("operation failed");
    assert!(
        err.is_invalid_argument(),
        "expected INVALID_ARGUMENT, got {err:?}"
    );
}
