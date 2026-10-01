//! Step definitions for `features/client/rejected_compensation.feature`.
//!
//! Rejections are delivered as real Notification commands through a
//! `CommandHandlerRouter` built from `#[command_handler]` types with
//! `#[rejected]` compensation methods.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use angzarr_client::proto::{
    business_response, page_header::SequenceType, BusinessResponse, Cover, EventBook, EventPage,
    Notification, PageHeader,
};
use angzarr_client::router::runtime::CommandHandlerRouter;
use angzarr_client::router::{Built, Router};
use angzarr_client::{command_handler, CommandResult};
use cucumber::{given, then, when, World};

use super::deferred::{event_page_of, events_of, rejection_delivery};
use crate::common::fixtures::{
    CreateShipment, FundsDeposited, FundsReleased, ProcessPayment, ReserveStock, WorkflowFailed,
};

#[derive(Default)]
pub struct PaymentState {
    bankroll: i64,
}

fn events_response(pages: Vec<EventPage>) -> BusinessResponse {
    BusinessResponse {
        result: Some(business_response::Result::Events(EventBook {
            pages,
            ..Default::default()
        })),
    }
}

/// Payment with a stateful ReserveStock compensation emitting `releases`
/// FundsReleased events, each carrying the rebuilt bankroll.
pub struct StatefulPayment {
    releases: Arc<AtomicU32>,
}

#[command_handler(domain = "payment", state = PaymentState)]
impl StatefulPayment {
    #[applies(FundsDeposited)]
    fn on_deposit(state: &mut PaymentState, evt: FundsDeposited) {
        state.bankroll = evt.new_bankroll;
    }

    #[rejected(domain = "inventory", command = "ReserveStock")]
    fn on_reserve_stock_rejected(
        &self,
        _notification: &Notification,
        state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        let n = self.releases.load(Ordering::SeqCst);
        let pages = (0..n)
            .map(|_| {
                event_page_of(&FundsReleased {
                    amount: state.bankroll,
                    reason: "stock rejected".into(),
                })
            })
            .collect();
        Ok(events_response(pages))
    }
}

/// Payment with two compensation methods for different rejections.
pub struct TwoCompensations;

#[command_handler(domain = "payment", state = PaymentState)]
impl TwoCompensations {
    #[rejected(domain = "inventory", command = "ReserveStock")]
    fn on_reserve_stock_rejected(
        &self,
        _notification: &Notification,
        _state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(events_response(vec![event_page_of(&FundsReleased {
            amount: 0,
            reason: "stock rejected".into(),
        })]))
    }

    #[rejected(domain = "payment", command = "ProcessPayment")]
    fn on_process_payment_rejected(
        &self,
        _notification: &Notification,
        _state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(events_response(vec![event_page_of(&WorkflowFailed {
            reason: "payment rejected".into(),
            failed_domain: "payment".into(),
            failed_command: "ProcessPayment".into(),
        })]))
    }
}

/// Payment with no compensation methods.
pub struct NoCompensation;

#[command_handler(domain = "payment", state = PaymentState)]
impl NoCompensation {
    #[handles(ProcessPayment)]
    fn on_process_payment(
        &self,
        _cmd: ProcessPayment,
        _state: &PaymentState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Variant {
    #[default]
    Stateful,
    Two,
    None,
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct RejectedCompensationWorld {
    variant: Variant,
    releases: Arc<AtomicU32>,
    prior: Option<EventBook>,
    response: Option<BusinessResponse>,
}

impl RejectedCompensationWorld {
    fn new() -> Self {
        Self {
            variant: Variant::Stateful,
            releases: Arc::new(AtomicU32::new(1)),
            prior: None,
            response: None,
        }
    }

    fn router(&self) -> CommandHandlerRouter {
        let releases = Arc::clone(&self.releases);
        let built = match self.variant {
            Variant::Stateful => Router::new("payment")
                .with_handler(move || StatefulPayment {
                    releases: Arc::clone(&releases),
                })
                .build(),
            Variant::Two => Router::new("payment")
                .with_handler(|| TwoCompensations)
                .build(),
            Variant::None => Router::new("payment")
                .with_handler(|| NoCompensation)
                .build(),
        }
        .expect("router builds");
        match built {
            Built::CommandHandler(r) => r,
            other => panic!("expected a command-handler router, got {other:?}"),
        }
    }

    fn deliver<M: prost::Message + prost::Name>(&mut self, rejected: &M, domain: &str) {
        let delivery = rejection_delivery(rejected, domain, "payment", self.prior.clone());
        self.response = Some(
            self.router()
                .dispatch(delivery)
                .expect("rejection dispatch"),
        );
    }

    fn events(&self) -> &EventBook {
        match self.response.as_ref().and_then(|r| r.result.as_ref()) {
            Some(business_response::Result::Events(book)) => book,
            other => panic!("expected an Events response, got {other:?}"),
        }
    }
}

fn prior_page(seq: u32, deposit: i64) -> EventPage {
    EventPage {
        header: Some(PageHeader {
            sequence_type: Some(SequenceType::Sequence(seq)),
            sync_mode: None,
        }),
        ..event_page_of(&FundsDeposited {
            new_bankroll: deposit,
        })
    }
}

// --- Given -----------------------------------------------------------------

#[given(expr = "a command handler {string} for domain {string} with stateful rejection")]
fn given_stateful(world: &mut RejectedCompensationWorld, name: String, domain: String) {
    assert_eq!((name.as_str(), domain.as_str()), ("Payment", "payment"));
    world.variant = Variant::Stateful;
}

#[given(expr = "a command handler {string} for domain {string} with two compensation handlers")]
fn given_two(world: &mut RejectedCompensationWorld, name: String, domain: String) {
    assert_eq!((name.as_str(), domain.as_str()), ("Payment", "payment"));
    world.variant = Variant::Two;
}

#[given(expr = "a command handler {string} for domain {string} with no rejection handlers")]
fn given_none(world: &mut RejectedCompensationWorld, name: String, domain: String) {
    assert_eq!((name.as_str(), domain.as_str()), ("Payment", "payment"));
    world.variant = Variant::None;
}

#[given("deposits update Payment's bankroll")]
fn given_deposits(world: &mut RejectedCompensationWorld) {
    assert_eq!(world.variant, Variant::Stateful);
}

#[given(
    "Payment compensates a rejected ReserveStock from inventory by emitting FundsReleased with the current bankroll"
)]
fn given_release_bankroll(world: &mut RejectedCompensationWorld) {
    world.releases.store(1, Ordering::SeqCst);
}

#[given("Payment compensates a rejected ReserveStock from inventory by emitting FundsReleased")]
fn given_release(world: &mut RejectedCompensationWorld) {
    assert_eq!(world.variant, Variant::Two);
}

#[given("Payment compensates a rejected ProcessPayment from payment by emitting WorkflowFailed")]
fn given_workflow_failed(world: &mut RejectedCompensationWorld) {
    assert_eq!(world.variant, Variant::Two);
}

#[given(
    "Payment compensates a rejected ReserveStock from inventory by emitting two FundsReleased events"
)]
fn given_two_releases(world: &mut RejectedCompensationWorld) {
    world.releases.store(2, Ordering::SeqCst);
}

