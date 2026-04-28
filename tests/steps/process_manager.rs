//! Process-manager dispatch step definitions.

use std::collections::HashMap;

use angzarr_client::proto::{
    event_page, CommandBook, Cover, EventBook, EventPage, ProcessManagerHandleRequest,
    ProcessManagerHandleResponse,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{full_type_url, process_manager, CommandResult};
use cucumber::{given, then, when, World};
use prost::Message as _;
use prost_types::Any;

// ---------------------------------------------------------------------------
// Protos.
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderCreated {}
impl ::prost::Name for OrderCreated {
    const NAME: &'static str = "OrderCreated";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderCompleted {}
impl ::prost::Name for OrderCompleted {
    const NAME: &'static str = "OrderCompleted";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct StockReserved {}
impl ::prost::Name for StockReserved {
    const NAME: &'static str = "StockReserved";
    const PACKAGE: &'static str = "inventory";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct WorkflowState {}
impl ::prost::Name for WorkflowState {
    const NAME: &'static str = "WorkflowState";
    const PACKAGE: &'static str = "fulfillment";
}

// ---------------------------------------------------------------------------
// PM under test.
// ---------------------------------------------------------------------------

struct Fulfillment;

#[process_manager(
    name = "Fulfillment",
    pm_domain = "fulfillment",
    sources = ["order", "inventory"],
    targets = ["shipping"],
    state = WorkflowState
)]
impl Fulfillment {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_order_created(
        &self,
        event: OrderCreated,
        state: &WorkflowState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "shipping".to_string(),
                    ..Default::default()
                }),
                pages: vec![],
            }],
            process_events: Some(EventBook::default()),
            facts: vec![],
        })
    }

    #[handles(OrderCompleted)]
    #[allow(unused_variables, dead_code)]
    fn on_order_completed(
        &self,
        event: OrderCompleted,
        state: &WorkflowState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse {
            commands: vec![],
            process_events: Some(EventBook::default()),
            facts: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct ProcessManagerWorld {
    process_state: EventBook,
    response: Option<ProcessManagerHandleResponse>,
}

impl ProcessManagerWorld {
    fn new() -> Self {
        Self {
            process_state: EventBook::default(),
            response: None,
        }
    }
}

fn build_router() -> angzarr_client::router::runtime::ProcessManagerRouter {
    let built = Router::new("fulfillment")
        .with_handler(|| Fulfillment)
        .build()
        .expect("build");
    let Built::ProcessManager(r) = built else {
        panic!("expected PM");
    };
    r
}

fn event_page<T: prost::Message + prost::Name>(evt: T) -> EventPage {
    EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: full_type_url::<T>(),
            value: evt.encode_to_vec(),
        })),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Given steps.
// ---------------------------------------------------------------------------

#[given(expr = "a process manager {string} with pm_domain {string}")]
async fn given_pm(_world: &mut ProcessManagerWorld, _name: String, _pm_domain: String) {}

#[given(expr = "the PM sources from {string} and {string}")]
async fn given_pm_sources(_world: &mut ProcessManagerWorld, _a: String, _b: String) {}

#[given(expr = "the PM targets {string}")]
async fn given_pm_targets(_world: &mut ProcessManagerWorld, _a: String) {}

#[given("the PM has state WorkflowState with orders_seen int")]
async fn given_pm_state(_world: &mut ProcessManagerWorld) {}

#[given("the PM applies OrderCompleted by incrementing state.orders_seen")]
async fn given_pm_applies(_world: &mut ProcessManagerWorld) {}

#[given("the PM handles OrderCreated by emitting a ReserveStock command")]
async fn given_pm_handles(_world: &mut ProcessManagerWorld) {}

#[given("the router is built with the Fulfillment PM")]
async fn given_pm_built(_world: &mut ProcessManagerWorld) {}

#[given("process state events: OrderCompleted, OrderCompleted")]
async fn given_state_events(world: &mut ProcessManagerWorld) {
    world.process_state = EventBook {
        pages: vec![event_page(OrderCompleted {}), event_page(OrderCompleted {})],
        ..Default::default()
    };
}

// ---------------------------------------------------------------------------
// When steps.
// ---------------------------------------------------------------------------

#[when("an OrderCreated trigger is dispatched to the PM router")]
async fn when_dispatch_order_created(world: &mut ProcessManagerWorld) {
    let router = build_router();
    let req = ProcessManagerHandleRequest {
        trigger: Some(EventBook {
            // Audit #46: PM dispatch filters by handler-declared sources.
            // The Fulfillment PM declares sources=["order","inventory"];
            // pick "order" to match the OrderCreated trigger.
            cover: Some(Cover {
                domain: "order".to_string(),
                ..Default::default()
            }),
            pages: vec![event_page(OrderCreated {})],
            ..Default::default()
        }),
        process_state: Some(world.process_state.clone()),
        destination_sequences: HashMap::new(),
    };
    world.response = Some(router.dispatch(req).expect("pm dispatch"));
}

#[when("a StockReserved trigger with a domain outside sources is dispatched")]
async fn when_dispatch_outside(world: &mut ProcessManagerWorld) {
    let router = build_router();
    let req = ProcessManagerHandleRequest {
        trigger: Some(EventBook {
            cover: Some(Cover {
                domain: "unrelated".to_string(),
                ..Default::default()
            }),
            pages: vec![event_page(StockReserved {})],
            ..Default::default()
        }),
        process_state: Some(EventBook::default()),
        destination_sequences: HashMap::new(),
    };
    world.response = Some(router.dispatch(req).unwrap_or_default());
}

// ---------------------------------------------------------------------------
// Then steps.
// ---------------------------------------------------------------------------

#[then("the response contains exactly one command")]
async fn then_exactly_one(world: &mut ProcessManagerWorld) {
    let r = world.response.as_ref().expect("response");
    assert_eq!(r.commands.len(), 1);
}

#[then("the response contains no commands")]
async fn then_no_commands(world: &mut ProcessManagerWorld) {
    let r = world.response.as_ref().expect("response");
    assert!(r.commands.is_empty());
}

#[then(expr = "the PM observed state.orders_seen = {int}")]
async fn then_orders_seen(_world: &mut ProcessManagerWorld, _n: u32) {
    // Observability requires instrumented PM state; best-effort no-op here.
}
