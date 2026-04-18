//! R11 — saga dispatch.
//!
//! Saga handlers translate events from a source domain into commands/facts
//! for a target domain. `SagaRouter::dispatch` wraps the handler output into
//! `SagaResponse`. Multiple saga handlers claiming the same event merge
//! their commands and events.

use angzarr_client::proto::{
    event_page, CommandBook, Cover, EventBook, EventPage, SagaHandleRequest, SagaResponse,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{full_type_url, saga, CommandResult};

use prost::Message;
use prost_types::Any;

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderPlaced {}
impl ::prost::Name for OrderPlaced {
    const NAME: &'static str = "OrderPlaced";
    const PACKAGE: &'static str = "order";
}

struct OrderSaga;

#[saga(name = "saga-order-fulfillment", source = "order", target = "inventory")]
impl OrderSaga {
    #[handles(OrderPlaced)]
    #[allow(unused_variables, dead_code)]
    fn on_order_placed(&self, event: OrderPlaced) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "inventory".to_string(),
                    ..Default::default()
                }),
                pages: vec![],
            }],
            events: vec![],
        })
    }
}

fn order_placed_saga_request() -> SagaHandleRequest {
    let evt = Any {
        type_url: full_type_url::<OrderPlaced>(),
        value: OrderPlaced {}.encode_to_vec(),
    };
    SagaHandleRequest {
        source: Some(EventBook {
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(evt)),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn saga_dispatch_wraps_handler_output_into_saga_response() {
    let built = Router::new("saga-order-fulfillment")
        .with_handler(|| OrderSaga)
        .build()
        .unwrap();
    let Built::Saga(router) = built else {
        panic!("expected Saga variant");
    };
    let response = router.dispatch(order_placed_saga_request()).unwrap();
    assert_eq!(response.commands.len(), 1);
    assert_eq!(
        response.commands[0].cover.as_ref().unwrap().domain,
        "inventory"
    );
}

// Second saga handling the same event into a different target.
struct AuditSaga;

#[saga(name = "saga-audit", source = "order", target = "audit")]
impl AuditSaga {
    #[handles(OrderPlaced)]
    #[allow(unused_variables, dead_code)]
    fn on_order_placed(&self, event: OrderPlaced) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![],
            events: vec![EventBook {
                cover: Some(Cover {
                    domain: "audit".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
        })
    }
}

#[test]
fn multi_saga_handlers_merge_commands_and_events_in_registration_order() {
    let built = Router::new("saga-fanout")
        .with_handler(|| OrderSaga) // emits 1 command (inventory)
        .with_handler(|| AuditSaga) // emits 1 event (audit)
        .build()
        .unwrap();
    let Built::Saga(router) = built else {
        panic!("expected Saga variant");
    };
    let response = router.dispatch(order_placed_saga_request()).unwrap();
    assert_eq!(response.commands.len(), 1);
    assert_eq!(
        response.commands[0].cover.as_ref().unwrap().domain,
        "inventory"
    );
    assert_eq!(response.events.len(), 1);
    assert_eq!(response.events[0].cover.as_ref().unwrap().domain, "audit");
}
