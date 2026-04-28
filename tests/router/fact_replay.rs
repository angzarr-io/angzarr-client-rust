//! Audit #45 — `#[handles_fact]` and `supports_replay` opt-in surface.
//!
//! Covers:
//! - `CommandHandlerRouter::supports_handle_fact()` and `supports_replay()`
//!   correctly read metadata from the `#[command_handler]`-emitted config.
//! - `dispatch_fact` routes facts to the matching `#[handles_fact]` method
//!   and concatenates emitted events.
//! - `dispatch_replay` round-trips state through `Any` using the
//!   `#[applies]` machinery.
//! - Aggregates that don't opt in get `false` from `supports_*` —
//!   the gRPC adapter then returns `UNIMPLEMENTED` (covered in the
//!   adapter-level integration tests).

use angzarr_client::proto::{
    EventBook, EventPage, FactRequest, PageHeader, ReplayRequest, Snapshot,
};
use angzarr_client::router::Router;
// `applies` and `handles` are macro markers consumed by
// `#[command_handler]`; the compiler counts the imports as unused once
// the parent macro strips the markers. Allow at the import level.
#[allow(unused_imports)]
use angzarr_client::{
    applies, command_handler, full_type_url, handles, handles_fact, CommandResult,
};
use prost_types::Any;

// Test-local proto stubs. Real prost messages so Any pack/unpack works
// for the Replay round-trip.
macro_rules! test_proto {
    ($name:ident { $($field:ident : $ty:ty = $tag:literal,)* }) => {
        #[derive(Clone, PartialEq, ::prost::Message)]
        struct $name {
            $(
                #[prost(string, tag = $tag)]
                $field: ::prost::alloc::string::String,
            )*
        }

        impl ::prost::Name for $name {
            const NAME: &'static str = stringify!($name);
            const PACKAGE: &'static str = "test";
        }
    };
}

test_proto!(CreateOrder {
    order_id: String = "1",
});
test_proto!(OrderCreated {
    order_id: String = "1",
});
test_proto!(StockReserved {
    order_id: String = "1",
});

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderState {
    #[prost(string, tag = "1")]
    order_id: String,
    #[prost(uint32, tag = "2")]
    apply_count: u32,
}

impl ::prost::Name for OrderState {
    const NAME: &'static str = "OrderState";
    const PACKAGE: &'static str = "test";
}

// --------------------------------------------------------------------------
// Aggregate WITHOUT opt-in — supports_* should be false.

struct PlainOrder;

#[command_handler(domain = "order", state = OrderState)]
impl PlainOrder {
    #[applies(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn apply_created(state: &mut OrderState, evt: OrderCreated) {
        state.apply_count += 1;
        state.order_id = evt.order_id;
    }

    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn create(&self, cmd: CreateOrder, state: &OrderState, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[test]
fn supports_handle_fact_false_without_opt_in() {
    let r = Router::new("orders")
        .with_handler(|| PlainOrder)
        .build()
        .expect("build");
    let r = match r {
        angzarr_client::router::Built::CommandHandler(r) => r,
        _ => panic!("expected CommandHandler"),
    };
    assert!(!r.supports_handle_fact());
}

#[test]
fn supports_replay_false_without_opt_in() {
    let r = Router::new("orders")
        .with_handler(|| PlainOrder)
        .build()
        .expect("build");
    let r = match r {
        angzarr_client::router::Built::CommandHandler(r) => r,
        _ => panic!("expected CommandHandler"),
    };
    assert!(!r.supports_replay());
}

// --------------------------------------------------------------------------
// Aggregate WITH `#[handles_fact]` — supports_handle_fact() true,
// dispatch_fact routes correctly.

struct FactOrder;

#[command_handler(domain = "order", state = OrderState)]
impl FactOrder {
    #[applies(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn apply_created(state: &mut OrderState, evt: OrderCreated) {
        state.apply_count += 1;
        state.order_id = evt.order_id;
    }

    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn create(&self, cmd: CreateOrder, state: &OrderState, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }

    #[handles_fact(StockReserved)]
    #[allow(unused_variables, dead_code)]
    fn on_stock_reserved(
        &self,
        evt: StockReserved,
        state: &OrderState,
    ) -> CommandResult<EventBook> {
        // Emit a derived event (semantically: "we acknowledge the stock fact").
        let mut book = EventBook::default();
        let mut page = EventPage::default();
        let any = Any {
            type_url: full_type_url::<OrderCreated>(),
            value: ::prost::Message::encode_to_vec(&OrderCreated {
                order_id: format!("{}-derived", evt.order_id),
            }),
        };
        page.payload = Some(angzarr_client::proto::event_page::Payload::Event(any));
        page.header = Some(PageHeader::default());
        book.pages.push(page);
        Ok(book)
    }
}

