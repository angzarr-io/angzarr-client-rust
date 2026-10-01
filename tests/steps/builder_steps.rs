//! Step definitions for `features/client/builder.feature`.
//!
//! Exercises `Router::new(..).with_handler(..).build()` with real
//! `#[command_handler]` / `#[saga]` types. Registering a type that is not a
//! handler kind is a compile error in Rust, so that scenario runs a trybuild
//! compile-fail fixture.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use angzarr_client::error_codes::codes;
use angzarr_client::proto::{business_response, EventBook, SagaHandleRequest, SagaResponse};
use angzarr_client::router::{BuildError, Built, Router};
use angzarr_client::{command_handler, saga, CommandResult};
use cucumber::{given, then, when, World};

use super::deferred::{event_page_of, trigger_book, unsequenced_command};
use crate::common::fixtures::{CreateOrder, OrderCreated, ProcessPayment, ReserveStock};

type CallLog = Arc<Mutex<Vec<&'static str>>>;

#[derive(Default)]
pub struct OrderState;

pub struct Order {
    calls: CallLog,
}

#[command_handler(domain = "order", state = OrderState)]
impl Order {
    #[handles(CreateOrder)]
    fn on_create(
        &self,
        _cmd: CreateOrder,
        _state: &OrderState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        self.calls.lock().unwrap().push("Order");
        Ok(EventBook {
            pages: vec![event_page_of(&OrderCreated::default())],
            ..Default::default()
        })
    }
}

#[derive(Default)]
pub struct PaymentState;

pub struct Payment {
    calls: CallLog,
}

#[command_handler(domain = "payment", state = PaymentState)]
impl Payment {
    #[handles(ProcessPayment)]
    fn on_process(
        &self,
        _cmd: ProcessPayment,
        _state: &PaymentState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        self.calls.lock().unwrap().push("Payment");
        Ok(EventBook::default())
    }
}

pub struct Alpha;

#[command_handler(domain = "order", state = OrderState)]
impl Alpha {
    #[handles(CreateOrder)]
    fn on_create(
        &self,
        _cmd: CreateOrder,
        _state: &OrderState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

pub struct Beta;

#[command_handler(domain = "order", state = OrderState)]
impl Beta {
    #[handles(CreateOrder)]
    fn on_create(
        &self,
        _cmd: CreateOrder,
        _state: &OrderState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

pub struct OrderFulfillment;

#[saga(name = "OrderFulfillment", source = "order", target = "inventory")]
impl OrderFulfillment {
    #[handles(OrderCreated)]
    fn on_created(&self, _event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![unsequenced_command(&ReserveStock::default(), "inventory")],
            events: vec![],
        })
    }
}

/// Registration recipe; `Router` is consumed by `build`, so the world keeps
/// what to register and assembles the router in the When step.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Registration {
    Order,
    Payment,
    Alpha,
    Beta,
    Saga,
}

#[derive(Debug, Default, World)]
pub struct BuilderWorld {
    registrations: Vec<Registration>,
    calls: CallLog,
    introspections: Arc<AtomicU32>,
    count_introspections: bool,
    unmarked_fixture: Option<&'static str>,
    built: Option<Built>,
    error: Option<BuildError>,
}

impl BuilderWorld {
    fn build(&mut self) {
        let mut router = Router::new("builder");
        for r in self.registrations.clone() {
            let calls = Arc::clone(&self.calls);
            router = match r {
                Registration::Order if self.count_introspections => {
                    let introspections = Arc::clone(&self.introspections);
                    router.with_handler(move || {
                        introspections.fetch_add(1, Ordering::SeqCst);
                        Order {
                            calls: Arc::clone(&calls),
                        }
                    })
                }
                Registration::Order => router.with_handler(move || Order {
                    calls: Arc::clone(&calls),
                }),
                Registration::Payment => router.with_handler(move || Payment {
                    calls: Arc::clone(&calls),
                }),
                Registration::Alpha => router.with_handler(|| Alpha),
                Registration::Beta => router.with_handler(|| Beta),
                Registration::Saga => router.with_handler(|| OrderFulfillment),
            };
        }
        match router.build() {
            Ok(built) => self.built = Some(built),
            Err(e) => self.error = Some(e),
        }
    }

    fn error_code(&self) -> &'static str {
        assert!(
            self.built.is_none(),
            "expected a build error, got {:?}",
            self.built
        );
        self.error.as_ref().expect("build error").code()
    }
}

// --- Given -----------------------------------------------------------------

#[given("an empty handler configuration")]
fn given_empty(world: &mut BuilderWorld) {
    world.registrations.clear();
}

#[given("a component that has not been marked as a handler kind")]
fn given_unmarked(world: &mut BuilderWorld) {
    world.unmarked_fixture = Some("tests/router/ui/with_handler_rejects_non_handler.rs");
}

#[given(expr = "a command handler {string} for domain {string} with order state")]
fn given_order(world: &mut BuilderWorld, name: String, domain: String) {
    assert_eq!((name.as_str(), domain.as_str()), ("Order", "order"));
    world.registrations.push(Registration::Order);
}

