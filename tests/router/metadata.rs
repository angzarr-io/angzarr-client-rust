//! Runtime-router metadata and pass-through: names, output domains,
//! handler counts, and what the projector / upcaster routers
//! return.

use std::sync::{Arc, Mutex};

use angzarr_client::proto::{
    Cover, EventBook, EventPage, ProcessManagerHandleResponse, SagaResponse, UpcastRequest,
};
#[allow(unused_imports)]
use angzarr_client::router::{
    command_handler, handles, process_manager, projector, saga, upcaster, upcasts,
};
use angzarr_client::router::{Built, Router};
#[allow(unused_imports)]
use angzarr_client::{full_type_url, CommandResult};
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
#[saga(name = "s-sync", source = "a", target = "inventory")]
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
    let Ok(Built::Saga(r)) = Router::new("sagas").with_handler(|| AsyncSaga).build() else {
        panic!("saga router");
    };
    assert_eq!(r.name(), "s-async");
    assert_eq!(r.output_domains(), vec!["shipping"]);

    let Ok(Built::Saga(only_sync)) = Router::new("sagas").with_handler(|| SyncSaga).build() else {
        panic!("saga router");
    };
    assert_eq!(only_sync.name(), "s-sync");
    assert_eq!(only_sync.output_domains(), vec!["inventory"]);
}

struct Pm;
#[process_manager(
    name = "flow",
    pm_domain = "flow",
    state = NoState,
    sources = ["a"],
    targets = ["inventory", "shipping"]
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
    targets = ["inventory"]
)]
impl SyncPm {
    #[handles(Ping)]
    fn on(&self, _e: Ping, _s: &NoState) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
}

#[test]
fn process_manager_router_metadata() {
    let Ok(Built::ProcessManager(r)) = Router::new("pms").with_handler(|| Pm).build() else {
        panic!("PM router");
    };
    assert_eq!(r.name(), "flow");
    assert_eq!(r.output_domains(), vec!["inventory", "shipping"]);

    let Ok(Built::ProcessManager(sync_only)) = Router::new("pms").with_handler(|| SyncPm).build()
    else {
        panic!("PM router");
    };
    assert_eq!(sync_only.output_domains(), vec!["inventory"]);
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
    let cover = Cover {
        domain: "a".into(),
        ..Default::default()
    };
    let projection = r
        .dispatch(EventBook {
            cover: Some(cover.clone()),
            pages: vec![EventPage {
                payload: Some(Any {
                    type_url: full_type_url::<Ping>(),
                    value: vec![],
                }),
                sequence: 6,
                ..Default::default()
            }],
            next_sequence: 7,
            ..Default::default()
        })
        .expect("projector dispatch");
    assert_eq!(projection.cover, Some(cover.clone()));
    assert_eq!(projection.projector, "proj");
    assert_eq!(projection.next_sequence, 7);
    assert_eq!(*count.lock().unwrap(), 1);

    // A book with no pages answers its own next_sequence.
    let empty = EventBook {
        cover: Some(cover.clone()),
        next_sequence: 4,
        ..Default::default()
    };
    let projection = r.dispatch(empty).expect("projector dispatch");
    assert_eq!(projection.cover, Some(cover));
    assert_eq!(projection.projector, "proj");
    assert_eq!(projection.next_sequence, 4);
}

struct Ledger;
#[command_handler(domain = "a", state = NoState)]
impl Ledger {
    #[handles(Ping)]
    fn on(&self, _c: Ping, _s: &NoState, _seq: u64) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

struct Up;
#[upcaster(name = "up", domain = "a")]
impl Up {
    #[upcasts(from = Ping, to = PingV2)]
    fn up(_old: Ping) -> PingV2 {
        PingV2 {}
    }
}

fn ping_page() -> EventPage {
    EventPage {
        payload: Some(Any {
            type_url: full_type_url::<Ping>(),
            value: vec![],
        }),
        ..Default::default()
    }
}

/// An upcaster chain is part of its component: the component's router
/// upcasts its domain's events, and passes other domains' through.
#[test]
fn a_components_upcaster_chain_upcasts_its_domain() {
    let Ok(Built::CommandHandler(r)) = Router::new("a")
        .with_handler(|| Ledger)
        .with_upcaster(|| Up)
        .build()
    else {
        panic!("command-handler router");
    };
    assert!(r.has_upcaster());
    let out = r
        .upcast(UpcastRequest {
            domain: "a".into(),
            events: vec![ping_page()],
        })
        .expect("upcast");
    let Some(any) = &out.events[0].payload else {
        panic!("event");
    };
    assert_eq!(any.type_url, full_type_url::<PingV2>());
    let other = r
        .upcast(UpcastRequest {
            domain: "b".into(),
            events: vec![ping_page()],
        })
        .expect("pass through");
    assert_eq!(other.events, vec![ping_page()]);

    let Ok(Built::CommandHandler(plain)) = Router::new("a").with_handler(|| Ledger).build() else {
        panic!("command-handler router");
    };
    assert!(!plain.has_upcaster());
}

struct AnyDomain(Arc<Mutex<u32>>);
#[projector(name = "any", domains = ["*"])]
impl AnyDomain {
    #[handles(Ping)]
    fn on(&self, _e: Ping) -> CommandResult<()> {
        *self.0.lock().unwrap() += 1;
        Ok(())
    }
}

#[test]
fn a_wildcard_projector_consumes_every_domain() {
    let count = Arc::new(Mutex::new(0));
    let c = Arc::clone(&count);
    let Ok(Built::Projector(r)) = Router::new("projectors")
        .with_handler(move || AnyDomain(Arc::clone(&c)))
        .build()
    else {
        panic!("projector router");
    };
    for domain in ["a", "z"] {
        r.dispatch(EventBook {
            cover: Some(Cover {
                domain: domain.into(),
                ..Default::default()
            }),
            pages: vec![EventPage {
                payload: Some(Any {
                    type_url: full_type_url::<Ping>(),
                    value: vec![],
                }),
                ..Default::default()
            }],
            ..Default::default()
        })
        .expect("projector dispatch");
    }
    assert_eq!(*count.lock().unwrap(), 2);
}

#[test]
fn a_router_debug_names_it_and_its_component_kind() {
    let router = Router::new("projectors").with_handler(|| AnyDomain(Arc::new(Mutex::new(0))));
    let shown = format!("{router:?}");
    assert!(
        shown.starts_with(
            r#"Router { name: "projectors", component: OneComponent { kind: Projector"#
        ),
        "{shown}"
    );
}
