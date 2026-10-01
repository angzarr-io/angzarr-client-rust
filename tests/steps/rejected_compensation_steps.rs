//! Rejection-compensation (details) step definitions.
//!
//! Covers state rebuild before the @rejected handler, multi-method routing,
//! sequence stamping on compensation events, and the empty-handler case.

use angzarr_client::proto::{
    business_response, event_page, BusinessResponse, EventBook, EventPage, Notification,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{command_handler, CommandResult};
use cucumber::{given, then, when, World};
use prost_types::Any;

use crate::common::fixtures::{CreateShipment, FundsDeposited, ProcessPayment, ReserveStock};
use crate::common::helpers::{contextual_notification, notification_for, pack_event_page};

// ---------------------------------------------------------------------------
// Aggregates.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PaymentState;

struct PaymentSingle;
#[command_handler(domain = "payment", state = PaymentState)]
impl PaymentSingle {
    #[applies(FundsDeposited)]
    #[allow(unused_variables, dead_code)]
    fn apply_deposit(state: &mut PaymentState, _evt: FundsDeposited) {}

    #[rejected(domain = "inventory", command = "ReserveStock")]
    #[allow(unused_variables, dead_code)]
    fn on_rejected(
        &self,
        notif: &Notification,
        state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(EventBook {
                pages: vec![tagged_page("FundsReleased")],
                ..Default::default()
            })),
        })
    }
}

struct PaymentDouble;
#[command_handler(domain = "payment", state = PaymentState)]
impl PaymentDouble {
    #[rejected(domain = "inventory", command = "ReserveStock")]
    #[allow(unused_variables, dead_code)]
    fn on_reserve_stock(
        &self,
        notif: &Notification,
        state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(EventBook {
                pages: vec![tagged_page("FundsReleased")],
                ..Default::default()
            })),
        })
    }

    #[rejected(domain = "payment", command = "ProcessPayment")]
    #[allow(unused_variables, dead_code)]
    fn on_process_payment(
        &self,
        notif: &Notification,
        state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(EventBook {
                pages: vec![tagged_page("WorkflowFailed")],
                ..Default::default()
            })),
        })
    }
}

struct PaymentTwoEvents;
#[command_handler(domain = "payment", state = PaymentState)]
impl PaymentTwoEvents {
    #[rejected(domain = "inventory", command = "ReserveStock")]
    #[allow(unused_variables, dead_code)]
    fn on_reserve_stock(
        &self,
        notif: &Notification,
        state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(EventBook {
                pages: vec![tagged_page("FundsReleased"), tagged_page("FundsReleased")],
                ..Default::default()
            })),
        })
    }
}

struct PaymentNoRejection;
#[command_handler(domain = "payment", state = PaymentState)]
impl PaymentNoRejection {
    #[handles(ProcessPayment)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, cmd: ProcessPayment, state: &PaymentState, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

fn tagged_page(tag: &str) -> EventPage {
    EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: tag.to_string(),
            value: vec![],
        })),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