#[test]
fn supports_handle_fact_true_with_handles_fact() {
    let r = Router::new("orders")
        .with_handler(|| FactOrder)
        .build()
        .expect("build");
    let r = match r {
        angzarr_client::router::Built::CommandHandler(r) => r,
        _ => panic!("expected CommandHandler"),
    };
    assert!(r.supports_handle_fact());
}

#[test]
fn dispatch_fact_routes_to_matching_handler() {
    let r = Router::new("orders")
        .with_handler(|| FactOrder)
        .build()
        .expect("build");
    let r = match r {
        angzarr_client::router::Built::CommandHandler(r) => r,
        _ => panic!("expected CommandHandler"),
    };

    let mut req = FactRequest::default();
    let mut facts = EventBook::default();
    let mut page = EventPage::default();
    let any = Any {
        type_url: full_type_url::<StockReserved>(),
        value: ::prost::Message::encode_to_vec(&StockReserved {
            order_id: "o-1".into(),
        }),
    };
    page.payload = Some(angzarr_client::proto::event_page::Payload::Event(any));
    page.header = Some(PageHeader::default());
    facts.pages.push(page);
    req.facts = Some(facts);

    let book = r.dispatch_fact(req).expect("dispatch_fact");
    assert_eq!(book.pages.len(), 1);
}

// --------------------------------------------------------------------------
// Aggregate WITH `supports_replay = true` — supports_replay() true,
// dispatch_replay round-trips state through Any.

struct ReplayOrder;

#[command_handler(domain = "order", state = OrderState, supports_replay = true)]
impl ReplayOrder {
    #[applies(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn apply_created(state: &mut OrderState, evt: OrderCreated) {
        state.apply_count += 1;
        state.order_id = evt.order_id;
    }

    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn create(&self, cmd: CreateOrder, state: &OrderState, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[test]
fn supports_replay_true_when_opted_in() {
    let r = Router::new("orders")
        .with_handler(|| ReplayOrder)
        .build()
        .expect("build");
    let r = match r {
        angzarr_client::router::Built::CommandHandler(r) => r,
        _ => panic!("expected CommandHandler"),
    };
    assert!(r.supports_replay());
}

#[test]
fn dispatch_replay_round_trips_state_through_any() {
    let r = Router::new("orders")
        .with_handler(|| ReplayOrder)
        .build()
        .expect("build");
    let r = match r {
        angzarr_client::router::Built::CommandHandler(r) => r,
        _ => panic!("expected CommandHandler"),
    };

    // Base snapshot: order_id="initial", apply_count=5.
    let mut req = ReplayRequest::default();
    let mut snap = Snapshot::default();
    snap.state = Some(Any {
        type_url: full_type_url::<OrderState>(),
        value: ::prost::Message::encode_to_vec(&OrderState {
            order_id: "initial".into(),
            apply_count: 5,
        }),
    });
    req.base_snapshot = Some(snap);

    // One OrderCreated event to apply.
    let mut page = EventPage::default();
    let any = Any {
        type_url: full_type_url::<OrderCreated>(),
        value: ::prost::Message::encode_to_vec(&OrderCreated {
            order_id: "after-replay".into(),
        }),
    };
    page.payload = Some(angzarr_client::proto::event_page::Payload::Event(any));
    page.header = Some(PageHeader::default());
    req.events.push(page);

    let resp = r.dispatch_replay(req).expect("dispatch_replay");
    let any = resp.state.expect("state present");
    let resulting: OrderState =
        ::prost::Message::decode(any.value.as_slice()).expect("decode state");

    // apply_created bumped apply_count to 6 and overwrote order_id.
    assert_eq!(resulting.apply_count, 6);
    assert_eq!(resulting.order_id, "after-replay");
}
