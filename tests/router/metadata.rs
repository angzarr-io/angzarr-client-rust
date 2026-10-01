//! Runtime-router metadata and pass-through: names, output domains, sync
//! targets, handler counts, and what the projector / upcaster routers
//! return.

use std::sync::{Arc, Mutex};

use angzarr_client::proto::{
    event_page, Cover, EventBook, EventPage, ProcessManagerHandleResponse, SagaResponse,
    UpcastRequest,
};
use angzarr_client::router::{Built, Router};
#[allow(unused_imports)]
use angzarr_client::{
    full_type_url, handles, process_manager, projector, saga, upcaster, upcasts, CommandResult,
};
use prost_types::Any;

macro_rules! msg {
    ($name:ident) => {
        #[derive(Clone, PartialEq, ::prost::Message)]
        struct $name {}
        impl ::prost::Name for $name {
            const NAME: &'static str = stringify!($name);
            const PACKAGE: &'static str = "metadata";
        }
    };
}
msg!(Ping);
msg!(PingV2);

#[derive(Default)]
struct NoState;

struct SyncSaga;
#[saga(name = "s-sync", source = "a", target = "inventory", sync = true)]
impl SyncSaga {
    #[handles(Ping)]
    fn on(&self, _e: Ping) -> CommandResult<SagaResponse> {
        Ok(SagaResponse::default())
    }
}

struct AsyncSaga;
#[saga(name = "s-async", source = "a", target = "shipping")]
impl AsyncSaga {
    #[handles(Ping)]
    fn on(&self, _e: Ping) -> CommandResult<SagaResponse> {
        Ok(SagaResponse::default())
    }
}

#[test]
fn saga_router_metadata() {
    let Ok(Built::Saga(r)) = Router::new("sagas")
        .with_handler(|| SyncSaga)
        .with_handler(|| AsyncSaga)
        .with_handler(|| SyncSaga)
        .build()
    else {
        panic!("saga router");
    };
    assert_eq!(r.name(), "s-sync");
    assert_eq!(r.output_domains(), vec!["inventory", "shipping"]);
    assert_eq!(r.sync_output_domains(), vec!["inventory"]);
    assert!(r.has_async_outputs());
    assert_eq!(r.handler_count(), 3);

    let Ok(Built::Saga(only_sync)) = Router::new("sagas").with_handler(|| SyncSaga).build() else {
        panic!("saga router");
    };
    assert!(!only_sync.has_async_outputs());
    assert_eq!(only_sync.handler_count(), 1);
}

struct Pm;
#[process_manager(
    name = "flow",
    pm_domain = "flow",
    state = NoState,
    sources = ["a"],
    targets = ["inventory", "shipping"],
    sync_targets = ["inventory"]
)]
impl Pm {
    #[handles(Ping)]
    fn on(&self, _e: Ping, _s: &NoState) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
}

struct SyncPm;
#[process_manager(
    name = "sync-flow",
    pm_domain = "sync-flow",
    state = NoState,
    sources = ["a"],
    targets = ["inventory"],
    sync_targets = ["inventory"]
)]
impl SyncPm {
    #[handles(Ping)]
    fn on(&self, _e: Ping, _s: &NoState) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
}

#[test]
fn process_manager_router_metadata() {
    let Ok(Built::ProcessManager(r)) = Router::new("pms")
        .with_handler(|| Pm)
        .with_handler(|| SyncPm)
        .build()
    else {
        panic!("PM router");
    };
    assert_eq!(r.name(), "flow");
    assert_eq!(r.output_domains(), vec!["inventory", "shipping"]);
    assert_eq!(r.sync_output_domains(), vec!["inventory"]);
    assert!(r.has_async_outputs());
    assert_eq!(r.handler_count(), 2);

    let Ok(Built::ProcessManager(sync_only)) = Router::new("pms").with_handler(|| SyncPm).build()
    else {
        panic!("PM router");
    };
    assert!(!sync_only.has_async_outputs());
}

struct Proj(Arc<Mutex<u32>>);
#[projector(name = "proj", domains = ["a"])]
impl Proj {
    #[handles(Ping)]
    fn on(&self, _e: Ping) -> CommandResult<()> {
        *self.0.lock().unwrap() += 1;
        Ok(())
    }
}

#[test]
fn projector_router_runs_projectors_and_echoes_the_book() {
    let count = Arc::new(Mutex::new(0));
    let c = Arc::clone(&count);
    let Ok(Built::Projector(r)) = Router::new("projectors")
        .with_handler(move || Proj(Arc::clone(&c)))
        .build()
    else {
        panic!("projector router");
    };
    assert_eq!(r.name(), "proj");
    assert!(r.output_domains().is_empty());
    assert_eq!(r.handler_count(), 1);
    let c2 = Arc::clone(&count);
    let c3 = Arc::clone(&count);
    let Ok(Built::Projector(two)) = Router::new("projectors")
        .with_handler(move || Proj(Arc::clone(&c2)))
        .with_handler(move || Proj(Arc::clone(&c3)))
        .build()
    else {
        panic!("projector router");
    };
    assert_eq!(two.handler_count(), 2);
    let cover = Cover {
        domain: "a".into(),
        ..Default::default()
    };
    let projection = r
        .dispatch(EventBook {
            cover: Some(cover.clone()),
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(Any {
                    type_url: full_type_url::<Ping>(),
                    value: vec![],
                })),
                ..Default::default()
            }],
            next_sequence: 7,
            ..Default::default()
        })
        .expect("projector dispatch");
    assert_eq!(projection.cover, Some(cover));
    assert_eq!(projection.sequence, 7);
    assert_eq!(*count.lock().unwrap(), 1);
}

struct Up;
#[upcaster(name = "up", domain = "a")]
impl Up {
    #[upcasts(from = Ping, to = PingV2)]
    fn up(_old: Ping) -> PingV2 {
        PingV2 {}
    }
}

#[test]
fn upcaster_router_metadata_and_dispatch() {
    let Ok(Built::Upcaster(two)) = Router::new("ups")
        .with_handler(|| Up)
        .with_handler(|| Up)
        .build()
    else {
        panic!("upcaster router");
    };
    assert_eq!(two.handler_count(), 2);
    let Ok(Built::Upcaster(r)) = Router::new("ups").with_handler(|| Up).build() else {
        panic!("upcaster router");
    };
    assert_eq!(r.name(), "up");
    assert!(r.output_domains().is_empty());
    assert_eq!(r.handler_count(), 1);
    let out = r
        .dispatch(UpcastRequest {
            domain: "a".into(),
            events: vec![EventPage {
                payload: Some(event_page::Payload::Event(Any {
                    type_url: full_type_url::<Ping>(),
                    value: vec![],
                })),
                ..Default::default()
            }],
        })
        .expect("upcast");
    let Some(event_page::Payload::Event(any)) = &out.events[0].payload else {
        panic!("event");
    };
    assert_eq!(any.type_url, full_type_url::<PingV2>());
}