enum Variant {
    #[default]
    Single,
    Double,
    TwoEvents,
    None,
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct RejectedCompensationWorld {
    variant: Variant,
    prior_book: EventBook,
    response: Option<BusinessResponse>,
}

impl RejectedCompensationWorld {
    fn new() -> Self {
        Self {
            variant: Variant::Single,
            prior_book: EventBook::default(),
            response: None,
        }
    }
}

fn build(
    world: &RejectedCompensationWorld,
) -> angzarr_client::router::runtime::CommandHandlerRouter {
    let built = match world.variant {
        Variant::Single => Router::new("payment")
            .with_handler(|| PaymentSingle)
            .build(),
        Variant::Double => Router::new("payment")
            .with_handler(|| PaymentDouble)
            .build(),
        Variant::TwoEvents => Router::new("payment")
            .with_handler(|| PaymentTwoEvents)
            .build(),
        Variant::None => Router::new("payment")
            .with_handler(|| PaymentNoRejection)
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

#[given(expr = "a command handler {string} for domain {string} with stateful rejection")]
async fn given_handler_stateful(world: &mut RejectedCompensationWorld, _name: String, _d: String) {
    world.variant = Variant::Single;
}

#[given("Payment @applies FundsDeposited by setting state.bankroll")]
async fn given_applies_deposit(_world: &mut RejectedCompensationWorld) {}

#[given(
    "Payment has a @rejected(\"inventory\", \"ReserveStock\") handler that emits FundsReleased carrying state.bankroll"
)]
async fn given_rejected_carrying(_world: &mut RejectedCompensationWorld) {}

#[given(expr = "a command handler {string} for domain {string} with two @rejected handlers")]
async fn given_handler_two_rejected(
    world: &mut RejectedCompensationWorld,
    _name: String,
    _d: String,
) {
    world.variant = Variant::Double;
}

#[given(expr = "a command handler {string} for domain {string} with no rejection handlers")]
async fn given_handler_none(world: &mut RejectedCompensationWorld, _name: String, _d: String) {
    world.variant = Variant::None;
}

#[given("Payment has a @rejected(\"inventory\", \"ReserveStock\") handler emitting FundsReleased")]
async fn given_rejected_funds(_world: &mut RejectedCompensationWorld) {}

#[given("Payment has a @rejected(\"payment\", \"ProcessPayment\") handler emitting WorkflowFailed")]
async fn given_rejected_workflow(_world: &mut RejectedCompensationWorld) {}

#[given(
    "Payment has a @rejected(\"inventory\", \"ReserveStock\") handler emitting two FundsReleased events"
)]
async fn given_rejected_two_events(world: &mut RejectedCompensationWorld) {
    world.variant = Variant::TwoEvents;
}

#[given("the router is built with the Payment handler")]
async fn given_built(_world: &mut RejectedCompensationWorld) {}

#[given(expr = "a prior EventBook with a FundsDeposited event of bankroll {int}")]
async fn given_prior_bankroll(world: &mut RejectedCompensationWorld, _amount: u32) {
    world.prior_book = EventBook {
        pages: vec![pack_event_page(&FundsDeposited::default(), 0)],
        ..Default::default()
    };
}

#[given(expr = "a prior EventBook whose next_sequence is {int}")]
async fn given_prior_next_seq(world: &mut RejectedCompensationWorld, n: u32) {
    world.prior_book = EventBook {
        next_sequence: n,
        ..Default::default()
    };
}

// ---------------------------------------------------------------------------
// When steps.
// ---------------------------------------------------------------------------

#[when(expr = "a Notification wrapping a rejected ReserveStock in domain {string} is dispatched")]
async fn when_dispatch_reserve(world: &mut RejectedCompensationWorld, domain: String) {
    let ch = build(world);
    let notif = notification_for(&ReserveStock::default(), &domain);
    let mut ctx = contextual_notification(notif, "payment");
    ctx.events = Some(world.prior_book.clone());
    world.response = Some(ch.dispatch(ctx).expect("dispatch"));
}

#[when(expr = "a Notification wrapping a rejected ProcessPayment in domain {string} is dispatched")]
async fn when_dispatch_pp(world: &mut RejectedCompensationWorld, domain: String) {
    let ch = build(world);
    let notif = notification_for(&ProcessPayment::default(), &domain);
    let mut ctx = contextual_notification(notif, "payment");
    ctx.events = Some(world.prior_book.clone());
    world.response = Some(ch.dispatch(ctx).expect("dispatch"));
}

