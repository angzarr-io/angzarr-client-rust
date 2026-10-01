//! Handler context and state rebuild through the angzarr-router engine:
//! saga / process-manager handlers receive the optional `destinations`
//! and `source_cover` parameters by name, and aggregate state starts from
//! the EventBook snapshot when the state type is a protobuf message.

use std::sync::{Arc, Mutex};

use angzarr_client::proto::{
    command_page, event_page, page_header::SequenceType, CommandBook, CommandPage,
    ContextualCommand, Cover, EventBook, EventPage, PageHeader, ProcessManagerHandleRequest,
    ProcessManagerHandleResponse, SagaHandleRequest, SagaResponse, Snapshot,
};
use angzarr_client::router::{Built, Router};
#[allow(unused_imports)]
use angzarr_client::{
    applies, command_handler, full_type_url, handles, process_manager, saga, CommandResult,
    Destinations,
};
use prost_types::Any;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Tick {
    #[prost(uint32, tag = "1")]
    n: u32,
}
impl ::prost::Name for Tick {
    const NAME: &'static str = "Tick";
    const PACKAGE: &'static str = "context";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Count {
    #[prost(uint32, tag = "1")]
    count: u32,
}
impl ::prost::Name for Count {
    const NAME: &'static str = "Count";
    const PACKAGE: &'static str = "context";
}

type Seen = Arc<Mutex<Vec<(Vec<String>, Option<Cover>)>>>;

fn record(seen: &Seen, destinations: &Destinations, source_cover: Option<Cover>) {
    seen.lock()
        .unwrap()
        .push((destinations.domains().to_vec(), source_cover));
}

struct ContextSaga(Seen);

#[saga(name = "ctx-saga", source = "clock", target = "audit")]
impl ContextSaga {
    #[handles(Tick)]
    fn on_tick(
        &self,
        tick: Tick,
        destinations: &Destinations,
        source_cover: Option<Cover>,
        source_seq: u32,
    ) -> CommandResult<SagaResponse> {
        assert_eq!(
            source_seq, tick.n,
            "source_seq is the triggering page's sequence"
        );
        record(&self.0, destinations, source_cover);
        Ok(SagaResponse::default())
    }
}

#[derive(Default)]
struct NoState;

struct ContextPm(Seen);

#[process_manager(
    name = "ctx-pm",
    pm_domain = "ctx",
    state = NoState,
    sources = ["clock"],
    targets = ["audit", "billing"]
)]
impl ContextPm {
    #[handles(Tick)]
    fn on_tick(
        &self,
        _tick: Tick,
        _state: &NoState,
        source_cover: Option<Cover>,
        destinations: &Destinations,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        record(&self.0, destinations, source_cover);
        Ok(ProcessManagerHandleResponse::default())
    }
}

fn tick_page(seq: u32) -> EventPage {
    EventPage {
        header: Some(PageHeader {
            sequence_type: Some(SequenceType::Sequence(seq)),
            sync_mode: None,
        }),
        payload: Some(event_page::Payload::Event(Any {
            type_url: full_type_url::<Tick>(),
            value: ::prost::Message::encode_to_vec(&Tick { n: seq }),
        })),
        ..Default::default()
    }
}

fn clock_cover() -> Cover {
    Cover {
        domain: "clock".into(),
        correlation_id: "corr-ctx".into(),
        ..Default::default()
    }
}

#[test]
fn saga_handler_receives_destinations_and_source_cover() {
    let seen: Seen = Arc::default();
    let s = Arc::clone(&seen);
    let Ok(Built::Saga(router)) = Router::new("sagas")
        .with_handler(move || ContextSaga(Arc::clone(&s)))
        .build()
    else {
        panic!("expected a saga router");
    };
    router
        .dispatch(SagaHandleRequest {
            source: Some(EventBook {
                cover: Some(clock_cover()),
                pages: vec![tick_page(3)],
                next_sequence: 4,
                ..Default::default()
            }),
            ..Default::default()
        })
        .expect("saga dispatch");
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(vec!["audit".to_string()], Some(clock_cover()))]
    );
}

#[test]
fn process_manager_handler_receives_destinations_and_source_cover() {
    let seen: Seen = Arc::default();
    let s = Arc::clone(&seen);
    let Ok(Built::ProcessManager(router)) = Router::new("pms")
        .with_handler(move || ContextPm(Arc::clone(&s)))
        .build()
    else {
        panic!("expected a PM router");
    };
    router
        .dispatch(ProcessManagerHandleRequest {
            trigger: Some(EventBook {
                cover: Some(clock_cover()),
                pages: vec![tick_page(2)],
                next_sequence: 3,
                ..Default::default()
            }),
            ..Default::default()
        })
        .expect("PM dispatch");
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(
            vec!["audit".to_string(), "billing".to_string()],
            Some(clock_cover())
        )]
    );
}

struct Counter(Arc<Mutex<Option<u32>>>);

#[command_handler(domain = "counter", state = Count)]
impl Counter {
    #[applies(Tick)]
    fn on_tick(state: &mut Count, _tick: Tick) {
        state.count += 1;
    }

    #[handles(Tick)]
    fn observe(&self, _cmd: Tick, state: &Count, _seq: u32) -> CommandResult<EventBook> {
        *self.0.lock().unwrap() = Some(state.count);
        Ok(EventBook::default())
    }
}

/// State starts from the snapshot; pages it already covers are not
/// re-applied, later pages are.
#[test]
fn aggregate_state_starts_from_the_snapshot() {
    let observed = Arc::new(Mutex::new(None));
    let o = Arc::clone(&observed);
    let Ok(Built::CommandHandler(router)) = Router::new("counter")
        .with_handler(move || Counter(Arc::clone(&o)))
        .build()
    else {
        panic!("expected a command-handler router");
    };
    let prior = EventBook {
        cover: Some(Cover {
            domain: "counter".into(),
            ..Default::default()
        }),
        snapshot: Some(Snapshot {
            sequence: 4,
            state: Some(Any {
                type_url: full_type_url::<Count>(),
                value: ::prost::Message::encode_to_vec(&Count { count: 5 }),
            }),
            ..Default::default()
        }),
        pages: vec![tick_page(4), tick_page(5)],
        next_sequence: 6,
    };
    router
        .dispatch(ContextualCommand {
            command: Some(CommandBook {
                cover: Some(Cover {
                    domain: "counter".into(),
                    ..Default::default()
                }),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(Any {
                        type_url: full_type_url::<Tick>(),
                        value: ::prost::Message::encode_to_vec(&Tick { n: 9 }),
                    })),
                    ..Default::default()
                }],
            }),
            events: Some(prior),
        })
        .expect("dispatch");
    assert_eq!(*observed.lock().unwrap(), Some(6));
}

#[test]
fn unknown_context_parameter_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/saga_unknown_context_parameter.rs");
}
