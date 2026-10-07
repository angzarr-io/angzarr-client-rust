//! Audit #45 — `#[handles_fact]` and `supports_replay` opt-in surface.
//!
//! Covers:
//! - `CommandHandlerRouter::supports_handle_fact()` and `supports_replay()`
//!   correctly read metadata from the `#[command_handler]`-emitted config.
//! - `dispatch_fact` routes facts by their cover domain to the matching
//!   `#[handles_fact]` method, which records (possibly annotated) facts.
//! - `dispatch_replay` round-trips state through `Any` using the
//!   `#[applies]` machinery.
//! - Aggregates that don't opt in get `false` from `supports_*`. Replay
//!   then answers `UNIMPLEMENTED`; HandleFact refuses every fact with
//!   `INVALID_ARGUMENT` / `NO_FACT_HANDLER`.

use angzarr_client::proto::{
    EventBook, EventPage, FactRequest, PageHeader, ReplayRequest, Snapshot,
};
use angzarr_client::router::Router;
// `applies` and `handles` are macro markers consumed by
// `#[command_handler]`; the compiler counts the imports as unused once
// the parent macro strips the markers. Allow at the import level.
#[allow(unused_imports)]
use angzarr_client::router::{applies, command_handler, handles, handles_fact};
#[allow(unused_imports)]
use angzarr_client::{full_type_url, CommandResult};
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
    ) -> CommandResult<StockReserved> {
        // Facts cannot be refused; the handler records them, here annotated.
        Ok(StockReserved {
            order_id: format!("{}-seen", evt.order_id),
        })
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
    let mut facts = EventBook {
        cover: Some(angzarr_client::proto::Cover {
            domain: "order".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
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
    let Some(angzarr_client::proto::event_page::Payload::Event(recorded)) = &book.pages[0].payload
    else {
        panic!("expected an event page");
    };
    assert_eq!(recorded.type_url, full_type_url::<StockReserved>());
    let recorded: StockReserved = ::prost::Message::decode(recorded.value.as_slice()).unwrap();
    assert_eq!(recorded.order_id, "o-1-seen");
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
    let snap = Snapshot {
        state: Some(Any {
            type_url: full_type_url::<OrderState>(),
            value: ::prost::Message::encode_to_vec(&OrderState {
                order_id: "initial".into(),
                apply_count: 5,
            }),
        }),
        ..Default::default()
    };
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

#[tokio::test]
async fn handle_fact_without_fact_handlers_refuses_with_no_fact_handler() {
    use angzarr_client::proto::command_handler_service_server::CommandHandlerService;
    let angzarr_client::router::Built::CommandHandler(r) = Router::new("orders")
        .with_handler(|| PlainOrder)
        .build()
        .expect("build")
    else {
        panic!("expected CommandHandler");
    };
    let grpc = angzarr_client::handler::CommandHandlerGrpc::new(r);
    let request = FactRequest {
        facts: Some(EventBook {
            cover: Some(angzarr_client::proto::Cover {
                domain: "order".into(),
                ..Default::default()
            }),
            pages: vec![EventPage {
                payload: Some(angzarr_client::proto::event_page::Payload::Event(Any {
                    type_url: full_type_url::<StockReserved>(),
                    value: vec![],
                })),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let status = grpc
        .handle_fact(tonic::Request::new(request))
        .await
        .expect_err("undeclared fact is refused");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    let (code, _meta, _cover) =
        angzarr_client::error::unpack_status_details(status.details()).expect("details");
    assert_eq!(code, angzarr_client::error_codes::codes::NO_FACT_HANDLER);
}
