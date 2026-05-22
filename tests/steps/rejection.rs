//! Rejection-compensation step definitions.

use angzarr_client::proto::{
    business_response, event_page, BusinessResponse, EventBook, EventPage, Notification,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{command_handler, full_type_url, CommandResult};
use cucumber::{given, then, when, World};
use prost_types::Any;

use crate::common::fixtures::{CreateShipment, FundsReleased, ProcessPayment, ReserveStock};
use crate::common::helpers::{contextual_notification, notification_for};

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PaymentState;

struct Payment;

#[command_handler(domain = "payment", state = PaymentState)]
impl Payment {
    #[rejected(domain = "inventory", command = "ReserveStock")]
    #[allow(unused_variables, dead_code)]
    fn on_rejected(
        &self,
        notif: &Notification,
        state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(EventBook {
                pages: vec![EventPage {
                    payload: Some(event_page::Payload::Event(Any {
                        type_url: full_type_url::<FundsReleased>(),
                        value: vec![],
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        })
    }
}

struct Payment2;

#[command_handler(domain = "payment", state = PaymentState)]
impl Payment2 {
    #[rejected(domain = "inventory", command = "ReserveStock")]
    #[allow(unused_variables, dead_code)]
    fn on_rejected(
        &self,
        notif: &Notification,
        state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(EventBook {
                pages: vec![EventPage {
                    payload: Some(event_page::Payload::Event(Any {
                        type_url: full_type_url::<FundsReleased>(),
                        value: vec![],
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            })),
        })
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
enum Routers {
    #[default]
    Single,
    Double,
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct RejectionWorld {
    variant: Routers,
    response: Option<BusinessResponse>,
}

impl RejectionWorld {
    fn new() -> Self {
        Self {
            variant: Routers::Single,
            response: None,
        }
    }
}

fn build(world: &RejectionWorld) -> angzarr_client::router::runtime::CommandHandlerRouter {
    let built = match world.variant {
        Routers::Single => Router::new("payment").with_handler(|| Payment).build(),
        Routers::Double => Router::new("payment")
            .with_handler(|| Payment)
            .with_handler(|| Payment2)
            .build(),
    }
    .expect("build");
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    ch
}

// ---------------------------------------------------------------------------
// Given steps.
// ---------------------------------------------------------------------------

#[given(expr = "a command handler {string} for domain {string} with state {word}")]
async fn given_handler(_world: &mut RejectionWorld, _name: String, _d: String, _s: String) {}

#[given(expr = "Payment has a @rejected\\({string}, {string}\\) handler emitting FundsReleased")]
async fn given_rejected_handler(_world: &mut RejectionWorld, _d: String, _c: String) {}

#[given("the router is built with the Payment handler")]
async fn given_built(_world: &mut RejectionWorld) {}

#[given(
    expr = "a second Payment handler Payment2 with the same @rejected key emitting FundsReleased"
)]
async fn given_second(world: &mut RejectionWorld) {
    world.variant = Routers::Double;
}

#[given("the router is built with Payment then Payment2")]
async fn given_built_double(world: &mut RejectionWorld) {
    world.variant = Routers::Double;
}

// ---------------------------------------------------------------------------
// When steps.
// ---------------------------------------------------------------------------

#[when(expr = "a Notification wrapping a rejected ReserveStock in domain {string} is dispatched")]
async fn when_dispatch_reserve_stock(world: &mut RejectionWorld, domain: String) {
    let ch = build(world);
    let notif = notification_for(&ReserveStock::default(), &domain);
    world.response = Some(
        ch.dispatch(contextual_notification(notif, "payment"))
            .expect("dispatch"),
    );
}

#[when(expr = "a Notification wrapping a rejected ProcessPayment in domain {string} is dispatched")]
async fn when_dispatch_process_payment(world: &mut RejectionWorld, domain: String) {
    let ch = build(world);
    let notif = notification_for(&ProcessPayment::default(), &domain);
    world.response = Some(
        ch.dispatch(contextual_notification(notif, "payment"))
            .expect("dispatch"),
    );
}

#[when(expr = "a Notification wrapping a rejected CreateShipment in domain {string} is dispatched")]
async fn when_dispatch_create_shipment(world: &mut RejectionWorld, domain: String) {
    let ch = build(world);
    let notif = notification_for(&CreateShipment::default(), &domain);
    world.response = Some(
        ch.dispatch(contextual_notification(notif, "payment"))
            .expect("dispatch"),
    );
}

// ---------------------------------------------------------------------------
// Then steps.
// ---------------------------------------------------------------------------

#[then("the response contains one FundsReleased event")]
async fn then_one_funds_released(world: &mut RejectionWorld) {
    let r = world.response.as_ref().expect("response");
    match &r.result {
        Some(business_response::Result::Events(b)) => assert_eq!(b.pages.len(), 1),
        other => panic!("expected Events, got {:?}", other),
    }
}

#[then("the response contains two FundsReleased events in registration order")]
async fn then_two_funds_released(world: &mut RejectionWorld) {
    let r = world.response.as_ref().expect("response");
    match &r.result {
        Some(business_response::Result::Events(b)) => assert_eq!(b.pages.len(), 2),
        other => panic!("expected Events, got {:?}", other),
    }
}

#[then("the response contains no events")]
async fn then_no_events(world: &mut RejectionWorld) {
    let r = world.response.as_ref().expect("response");
    match &r.result {
        Some(business_response::Result::Events(b)) => assert!(b.pages.is_empty()),
        None => {}
        other => panic!("expected empty Events, got {:?}", other),
    }
}
