//! Step definitions for `features/client/projector.feature`.
//!
//! The Output projector is a real `#[projector]` type dispatched through a
//! `ProjectorRouter`. Each instance gets a distinct id from the factory and
//! appends `(instance id, order id)` to a shared write log.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use angzarr_client::proto::{event_page, Cover, EventBook, EventPage, PageHeader};
use angzarr_client::router::ProjectorRouter;
use angzarr_client::router::{Built, Router};
use angzarr_client::{projector, CommandResult};
use cucumber::{given, then, when, World};
use prost_types::Any;

use super::deferred::pack;
use crate::common::fixtures::{OrderCompleted, OrderCreated};

type WriteLog = Arc<Mutex<Vec<(u32, String)>>>;

pub struct Output {
    instance: u32,
    log: WriteLog,
}

#[projector(name = "Output", domains = ["order"])]
impl Output {
    #[handles(OrderCreated)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<()> {
        self.log
            .lock()
            .unwrap()
            .push((self.instance, event.order_id));
        Ok(())
    }
}

#[derive(Debug, Default, World)]
pub struct ProjectorWorld {
    log: WriteLog,
    instances: Arc<AtomicU32>,
}

impl ProjectorWorld {
    fn router(&self) -> ProjectorRouter {
        let log = Arc::clone(&self.log);
        let instances = Arc::clone(&self.instances);
        let built = Router::new("projectors")
            .with_handler(move || Output {
                instance: instances.fetch_add(1, Ordering::SeqCst),
                log: Arc::clone(&log),
            })
            .build()
            .expect("router builds");
        match built {
            Built::Projector(r) => r,
            other => panic!("expected a projector router, got {other:?}"),
        }
    }

    fn dispatch(&self, domain: &str, events: Vec<Any>) {
        let pages: Vec<EventPage> = events
            .into_iter()
            .enumerate()
            .map(|(i, any)| EventPage {
                header: Some(PageHeader {
                    sequence_type: Some(
                        angzarr_client::proto::page_header::SequenceType::Sequence(i as u32),
                    ),
                    sync_mode: None,
                }),
                payload: Some(event_page::Payload::Event(any)),
                ..Default::default()
            })
            .collect();
        let book = EventBook {
            cover: Some(Cover {
                domain: domain.into(),
                ..Default::default()
            }),
            next_sequence: pages.len() as u32,
            pages,
            ..Default::default()
        };
        let projection = self.router().dispatch(book).expect("projector dispatch");
        assert_eq!(projection.cover.map(|c| c.domain), Some(domain.to_string()));
    }

    fn created(n: usize) -> Vec<Any> {
        (0..n)
            .map(|i| {
                pack(&OrderCreated {
                    order_id: format!("o-{i}"),
                    ..Default::default()
                })
            })
            .collect()
    }

    fn entries(&self) -> Vec<(u32, String)> {
        self.log.lock().unwrap().clone()
    }
}

// --- Given -----------------------------------------------------------------

#[given(expr = "a projector {string} consuming domains {string}")]
fn given_projector(_world: &mut ProjectorWorld, name: String, domain: String) {
    let config = <Output as angzarr_client::router::HandlerKind>::handler_config();
    let angzarr_client::router::HandlerConfig::Projector {
        name: n, domains, ..
    } = config
    else {
        panic!("not a projector config");
    };
    assert_eq!(n, name);
    assert_eq!(domains, vec![domain]);
}

#[given("the projector handles OrderCreated by appending to a write log")]
fn given_handles(_world: &mut ProjectorWorld) {
    // `#[handles(OrderCreated)]` on Output appends to the shared write log.
}

#[given("Output is the active projector")]
fn given_active(world: &mut ProjectorWorld) {
    let router = world.router();
    assert_eq!(router.name(), "Output");
    assert_eq!(router.handler_count(), 1);
}

// --- When ------------------------------------------------------------------

#[when("an EventBook with three OrderCreated events is dispatched")]
fn when_three(world: &mut ProjectorWorld) {
    world.dispatch("order", ProjectorWorld::created(3));
}

#[when("an EventBook with five OrderCreated events is dispatched")]
fn when_five(world: &mut ProjectorWorld) {
    world.dispatch("order", ProjectorWorld::created(5));
}

#[when("an EventBook mixing OrderCreated and OrderCompleted is dispatched")]
fn when_mixed(world: &mut ProjectorWorld) {
    world.dispatch(
        "order",
        vec![
            pack(&OrderCreated {
                order_id: "o-0".into(),
                ..Default::default()
            }),
            pack(&OrderCompleted {
                order_id: "o-0".into(),
                ..Default::default()
            }),
            pack(&OrderCreated {
                order_id: "o-1".into(),
                ..Default::default()
            }),
        ],
    );
}

#[when(expr = "an EventBook in domain {string} is dispatched")]
fn when_domain(world: &mut ProjectorWorld, domain: String) {
    world.dispatch(&domain, ProjectorWorld::created(1));
}

// --- Then ------------------------------------------------------------------

#[then(expr = "the write log contains {int} entries")]
fn then_count(world: &mut ProjectorWorld, n: usize) {
    let entries = world.entries();
    assert_eq!(entries.len(), n);
    let ids: Vec<String> = entries.into_iter().map(|(_, id)| id).collect();
    let expected: Vec<String> = (0..n).map(|i| format!("o-{i}")).collect();
    assert_eq!(ids, expected, "events must be projected in page order");
}

#[then("the write log contains only OrderCreated entries")]
fn then_only_created(world: &mut ProjectorWorld) {
    let ids: Vec<String> = world.entries().into_iter().map(|(_, id)| id).collect();
    assert_eq!(ids, vec!["o-0".to_string(), "o-1".to_string()]);
}

#[then("the write log remains empty")]
fn then_empty(world: &mut ProjectorWorld) {
    assert!(world.entries().is_empty());
}

#[then("every entry was appended by the same projector instance")]
fn then_same_instance(world: &mut ProjectorWorld) {
    let entries = world.entries();
    assert!(!entries.is_empty());
    let first = entries[0].0;
    assert!(
        entries.iter().all(|(i, _)| *i == first),
        "entries: {entries:?}"
    );
}
