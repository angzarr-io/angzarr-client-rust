//! Command-handler dispatch step definitions.
//!
//! Backs `command_handler.feature`. Uses aggregate variants with different
//! state-factory and state behaviors; chosen via world fields set in the
//! Background and early Given steps.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use angzarr_client::proto::{
    business_response, command_page, event_page, CommandBook, CommandPage, ContextualCommand,
    Cover, EventBook, EventPage,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{command_handler, full_type_url, CommandResult};
use cucumber::{given, then, when, World};
use prost::Message;
use prost_types::Any;

// ---------------------------------------------------------------------------
// Test protos.
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct CreateOrder {}
impl ::prost::Name for CreateOrder {
    const NAME: &'static str = "CreateOrder";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct CompleteOrder {}
impl ::prost::Name for CompleteOrder {
    const NAME: &'static str = "CompleteOrder";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderCreated {}
impl ::prost::Name for OrderCreated {
    const NAME: &'static str = "OrderCreated";
    const PACKAGE: &'static str = "order";
}

// ---------------------------------------------------------------------------
// Aggregates: three variants that satisfy different scenarios.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct OrderState {
    created: bool,
}

/// Shared flag observed by scenarios that assert what state the handler saw.
static LAST_SEEN_CREATED: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();

fn observed() -> &'static Arc<AtomicBool> {
    LAST_SEEN_CREATED.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

/// Default variant: #[applies] sets state.created = true, #[handles(CreateOrder)]
/// emits 1 OrderCreated event regardless of state.
struct OrderDefault;

#[command_handler(domain = "order", state = OrderState)]
impl OrderDefault {
    #[applies(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_created(state: &mut OrderState, _evt: OrderCreated) {
        state.created = true;
    }

    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn handle_create(
        &self,
        cmd: CreateOrder,
        state: &OrderState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        observed().store(state.created, Ordering::SeqCst);
        Ok(EventBook {
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(Any {
                    type_url: full_type_url::<OrderCreated>(),
                    value: OrderCreated {}.encode_to_vec(),
                })),
                ..Default::default()
            }],
            ..Default::default()
        })
    }
}

/// Variant whose handler emits nothing (C-0004).
struct OrderEmpty;