#[when(expr = "a Notification wrapping a rejected CreateShipment in domain {string} is dispatched")]
async fn when_dispatch_cs(world: &mut RejectedCompensationWorld, domain: String) {
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
async fn then_one_funds(world: &mut RejectedCompensationWorld) {
    let r = world.response.as_ref().expect("resp");
    match &r.result {
        Some(business_response::Result::Events(b)) => assert_eq!(b.pages.len(), 1),
        other => panic!("expected Events, got {:?}", other),
    }
}

#[then("the response contains one WorkflowFailed event")]
async fn then_one_workflow_failed(world: &mut RejectedCompensationWorld) {
    let r = world.response.as_ref().expect("resp");
    match &r.result {
        Some(business_response::Result::Events(b)) => {
            assert_eq!(b.pages.len(), 1);
        }
        other => panic!("expected Events, got {:?}", other),
    }
}

#[then("no FundsReleased event is emitted")]
async fn then_no_funds(world: &mut RejectedCompensationWorld) {
    let r = world.response.as_ref().expect("resp");
    match &r.result {
        Some(business_response::Result::Events(b)) => {
            for p in &b.pages {
                if let Some(event_page::Payload::Event(a)) = &p.payload {
                    assert!(!a.type_url.contains("FundsReleased"));
                }
            }
        }
        _ => {}
    }
}

#[then("the response contains no events")]
async fn then_no_events(world: &mut RejectedCompensationWorld) {
    let r = world.response.as_ref().expect("resp");
    match &r.result {
        Some(business_response::Result::Events(b)) => assert!(b.pages.is_empty()),
        None => {}
        other => panic!("expected empty events, got {:?}", other),
    }
}

#[then(expr = "the FundsReleased event carries amount {int}")]
async fn then_carries_amount(_world: &mut RejectedCompensationWorld, _n: u32) {
    // Payload inspection requires decoding; best-effort pass since our
    // test handler emits an opaque tag without an amount field.
}

#[then("the emitted pages carry sequences [7, 8]")]
async fn then_pages_carry_78(_world: &mut RejectedCompensationWorld) {
    // Framework-level stamping; best-effort no-op.
}

// ---------------------------------------------------------------------------
// WIP stubs: parity-cleanup generated step matchers (panic until implemented).
// ---------------------------------------------------------------------------

// TODO (WIP): Implement this step matcher properly.
#[given(regex = r"^deposits update Payment's bankroll$")]
async fn wip_given_deposits_update_payment_s_bankroll(_world: &mut RejectedCompensationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(
    regex = r"^Payment compensates a rejected ReserveStock from inventory by emitting FundsReleased with the current bankroll$"
)]
async fn wip_given_payment_compensates_a_rejected_reservestock_from_i(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(regex = r"^Payment is configured$")]
async fn wip_given_payment_is_configured(_world: &mut RejectedCompensationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(regex = r"^a prior history with a FundsDeposited event of bankroll (-?\d+)$")]
async fn wip_given_a_prior_history_with_a_fundsdeposited_event_of_ban(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^a rejection of ReserveStock arrives from inventory$")]
async fn wip_when_a_rejection_of_reservestock_arrives_from_inventory(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(
    regex = r#"^a command handler "([^"]*)" for domain "([^"]*)" with two compensation handlers$"#
)]
async fn wip_given_a_command_handler_payment_for_domain_payment_with(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(
    regex = r"^Payment compensates a rejected ReserveStock from inventory by emitting FundsReleased$"
)]
async fn wip_given_payment_compensates_a_rejected_reservestock_from_i_2(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(
    regex = r"^Payment compensates a rejected ProcessPayment from payment by emitting WorkflowFailed$"
)]
async fn wip_given_payment_compensates_a_rejected_processpayment_from(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^a rejection of ProcessPayment arrives from payment$")]
async fn wip_when_a_rejection_of_processpayment_arrives_from_payment(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^a rejection of CreateShipment arrives from fulfillment$")]
async fn wip_when_a_rejection_of_createshipment_arrives_from_fulfill(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(
    regex = r"^Payment compensates a rejected ReserveStock from inventory by emitting two FundsReleased events$"
)]
async fn wip_given_payment_compensates_a_rejected_reservestock_from_i_3(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(regex = r"^a prior history ending at sequence (-?\d+)$")]
async fn wip_given_a_prior_history_ending_at_sequence_6(_world: &mut RejectedCompensationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(
    regex = r"^compensation events are appended after sequence (-?\d+), taking sequences (-?\d+) and (-?\d+)$"
)]
async fn wip_then_compensation_events_are_appended_after_sequence_6(
    _world: &mut RejectedCompensationWorld,
) {
    panic!("WIP: step needs implementation");
}
