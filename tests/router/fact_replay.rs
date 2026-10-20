//! Audit #45 — the `#[handles_fact]` opt-in surface.
//!
//! Covers:
//! - a fact (the Fact arm of a command page) routes by its cover domain to
//!   the matching `#[handles_fact]` method, which records the (possibly
//!   annotated) fact;
//! - an aggregate that declares no fact handler refuses every fact with
//!   `INVALID_ARGUMENT` / `NO_FACT_HANDLER`.

use angzarr_client::proto::{
    command_page, CommandBook, CommandPage, ContextualCommand, EventBook, Fact,
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
// for fact payloads.
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
// Aggregate WITHOUT a fact handler — every fact is refused.

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
    fn create(&self, cmd: CreateOrder, state: &OrderState, seq: u64) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

// --------------------------------------------------------------------------
// Aggregate WITH `#[handles_fact]` — facts route to it.

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
    fn create(&self, cmd: CreateOrder, state: &OrderState, seq: u64) -> CommandResult<EventBook> {
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
fn a_fact_routes_to_its_fact_handler() {
    let r = Router::new("orders")
        .with_handler(|| FactOrder)
        .build()
        .expect("build");
    let r = match r {
        angzarr_client::router::Built::CommandHandler(r) => r,
        _ => panic!("expected CommandHandler"),
    };

    let req = fact_command(Any {
        type_url: full_type_url::<StockReserved>(),
        value: ::prost::Message::encode_to_vec(&StockReserved {
            order_id: "o-1".into(),
        }),
    });

    let book = r.dispatch(req).expect("dispatch").events.expect("events");
    assert_eq!(book.pages.len(), 1);
    let Some(recorded) = &book.pages[0].payload else {
        panic!("expected an event page");
    };
    assert_eq!(recorded.type_url, full_type_url::<StockReserved>());
    let recorded: StockReserved = ::prost::Message::decode(recorded.value.as_slice()).unwrap();
    assert_eq!(recorded.order_id, "o-1-seen");
}

#[tokio::test]
async fn a_fact_without_fact_handlers_is_refused_with_no_fact_handler() {
    use angzarr_client::proto::command_handler_service_server::CommandHandlerService;
    let angzarr_client::router::Built::CommandHandler(r) = Router::new("orders")
        .with_handler(|| PlainOrder)
        .build()
        .expect("build")
    else {
        panic!("expected CommandHandler");
    };
    let grpc = angzarr_client::handler::CommandHandlerGrpc::new(r);
    let request = fact_command(Any {
        type_url: full_type_url::<StockReserved>(),
        value: vec![],
    });
    let status = grpc
        .handle(tonic::Request::new(request))
        .await
        .expect_err("undeclared fact is refused");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    let (code, _meta, _cover) =
        angzarr_client::error::unpack_status_details(status.details()).expect("details");
    assert_eq!(code, angzarr_client::error_codes::codes::NO_FACT_HANDLER);
}

/// A contextual command whose page carries `fact` in its Fact arm, for the
/// "order" domain, with no prior events.
fn fact_command(fact: Any) -> ContextualCommand {
    ContextualCommand {
        command: Some(CommandBook {
            cover: Some(angzarr_client::proto::Cover {
                domain: "order".into(),
                ..Default::default()
            }),
            page: Some(CommandPage {
                body: Some(command_page::Body::Fact(Fact {
                    payload: Some(fact),
                })),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}