#[given(expr = "another command handler {string} for domain {string} with payment state")]
fn given_payment(world: &mut BuilderWorld, name: String, domain: String) {
    assert_eq!((name.as_str(), domain.as_str()), ("Payment", "payment"));
    world.registrations.push(Registration::Payment);
}

#[given(expr = "a saga {string} translating from {string} to {string}")]
fn given_saga(world: &mut BuilderWorld, name: String, source: String, target: String) {
    assert_eq!(
        (name.as_str(), source.as_str(), target.as_str()),
        ("OrderFulfillment", "order", "inventory")
    );
    world.registrations.push(Registration::Saga);
}

#[given(
    expr = "two command handlers Alpha and Beta for domain {string} both handling the same command"
)]
fn given_alpha_beta(world: &mut BuilderWorld, domain: String) {
    assert_eq!(domain, "order");
    world.registrations.push(Registration::Alpha);
    world.registrations.push(Registration::Beta);
}

#[given("the handler reports how many times it has been introspected")]
fn given_counting(world: &mut BuilderWorld) {
    world.count_introspections = true;
}

// --- When ------------------------------------------------------------------

#[when("I build the router")]
fn when_build(world: &mut BuilderWorld) {
    world.build();
}

#[when("I register the handler and build the router")]
fn when_register_and_build(world: &mut BuilderWorld) {
    world.build();
}

#[when("I attempt to register it")]
fn when_attempt(world: &mut BuilderWorld) {
    assert!(world.unmarked_fixture.is_some());
}

// --- Then ------------------------------------------------------------------

#[then("the configuration is rejected because no handlers are registered")]
fn then_empty(world: &mut BuilderWorld) {
    assert_eq!(world.error_code(), codes::ROUTER_NO_HANDLERS);
}

#[then("the configuration is rejected because the component is not a recognised handler")]
fn then_unmarked(world: &mut BuilderWorld) {
    let fixture = world.unmarked_fixture.expect("fixture");
    // `with_handler` requires `H: Handler`; the fixture passes a plain struct
    // and must fail to compile with the trait-bound error recorded beside it.
    trybuild::TestCases::new().compile_fail(fixture);
}

#[then("the result routes commands to their handlers")]
fn then_routes_commands(world: &mut BuilderWorld) {
    let Some(Built::CommandHandler(router)) = world.built.take() else {
        panic!(
            "expected a command-handler router, got {:?} / {:?}",
            world.built, world.error
        );
    };
    let order = router
        .dispatch(super::deferred::command_delivery(
            &CreateOrder::default(),
            "order",
        ))
        .expect("order dispatch");
    assert!(matches!(
        order.result,
        Some(business_response::Result::Events(ref b)) if b.pages.len() == 1
    ));
    router
        .dispatch(super::deferred::command_delivery(
            &ProcessPayment::default(),
            "payment",
        ))
        .expect("payment dispatch");
    // The build probe instantiates each command handler once without
    // dispatching; only the two dispatches above reach handler methods.
    assert_eq!(*world.calls.lock().unwrap(), vec!["Order", "Payment"]);
}

#[then("the configuration is rejected for mixing handler kinds")]
fn then_mixed(world: &mut BuilderWorld) {
    assert_eq!(world.error_code(), codes::MIXED_HANDLER_KINDS);
    let details = world.error.as_ref().unwrap().details();
    assert_eq!(
        details.get("handler_kind").map(String::as_str),
        Some("CommandHandler")
    );
    assert_eq!(details.get("other_kind").map(String::as_str), Some("Saga"));
}

#[then(
    "the configuration is rejected because two command handlers share the same domain and command"
)]
fn then_duplicate(world: &mut BuilderWorld) {
    assert_eq!(world.error_code(), codes::DUPLICATE_COMMAND_HANDLER);
    let details = world.error.as_ref().unwrap().details();
    assert_eq!(details.get("domain").map(String::as_str), Some("order"));
    assert_eq!(
        details.get("type_url").cloned(),
        Some(angzarr_client::full_type_url::<CreateOrder>())
    );
}

#[then("the handler has been introspected exactly once")]
fn then_introspected_once(world: &mut BuilderWorld) {
    assert!(world.built.is_some(), "build failed: {:?}", world.error);
    assert_eq!(world.introspections.load(Ordering::SeqCst), 1);
}

#[then("the result routes saga notifications to their handlers")]
fn then_routes_saga(world: &mut BuilderWorld) {
    let Some(Built::Saga(router)) = world.built.take() else {
        panic!(
            "expected a saga router, got {:?} / {:?}",
            world.built, world.error
        );
    };
    let response = router
        .dispatch(SagaHandleRequest {
            source: Some(trigger_book(
                &OrderCreated::default(),
                "order",
                "order-1",
                0,
            )),
            ..Default::default()
        })
        .expect("saga dispatch");
    assert_eq!(response.commands.len(), 1);
    assert_eq!(
        response.commands[0]
            .cover
            .as_ref()
            .map(|c| c.domain.as_str()),
        Some("inventory")
    );
}
