//! R7 — state rebuild via `#[applies]` and `#[state_factory]`.
//!
//! Prior events in the incoming `ContextualCommand.events` must replay
//! through the aggregate instance's `#[applies]` methods before the
//! matching `#[handles]` method runs. A declared `#[state_factory]`
//! method overrides `Default::default()` as the initial state constructor.

use angzarr_client::proto::{
    business_response, command_page, event_page, BusinessResponse, CommandBook, CommandPage,
    ContextualCommand, EventBook, EventPage,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{aggregate, full_type_url, CommandResult};

use prost::Message;
use prost_types::Any;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Tick {}
impl ::prost::Name for Tick {
    const NAME: &'static str = "Tick";
    const PACKAGE: &'static str = "test";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Report {}
impl ::prost::Name for Report {
    const NAME: &'static str = "Report";
    const PACKAGE: &'static str = "test";
}

#[derive(Default)]
struct Counter {
    value: u32,
}

struct CounterAgg;

#[aggregate(domain = "counter", state = Counter)]
impl CounterAgg {
    #[applies(Tick)]
    #[allow(unused_variables, dead_code)]
    fn on_tick(state: &mut Counter, evt: Tick) {
        state.value += 1;
    }

    #[handles(Report)]
    #[allow(unused_variables, dead_code)]
    fn report(&self, cmd: Report, state: &Counter, seq: u32) -> CommandResult<EventBook> {
        // Emit `state.value` empty pages so the test can count them.
        Ok(EventBook {
            pages: vec![EventPage::default(); state.value as usize],
            ..Default::default()
        })
    }
}

fn tick_event_page() -> EventPage {
    EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: full_type_url::<Tick>(),
            value: Tick {}.encode_to_vec(),
        })),
        ..Default::default()
    }
}

fn contextual_report_with_prior_ticks(n: usize) -> ContextualCommand {
    let cmd = Any {
        type_url: full_type_url::<Report>(),
        value: Report {}.encode_to_vec(),
    };
    let cmd_book = CommandBook {
        pages: vec![CommandPage {
            payload: Some(command_page::Payload::Command(cmd)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let prior = EventBook {
        pages: (0..n).map(|_| tick_event_page()).collect(),
        ..Default::default()
    };
    ContextualCommand {
        command: Some(cmd_book),
        events: Some(prior),
    }
}

fn dispatch_report_after(n_ticks: usize) -> BusinessResponse {
    let built = Router::new("agg-counter")
        .with_handler(|| CounterAgg)
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    ch.dispatch(contextual_report_with_prior_ticks(n_ticks))
        .expect("dispatch failed")
}

#[test]
fn applies_rebuilds_state_before_handles_runs() {
    let response = dispatch_report_after(3);
    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected Events, got {:?}", response.result);
    };
    assert_eq!(
        book.pages.len(),
        3,
        "three prior Ticks should lift counter to 3 before Report observes it"
    );
}

#[test]
fn applies_with_zero_prior_events_leaves_state_at_default() {
    let response = dispatch_report_after(0);
    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected Events, got {:?}", response.result);
    };
    assert_eq!(book.pages.len(), 0);
}

// ---------------------------------------------------------------------------
// #[state_factory] override
// ---------------------------------------------------------------------------

#[derive(Default)]
struct CounterWithSeed {
    value: u32,
}

struct CounterAggWithSeed;

#[aggregate(domain = "seed", state = CounterWithSeed)]
impl CounterAggWithSeed {
    #[state_factory]
    #[allow(dead_code)]
    fn seeded() -> CounterWithSeed {
        // Plan R7: this overrides `Default::default()` so initial state is 42.
        CounterWithSeed { value: 42 }
    }

    #[applies(Tick)]
    #[allow(unused_variables, dead_code)]
    fn on_tick(state: &mut CounterWithSeed, evt: Tick) {
        state.value += 1;
    }

    #[handles(Report)]
    #[allow(unused_variables, dead_code)]
    fn report(&self, cmd: Report, state: &CounterWithSeed, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook {
            pages: vec![EventPage::default(); state.value as usize],
            ..Default::default()
        })
    }
}

#[test]
fn state_factory_overrides_default_for_initial_state() {
    let built = Router::new("agg-seed")
        .with_handler(|| CounterAggWithSeed)
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    let response = ch
        .dispatch(contextual_report_with_prior_ticks(0))
        .expect("dispatch failed");
    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected Events");
    };
    assert_eq!(
        book.pages.len(),
        42,
        "state_factory must seed state.value to 42"
    );
}

#[test]
fn state_factory_seeded_state_still_replays_applies() {
    let built = Router::new("agg-seed")
        .with_handler(|| CounterAggWithSeed)
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    let response = ch
        .dispatch(contextual_report_with_prior_ticks(3))
        .expect("dispatch failed");
    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected Events");
    };
    assert_eq!(
        book.pages.len(),
        45,
        "state_factory seed 42 + 3 Ticks should lift counter to 45"
    );
}