#[given("Payment is configured")]
fn given_configured(world: &mut RejectedCompensationWorld) {
    let router = world.router();
    assert_eq!(router.name(), "payment");
    assert_eq!(router.handler_count(), 1);
}

#[given(expr = "a prior history with a FundsDeposited event of bankroll {int}")]
fn given_prior_bankroll(world: &mut RejectedCompensationWorld, bankroll: i64) {
    world.prior = Some(EventBook {
        cover: Some(Cover {
            domain: "payment".into(),
            ..Default::default()
        }),
        pages: vec![prior_page(0, bankroll)],
        next_sequence: 1,
        ..Default::default()
    });
}

#[given(expr = "a prior history ending at sequence {int}")]
fn given_prior_ending(world: &mut RejectedCompensationWorld, last: u32) {
    world.prior = Some(EventBook {
        cover: Some(Cover {
            domain: "payment".into(),
            ..Default::default()
        }),
        pages: (0..=last).map(|seq| prior_page(seq, 10)).collect(),
        next_sequence: last + 1,
        ..Default::default()
    });
}

// --- When ------------------------------------------------------------------

#[when("a rejection of ReserveStock arrives from inventory")]
fn when_reserve_stock(world: &mut RejectedCompensationWorld) {
    world.deliver(&ReserveStock::default(), "inventory");
}

#[when("a rejection of ProcessPayment arrives from payment")]
fn when_process_payment(world: &mut RejectedCompensationWorld) {
    world.deliver(&ProcessPayment::default(), "payment");
}

#[when("a rejection of CreateShipment arrives from fulfillment")]
fn when_create_shipment(world: &mut RejectedCompensationWorld) {
    world.deliver(&CreateShipment::default(), "fulfillment");
}

// --- Then ------------------------------------------------------------------

#[then("the response contains one FundsReleased event")]
fn then_one_release(world: &mut RejectedCompensationWorld) {
    let book = world.events();
    assert_eq!(book.pages.len(), 1);
    assert_eq!(events_of::<FundsReleased>(book).len(), 1);
}

#[then(expr = "the FundsReleased event carries amount {int}")]
fn then_amount(world: &mut RejectedCompensationWorld, amount: i64) {
    let released = events_of::<FundsReleased>(world.events());
    assert_eq!(
        released.iter().map(|e| e.amount).collect::<Vec<_>>(),
        vec![amount]
    );
}

#[then("the response contains one WorkflowFailed event")]
fn then_one_workflow_failed(world: &mut RejectedCompensationWorld) {
    let failed = events_of::<WorkflowFailed>(world.events());
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].failed_command, "ProcessPayment");
}

#[then("no FundsReleased event is emitted")]
fn then_no_release(world: &mut RejectedCompensationWorld) {
    assert!(events_of::<FundsReleased>(world.events()).is_empty());
}

#[then("the response contains no events")]
fn then_no_events(world: &mut RejectedCompensationWorld) {
    assert!(world.events().pages.is_empty());
}

#[then(
    expr = "compensation events are appended after sequence {int}, taking sequences {int} and {int}"
)]
fn then_sequences(world: &mut RejectedCompensationWorld, last: u32, first: u32, second: u32) {
    let book = world.events();
    assert_eq!(first, last + 1);
    let seqs: Vec<Option<SequenceType>> = book
        .pages
        .iter()
        .map(|p| p.header.as_ref().and_then(|h| h.sequence_type.clone()))
        .collect();
    assert_eq!(
        seqs,
        vec![
            Some(SequenceType::Sequence(first)),
            Some(SequenceType::Sequence(second))
        ]
    );
    assert_eq!(book.next_sequence, second + 1);
}
