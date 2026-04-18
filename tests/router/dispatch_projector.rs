//! R13 — projector dispatch.
//!
//! Projector instances are constructed ONCE per dispatch (so they can batch
//! writes across all events in the incoming EventBook). Each event page is
//! then routed to the matching `#[handles]` method on the same instance.
//! `ProjectorRouter::dispatch` does not merge handler outputs — projectors
//! operate via side effects. Multi-handler fan-out still runs each registered
//! projector instance.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use angzarr_client::proto::{event_page, EventBook, EventPage};
use angzarr_client::router::{Built, Router};
use angzarr_client::{full_type_url, projector, CommandResult};

use prost::Message;
use prost_types::Any;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Tick {}
impl ::prost::Name for Tick {
    const NAME: &'static str = "Tick";
    const PACKAGE: &'static str = "meter";
}

/// Projector whose handler increments an Arc<AtomicU32> for each event seen.
/// The counter is captured at factory construction and shared across the
/// reused instance for a single dispatch.
struct CounterProjector {
    seen: Arc<AtomicU32>,
}

#[projector(name = "prj-counter", domains = ["meter"])]
impl CounterProjector {
    #[handles(Tick)]
    #[allow(unused_variables, dead_code)]
    fn on_tick(&self, event: Tick) -> CommandResult<()> {
        self.seen.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn book_of_n_ticks(n: usize) -> EventBook {
    let page = || EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: full_type_url::<Tick>(),
            value: Tick {}.encode_to_vec(),
        })),
        ..Default::default()
    };
    EventBook {
        pages: (0..n).map(|_| page()).collect(),
        ..Default::default()
    }
}

#[test]
fn projector_handler_runs_once_per_event_in_book() {
    let seen = Arc::new(AtomicU32::new(0));
    let seen_clone = Arc::clone(&seen);
    let built = Router::new("prj-counter")
        .with_handler(move || CounterProjector {
            seen: Arc::clone(&seen_clone),
        })
        .build()
        .unwrap();
    let Built::Projector(router) = built else {
        panic!("expected Projector variant");
    };
    let _ = router.dispatch(book_of_n_ticks(4)).unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 4);
}

#[test]
fn projector_factory_invoked_once_per_dispatch_not_per_event() {
    // The projector instance must be reused across all events in the book
    // so that batching-style projectors (e.g. DB connection checkout at
    // instance construction, one commit at end) behave correctly.
    let instances = Arc::new(AtomicU32::new(0));
    let seen = Arc::new(AtomicU32::new(0));
    let instances_clone = Arc::clone(&instances);
    let seen_clone = Arc::clone(&seen);

    let built = Router::new("prj-counter")
        .with_handler(move || {
            instances_clone.fetch_add(1, Ordering::SeqCst);
            CounterProjector {
                seen: Arc::clone(&seen_clone),
            }
        })
        .build()
        .unwrap();
    let Built::Projector(router) = built else {
        panic!("expected Projector variant");
    };
    let _ = router.dispatch(book_of_n_ticks(5)).unwrap();

    // The runtime calls the factory once to peek at config (kind/handled),
    // and potentially a second time to produce the instance used for the
    // actual event loop — total is at most 2, importantly NOT 5.
    let ic = instances.load(Ordering::SeqCst);
    assert!(
        ic <= 2,
        "factory must be invoked ≤ 2× per dispatch (config peek + instance), was {}",
        ic
    );
    assert_eq!(seen.load(Ordering::SeqCst), 5);
}

// Multi-handler fan-out: every registered projector runs.
struct SecondProjector {
    seen: Arc<AtomicU32>,
}

#[projector(name = "prj-second", domains = ["meter"])]
impl SecondProjector {
    #[handles(Tick)]
    #[allow(unused_variables, dead_code)]
    fn on_tick(&self, event: Tick) -> CommandResult<()> {
        self.seen.fetch_add(100, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn projector_multi_handler_all_run_no_merge() {
    let seen = Arc::new(AtomicU32::new(0));
    let seen_a = Arc::clone(&seen);
    let seen_b = Arc::clone(&seen);

    let built = Router::new("prj-fanout")
        .with_handler(move || CounterProjector {
            seen: Arc::clone(&seen_a),
        })
        .with_handler(move || SecondProjector {
            seen: Arc::clone(&seen_b),
        })
        .build()
        .unwrap();
    let Built::Projector(router) = built else {
        panic!("expected Projector variant");
    };
    let _ = router.dispatch(book_of_n_ticks(2)).unwrap();
    // CounterProjector adds +1 per event (×2 events = 2),
    // SecondProjector adds +100 per event (×2 = 200). Total: 202.
    assert_eq!(seen.load(Ordering::SeqCst), 202);
}
