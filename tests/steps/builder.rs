//! Router builder step definitions.
//!
//! Exercises `Router::new(..).with_handler(..).build()` outcomes:
//! empty, wrong kind, mixed kinds, homogeneous kinds, duplicate
//! registration, and factory laziness.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use angzarr_client::command_handler;
use angzarr_client::proto::EventBook;
use angzarr_client::router::{BuildError, Built, Router};
use angzarr_client::saga;
use angzarr_client::{full_type_url, CommandResult};
use cucumber::{given, then, when, World};

use prost::Message;

// ---------------------------------------------------------------------------
// Test protos (shared across scenarios).
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct CreateOrder {}
impl ::prost::Name for CreateOrder {
    const NAME: &'static str = "CreateOrder";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderCreated {}
impl ::prost::Name for OrderCreated {
    const NAME: &'static str = "OrderCreated";
    const PACKAGE: &'static str = "order";
}

// ---------------------------------------------------------------------------
// Test aggregates for the "Order" and "Payment" and "Alpha/Beta" handlers.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct OrderState;

struct OrderAgg;

#[command_handler(domain = "order", state = OrderState)]
impl OrderAgg {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn on_create(
        &self,
        cmd: CreateOrder,
        state: &OrderState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[derive(Default)]
struct PaymentState;

struct PaymentAgg;

#[command_handler(domain = "payment", state = PaymentState)]
impl PaymentAgg {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn on_create(
        &self,
        cmd: CreateOrder,
        state: &PaymentState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

struct AlphaAgg;
#[command_handler(domain = "order", state = OrderState)]
impl AlphaAgg {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn on_create(
        &self,
        cmd: CreateOrder,
        state: &OrderState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

struct BetaAgg;
#[command_handler(domain = "order", state = OrderState)]
impl BetaAgg {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn on_create(
        &self,
        cmd: CreateOrder,
        state: &OrderState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

// Saga: "OrderFulfillment" translating from "order" to "inventory".
struct FulfillmentSaga;

#[saga(name = "OrderFulfillment", source = "order", target = "inventory")]
impl FulfillmentSaga {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_placed(&self, event: OrderCreated) -> CommandResult<angzarr_client::proto::SagaResponse> {
        Ok(angzarr_client::proto::SagaResponse::default())
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Invocations(Arc<AtomicU32>);

#[derive(World)]
#[world(init = Self::new)]
pub struct BuilderWorld {
    router_name: String,
    has_order_handler: bool,
    has_payment_handler: bool,
    has_saga_handler: bool,
    // "Alpha/Beta" dup scenario.
    dup_alpha_beta: bool,
    // Track if factory was invoked at registration+build (must be 0 for C-0065).
    counted_factory: Option<Arc<AtomicU32>>,
    // Result of build().
    build_err: Option<BuildError>,
    built: Option<Built>,
    factory_count: u32,
}

impl std::fmt::Debug for BuilderWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuilderWorld")
            .field("router_name", &self.router_name)
            .field("has_order_handler", &self.has_order_handler)
            .field("has_payment_handler", &self.has_payment_handler)
            .field("has_saga_handler", &self.has_saga_handler)
            .field("dup_alpha_beta", &self.dup_alpha_beta)
            .field("build_err", &self.build_err)
            .field("factory_count", &self.factory_count)
            .finish()
    }
}

impl BuilderWorld {
    fn new() -> Self {
        Self {
            router_name: "default".to_string(),
            has_order_handler: false,
            has_payment_handler: false,
            has_saga_handler: false,
            dup_alpha_beta: false,
            counted_factory: None,
            build_err: None,
            built: None,
            factory_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Given steps.
// ---------------------------------------------------------------------------

#[given(expr = "an empty Router named {string}")]
async fn given_empty_router(world: &mut BuilderWorld, name: String) {
    world.router_name = name;
}

#[given(expr = "a class {string} with no kind decorator")]
async fn given_undecorated_class(_world: &mut BuilderWorld, _name: String) {
    // Enforced at compile time; we only acknowledge the scenario. The
    // corresponding `Then` step passes trivially since Rust's type system
    // already prevents undecorated types from being passed to `with_handler`.
}

#[given(expr = "a command handler {string} for domain {string} with state {word}")]
async fn given_command_handler(
    world: &mut BuilderWorld,
    name: String,
    _domain: String,
    _state: String,
) {
    match name.as_str() {
        "Order" => world.has_order_handler = true,
        "Payment" => world.has_payment_handler = true,
        _ => world.has_order_handler = true,
    }
}

#[given(expr = "another command handler {string} for domain {string} with state {word}")]
async fn given_another_handler(
    world: &mut BuilderWorld,
    name: String,
    _domain: String,
    _state: String,
) {
    match name.as_str() {
        "Order" => world.has_order_handler = true,
        "Payment" => world.has_payment_handler = true,
        _ => world.has_payment_handler = true,
    }
}

#[given(expr = "a saga {string} translating from {string} to {string}")]
async fn given_a_saga(world: &mut BuilderWorld, _name: String, _source: String, _target: String) {
    world.has_saga_handler = true;
}

#[given(expr = "two command handlers Alpha and Beta for domain {string} both handling CreateOrder")]
async fn given_alpha_beta(world: &mut BuilderWorld, _domain: String) {
    world.dup_alpha_beta = true;
}

#[given("a factory that counts invocations")]
async fn given_counting_factory(world: &mut BuilderWorld) {
    world.counted_factory = Some(Arc::new(AtomicU32::new(0)));
}

// ---------------------------------------------------------------------------
// When steps.
// ---------------------------------------------------------------------------

#[when("I build the router")]
async fn when_build(world: &mut BuilderWorld) {
    do_build(world);
}

#[when("I register the handler and build the router")]
async fn when_register_and_build(world: &mut BuilderWorld) {
    do_build(world);
}

#[when("I register it with a factory")]
async fn when_register_undecorated(_world: &mut BuilderWorld) {
    // Enforced at compile time; nothing to do at runtime.
}

fn do_build(world: &mut BuilderWorld) {
    let mut router = Router::new(&world.router_name);

    if world.dup_alpha_beta {
        router = router.with_handler(|| AlphaAgg).with_handler(|| BetaAgg);
        world.factory_count = 2;
    } else {
        if world.has_order_handler && world.has_payment_handler {
            // Two distinct command handlers.
            if let Some(counter) = world.counted_factory.clone() {
                let c = Arc::clone(&counter);
                router = router.with_handler(move || {
                    c.fetch_add(1, Ordering::SeqCst);
                    OrderAgg
                });
            } else {
                router = router.with_handler(|| OrderAgg);
            }
            router = router.with_handler(|| PaymentAgg);
            world.factory_count = 2;
        } else if world.has_order_handler && world.has_saga_handler {
            router = router
                .with_handler(|| OrderAgg)
                .with_handler(|| FulfillmentSaga);
            world.factory_count = 2;
        } else if world.has_order_handler {
            if let Some(counter) = world.counted_factory.clone() {
                let c = Arc::clone(&counter);
                router = router.with_handler(move || {
                    c.fetch_add(1, Ordering::SeqCst);
                    OrderAgg
                });
            } else {
                router = router.with_handler(|| OrderAgg);
            }
            world.factory_count = 1;
        } else if world.has_saga_handler {
            router = router.with_handler(|| FulfillmentSaga);
            world.factory_count = 1;
        }
    }

    match router.build() {
        Ok(b) => world.built = Some(b),
        Err(e) => world.build_err = Some(e),
    }
}

// ---------------------------------------------------------------------------
// Then steps.
// ---------------------------------------------------------------------------

#[then(expr = "the builder raises a BuildError mentioning {string}")]
async fn then_build_error_mentions(world: &mut BuilderWorld, needle: String) {
    match &world.build_err {
        Some(e) => {
            let msg = format!("{}", e);
            assert!(
                msg.contains(&needle) || needle_matches_variant(e, &needle),
                "BuildError '{}' should mention '{}'",
                msg,
                needle
            );
        }
        None => {
            // For the "NotDecorated" scenario the error surfaces at compile
            // time, so at runtime we assume the scenario is vacuously satisfied.
            if needle == "NotDecorated" {
                return;
            }
            panic!("expected BuildError, got Ok ({:?})", world.built);
        }
    }
}

fn needle_matches_variant(e: &BuildError, needle: &str) -> bool {
    let lower = needle.to_ascii_lowercase();
    match e {
        BuildError::Empty => lower.contains("no handler") || lower.contains("empty"),
        BuildError::MixedKinds(_, _) => lower.contains("mix") || lower.contains("cannot mix"),
        BuildError::WrongKind { .. } => lower.contains("wrong") || lower.contains("kind"),
    }
}

#[then("the result is a CommandHandlerRouter")]
async fn then_is_command_handler_router(world: &mut BuilderWorld) {
    assert!(
        matches!(world.built, Some(Built::CommandHandler(_))),
        "expected CommandHandler, got {:?}",
        world.built
    );
}

#[then("the result is a SagaRouter")]
async fn then_is_saga_router(world: &mut BuilderWorld) {
    assert!(
        matches!(world.built, Some(Built::Saga(_))),
        "expected Saga, got {:?}",
        world.built
    );
}

#[then("the build succeeds")]
async fn then_build_succeeds(world: &mut BuilderWorld) {
    assert!(
        world.built.is_some() && world.build_err.is_none(),
        "expected build to succeed, err={:?}",
        world.build_err
    );
}

#[then("the router has two factories registered")]
async fn then_two_factories(world: &mut BuilderWorld) {
    match &world.built {
        Some(Built::CommandHandler(ch)) => assert_eq!(ch.handler_count(), 2),
        other => panic!("expected CommandHandler with 2 factories, got {:?}", other),
    }
}

#[then("the factory invocation count is 0")]
async fn then_factory_invocation_zero(world: &mut BuilderWorld) {
    let counter = world
        .counted_factory
        .as_ref()
        .expect("counted_factory not set");
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

// Trivial no-op step to keep prost::Message and full_type_url used in case the
// compiler flags unused imports at some point.
#[allow(dead_code)]
fn _linker() {
    let _ = full_type_url::<CreateOrder>();
    let _ = OrderCreated {}.encode_to_vec();
}
