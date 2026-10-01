//! Step definitions for `features/client/rejection.feature`.
//!
//! Payment (and optionally Payment2) are `#[command_handler]` types with a
//! `#[rejected]` compensation for ReserveStock rejected by inventory. Each
//! tags its FundsReleased with its own name so registration order is
//! observable.

use angzarr_client::proto::{business_response, BusinessResponse, EventBook, Notification};
use angzarr_client::router::CommandHandlerRouter;
use angzarr_client::router::{Built, Router};
use angzarr_client::{command_handler, CommandResult};
use cucumber::{given, then, when, World};

use super::deferred::{event_page_of, events_of, rejection_delivery};
use crate::common::fixtures::{FundsReleased, ProcessPayment, ReserveStock};

#[derive(Default)]
pub struct PaymentState;

fn release(by: &str) -> BusinessResponse {
    BusinessResponse {
        result: Some(business_response::Result::Events(EventBook {
            pages: vec![event_page_of(&FundsReleased {
                amount: 0,
                reason: by.into(),
            })],
            ..Default::default()
        })),
    }
}

pub struct Payment;

#[command_handler(domain = "payment", state = PaymentState)]
impl Payment {
    #[rejected(domain = "inventory", command = ReserveStock)]
    fn on_reserve_stock_rejected(
        &self,
        _notification: &Notification,
        _state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(release("Payment"))
    }
}

pub struct Payment2;

#[command_handler(domain = "payment", state = PaymentState)]
impl Payment2 {
    #[rejected(domain = "inventory", command = ReserveStock)]
    fn on_reserve_stock_rejected(
        &self,
        _notification: &Notification,
        _state: &PaymentState,
    ) -> CommandResult<BusinessResponse> {
        Ok(release("Payment2"))
    }
}

#[derive(Debug, Default, World)]
pub struct RejectionWorld {
    with_second: bool,
    response: Option<BusinessResponse>,
}

impl RejectionWorld {
    fn router(&self) -> CommandHandlerRouter {
        let router = Router::new("payment").with_handler(|| Payment);
        let router = if self.with_second {
            router.with_handler(|| Payment2)
        } else {
            router
        };
        match router.build().expect("router builds") {
            Built::CommandHandler(r) => r,
            other => panic!("expected a command-handler router, got {other:?}"),
        }
    }

    fn deliver<M: prost::Message + prost::Name>(&mut self, rejected: &M, domain: &str) {
        let delivery = rejection_delivery(rejected, domain, "payment", None);
        self.response = Some(
            self.router()
                .dispatch(delivery)
                .expect("rejection dispatch"),
        );
    }

    fn released_by(&self) -> Vec<String> {
        match self.response.as_ref().and_then(|r| r.result.as_ref()) {
            Some(business_response::Result::Events(book)) => {
                assert_eq!(
                    events_of::<FundsReleased>(book).len(),
                    book.pages.len(),
                    "only FundsReleased pages expected"
                );
                events_of::<FundsReleased>(book)
                    .into_iter()
                    .map(|e| e.reason)
                    .collect()
            }
            // No result: no compensation declared (DelegateToFramework).
            None => Vec::new(),
            other => panic!("expected an Events response, got {other:?}"),
        }
    }
}

// --- Given -----------------------------------------------------------------

#[given(expr = "Payment is a component in domain {string}")]
fn given_payment(_world: &mut RejectionWorld, domain: String) {
    let config = <Payment as angzarr_client::router::HandlerKind>::handler_config();
    let angzarr_client::router::HandlerConfig::CommandHandler { domain: d, .. } = config else {
        panic!("Payment is not a command handler");
    };
    assert_eq!(d, domain);
}

#[given("Payment compensates a rejected ReserveStock from inventory by releasing funds")]
fn given_compensates(_world: &mut RejectionWorld) {
    let config = <Payment as angzarr_client::router::HandlerKind>::handler_config();
    let angzarr_client::router::HandlerConfig::CommandHandler { compensates, .. } = config else {
        panic!("Payment is not a command handler");
    };
    assert_eq!(
        compensates,
        vec![format!(
            "inventory:{}",
            <ReserveStock as prost::Name>::full_name()
        )]
    );
}

#[given("Payment is the active component")]
fn given_active(world: &mut RejectionWorld) {
    world.with_second = false;
    assert_eq!(world.router().handler_count(), 1);
}

#[given("a second compensation handler for the same rejection also releases funds")]
fn given_second(_world: &mut RejectionWorld) {
    let config = <Payment2 as angzarr_client::router::HandlerKind>::handler_config();
    let angzarr_client::router::HandlerConfig::CommandHandler { compensates, .. } = config else {
        panic!("Payment2 is not a command handler");
    };
    assert_eq!(
        compensates,
        vec![format!(
            "inventory:{}",
            <ReserveStock as prost::Name>::full_name()
        )]
    );
}

#[given("Payment then Payment2 are configured")]
fn given_both(world: &mut RejectionWorld) {
    world.with_second = true;
    assert_eq!(world.router().handler_count(), 2);
}

// --- When ------------------------------------------------------------------

#[when("a rejection of ReserveStock arrives from inventory")]
fn when_reserve_stock(world: &mut RejectionWorld) {
    world.deliver(&ReserveStock::default(), "inventory");
}

#[when("a rejection of ProcessPayment arrives from inventory")]
fn when_process_payment(world: &mut RejectionWorld) {
    world.deliver(&ProcessPayment::default(), "inventory");
}

// --- Then ------------------------------------------------------------------

#[then("a FundsReleased event is emitted")]
fn then_released(world: &mut RejectionWorld) {
    assert_eq!(world.released_by(), vec!["Payment".to_string()]);
}

#[then("no events are emitted")]
fn then_none(world: &mut RejectionWorld) {
    assert!(world.released_by().is_empty());
}

#[then("two FundsReleased events are emitted in registration order")]
fn then_two(world: &mut RejectionWorld) {
    assert_eq!(
        world.released_by(),
        vec!["Payment".to_string(), "Payment2".to_string()]
    );
}