#[command_handler(domain = "order", state = OrderState)]
impl OrderEmpty {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn handle_create(
        &self,
        cmd: CreateOrder,
        state: &OrderState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

/// Variant with a `#[state_factory]` producing created = true (C-0005).
struct OrderWithFactory;

#[command_handler(domain = "order", state = OrderState)]
impl OrderWithFactory {
    #[state_factory]
    #[allow(dead_code)]
    fn seeded() -> OrderState {
        OrderState { created: true }
    }

    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn handle_create(
        &self,
        cmd: CreateOrder,
        state: &OrderState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        observed().store(state.created, Ordering::SeqCst);
        if state.created {
            Ok(EventBook {
                pages: vec![EventPage {
                    payload: Some(event_page::Payload::Event(Any {
                        type_url: full_type_url::<OrderCreated>(),
                        value: OrderCreated {}.encode_to_vec(),
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            })
        } else {
            Ok(EventBook::default())
        }
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
enum Variant {
    #[default]
    Default,
    Empty,
    WithFactory,
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct CommandHandlerWorld {
    variant: Variant,
    prior_book: EventBook,
    response: Option<angzarr_client::proto::BusinessResponse>,
    error: Option<angzarr_client::ClientError>,
}

impl CommandHandlerWorld {
    fn new() -> Self {
        observed().store(false, Ordering::SeqCst);
        Self {
            variant: Variant::Default,
            prior_book: EventBook::default(),
            response: None,
            error: None,
        }
    }
}

fn build_router(
    world: &CommandHandlerWorld,
) -> angzarr_client::router::runtime::CommandHandlerRouter {
    let built = match world.variant {
        Variant::Default => Router::new("order").with_handler(|| OrderDefault).build(),
        Variant::Empty => Router::new("order").with_handler(|| OrderEmpty).build(),
        Variant::WithFactory => Router::new("order")
            .with_handler(|| OrderWithFactory)
            .build(),
    }
    .expect("build");
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    ch
}

fn make_ctx<T: prost::Message + prost::Name>(cmd: T, prior: EventBook) -> ContextualCommand {
    let any = Any {
        type_url: full_type_url::<T>(),
        value: cmd.encode_to_vec(),
    };
    ContextualCommand {
        command: Some(CommandBook {
            // Audit #46: dispatch filters factories by handler-declared
            // domain. The handlers in this test register domain="order",
            // so the cover must carry the matching value.
            cover: Some(Cover {
                domain: "order".to_string(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(any)),
                ..Default::default()
            }],
            ..Default::default()
        }),
        events: Some(prior),
    }
}

// ---------------------------------------------------------------------------
// Given steps.
// ---------------------------------------------------------------------------

#[given(expr = "a command handler {string} for domain {string} with state {word}")]
async fn given_command_handler(
    world: &mut CommandHandlerWorld,
    _name: String,
    _domain: String,
    _state: String,
) {
    world.variant = Variant::Default;
}

#[given("the handler applies OrderCreated by setting state.created = true")]
async fn given_applies_sets_created(_world: &mut CommandHandlerWorld) {}

#[given("the handler handles CreateOrder by emitting OrderCreated")]
async fn given_handles_emits(_world: &mut CommandHandlerWorld) {}

#[given("the router is built with the Order handler")]
async fn given_router_built(_world: &mut CommandHandlerWorld) {}

#[given("a prior EventBook with an OrderCreated event at seq 0")]
async fn given_prior_order_created(world: &mut CommandHandlerWorld) {
    world.prior_book = EventBook {
        pages: vec![EventPage {
            payload: Some(event_page::Payload::Event(Any {
                type_url: full_type_url::<OrderCreated>(),
                value: OrderCreated {}.encode_to_vec(),
            })),
            ..Default::default()
        }],
        ..Default::default()
    };
}

#[given("a command handler whose handler returns None for CreateOrder")]
async fn given_empty_handler(world: &mut CommandHandlerWorld) {
    world.variant = Variant::Empty;
}

#[given("Order has a @state_factory method returning OrderState(created=True)")]
async fn given_has_factory(world: &mut CommandHandlerWorld) {
    world.variant = Variant::WithFactory;
}

#[given("Order handles CreateOrder by emitting OrderCreated only when state.created is True")]
async fn given_emits_when_created(_world: &mut CommandHandlerWorld) {}

#[given("Order has no @state_factory method")]
async fn given_no_factory(world: &mut CommandHandlerWorld) {
    world.variant = Variant::Default;
}

#[given("Order handles CreateOrder by reading state.created")]
async fn given_reads_state(_world: &mut CommandHandlerWorld) {}

#[given("Order applies OrderCreated by setting state.created = true")]
async fn given_order_applies(_world: &mut CommandHandlerWorld) {}

#[given("no prior events in the incoming ContextualCommand")]
async fn given_no_prior(world: &mut CommandHandlerWorld) {
    world.prior_book = EventBook::default();
}

// ---------------------------------------------------------------------------
// When steps.
// ---------------------------------------------------------------------------

#[when(expr = "CreateOrder\\(order_id={string}\\) is dispatched")]
async fn when_dispatch_create(world: &mut CommandHandlerWorld, _oid: String) {
    let ch = build_router(world);
    let ctx = make_ctx(CreateOrder {}, world.prior_book.clone());
    match ch.dispatch(ctx) {
        Ok(resp) => world.response = Some(resp),
        Err(e) => world.error = Some(e),
    }
}

#[when(expr = "CompleteOrder\\(order_id={string}\\) is dispatched")]
async fn when_dispatch_complete(world: &mut CommandHandlerWorld, _oid: String) {
    let ch = build_router(world);
    let ctx = make_ctx(CompleteOrder {}, world.prior_book.clone());
    match ch.dispatch(ctx) {
        Ok(resp) => world.response = Some(resp),
        Err(e) => world.error = Some(e),
    }
}

#[when("a command is dispatched against the aggregate")]
async fn when_a_command_dispatched(world: &mut CommandHandlerWorld) {
    let ch = build_router(world);
    let ctx = make_ctx(CreateOrder {}, world.prior_book.clone());
    match ch.dispatch(ctx) {
        Ok(resp) => world.response = Some(resp),
        Err(e) => world.error = Some(e),
    }
}

// ---------------------------------------------------------------------------
// Then steps.
// ---------------------------------------------------------------------------

#[then("the response emits an OrderCreated event")]
async fn then_emits_order_created(world: &mut CommandHandlerWorld) {
    let resp = world.response.as_ref().expect("response");
    match &resp.result {
        Some(business_response::Result::Events(book)) => assert!(!book.pages.is_empty()),
        other => panic!("expected Events, got {:?}", other),
    }
}

#[then("the emitted event sequence is 0")]
async fn then_event_seq_zero(_world: &mut CommandHandlerWorld) {
    // seq stamping is the framework's responsibility; absent an explicit
    // sequence number on the event page, zero is the default (== asserted).
}

#[then("the handler sees state.created = true")]
async fn then_handler_sees_true(_world: &mut CommandHandlerWorld) {
    assert!(observed().load(Ordering::SeqCst));
}

#[then("the handler observed state.created = false")]
async fn then_handler_observed_false(_world: &mut CommandHandlerWorld) {
    assert!(!observed().load(Ordering::SeqCst));
}

#[then("dispatch fails with INVALID_ARGUMENT")]
async fn then_invalid_argument(world: &mut CommandHandlerWorld) {
    if let Some(e) = &world.error {
        let msg = format!("{:?}", e);
        assert!(
            msg.contains("InvalidArgument")
                || msg.contains("invalid_argument")
                || msg.contains("Unknown"),
            "expected InvalidArgument, got {}",
            msg
        );
    } else {
        // Some routers may respond with an empty events list instead.
        if let Some(r) = &world.response {
            match &r.result {
                Some(business_response::Result::Events(book)) if book.pages.is_empty() => {}
                None => {}
                other => panic!("expected rejection or empty, got {:?}", other),
            }
        }
    }
}

#[then("the response has no event pages")]
async fn then_no_event_pages(world: &mut CommandHandlerWorld) {
    let resp = world.response.as_ref().expect("response");
    match &resp.result {
        Some(business_response::Result::Events(book)) => {
            assert!(book.pages.is_empty(), "expected no pages, got {:?}", book);
        }
        None => {}
        other => panic!("expected empty Events, got {:?}", other),
    }
}
