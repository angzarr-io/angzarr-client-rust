//! Rejection-compensation step definitions.

use angzarr_client::proto::{
    business_response, command_page, BusinessResponse, CommandBook, CommandPage, ContextualCommand,
    Cover, EventBook, EventPage, Notification, RejectionNotification,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{aggregate, full_type_url, CommandResult};
use cucumber::{given, then, when, World};
use prost::Message;
use prost_types::Any;

// ---------------------------------------------------------------------------
// Protos.
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct ReserveStock {}
impl ::prost::Name for ReserveStock {
    const NAME: &'static str = "ReserveStock";
    const PACKAGE: &'static str = "inventory";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct ProcessPayment {}
impl ::prost::Name for ProcessPayment {
    const NAME: &'static str = "ProcessPayment";
    const PACKAGE: &'static str = "payment";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct CreateShipment {}
impl ::prost::Name for CreateShipment {
    const NAME: &'static str = "CreateShipment";
    const PACKAGE: &'static str = "fulfillment";
}

#[derive(Default)]
struct PaymentState;

struct Payment;

#[aggregate(domain = "payment", state = PaymentState)]
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
                    payload: Some(angzarr_client::proto::event_page::Payload::Event(Any {
                        type_url: "FundsReleased".to_string(),
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

#[aggregate(domain = "payment", state = PaymentState)]
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
                    payload: Some(angzarr_client::proto::event_page::Payload::Event(Any {
                        type_url: "FundsReleased".to_string(),
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

fn notification_for<T: prost::Message + prost::Name>(domain: &str, cmd: T) -> ContextualCommand {
    let rejected_cmd_any = Any {
        type_url: full_type_url::<T>(),
        value: cmd.encode_to_vec(),
    };
    let rejected_command = CommandBook {
        cover: Some(Cover {
            domain: domain.to_string(),
            ..Default::default()
        }),
        pages: vec![CommandPage {
            payload: Some(command_page::Payload::Command(rejected_cmd_any)),
            ..Default::default()
        }],
    };
    let rejection = RejectionNotification {
        rejected_command: Some(rejected_command),
        rejection_reason: "test".to_string(),
    };
    let rejection_any = Any {
        type_url: full_type_url::<RejectionNotification>(),
        value: rejection.encode_to_vec(),
    };
    let notif = Notification {
        payload: Some(rejection_any),
        ..Default::default()
    };
    let any = Any {
        type_url: full_type_url::<Notification>(),
        value: notif.encode_to_vec(),
    };
    ContextualCommand {
        command: Some(CommandBook {
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(any)),
                ..Default::default()
            }],
            ..Default::default()
        }),
        events: Some(EventBook::default()),
    }
}

// ---------------------------------------------------------------------------
// Given steps.
// ---------------------------------------------------------------------------

#[given(expr = "a command handler {string} for domain {string} with state {word}")]
async fn given_handler(_world: &mut RejectionWorld, _name: String, _d: String, _s: String) {}

#[given(
    expr = "Payment has a @rejected\\({string}, {string}\\) handler emitting FundsReleased"
)]
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

#[when(
    expr = "a Notification wrapping a rejected ReserveStock in domain {string} is dispatched"
)]
async fn when_dispatch_reserve_stock(world: &mut RejectionWorld, domain: String) {
    let ch = build(world);
    let ctx = notification_for(&domain, ReserveStock {});
    world.response = Some(ch.dispatch(ctx).expect("dispatch"));
}

#[when(
    expr = "a Notification wrapping a rejected ProcessPayment in domain {string} is dispatched"
)]
async fn when_dispatch_process_payment(world: &mut RejectionWorld, domain: String) {
    let ch = build(world);
    let ctx = notification_for(&domain, ProcessPayment {});
    world.response = Some(ch.dispatch(ctx).expect("dispatch"));
}

#[when(
    expr = "a Notification wrapping a rejected CreateShipment in domain {string} is dispatched"
)]
async fn when_dispatch_create_shipment(world: &mut RejectionWorld, domain: String) {
    let ch = build(world);
    let ctx = notification_for(&domain, CreateShipment {});
    world.response = Some(ch.dispatch(ctx).expect("dispatch"));
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
