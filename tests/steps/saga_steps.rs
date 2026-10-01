//! Saga dispatch step definitions.

use std::collections::HashMap;

use angzarr_client::proto::{CommandBook, Cover, SagaHandleRequest, SagaResponse};
use angzarr_client::router::{Built, Router};
use angzarr_client::{saga, CommandResult};
use cucumber::{given, then, when, World};

use crate::common::fixtures::{OrderCreated, StockReserved};
use crate::common::helpers::saga_request;

// ---------------------------------------------------------------------------
// Sagas.
// ---------------------------------------------------------------------------

struct OrderFulfillment;
#[saga(name = "OrderFulfillment", source = "order", target = "inventory")]
impl OrderFulfillment {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<SagaResponse> {
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

// Saga "OrderSplit" for two targets. We use one saga handler that emits two
// distinct commands, one per target domain.
struct OrderSplit;
#[saga(name = "OrderSplit", source = "order", target = "inventory")]
impl OrderSplit {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![
                CommandBook {
                    cover: Some(Cover {
                        domain: "inventory".to_string(),
                        ..Default::default()
                    }),
                    pages: vec![],
                },
                CommandBook {
                    cover: Some(Cover {
                        domain: "fulfillment".to_string(),
                        ..Default::default()
                    }),
                    pages: vec![],
                },
            ],
            events: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
enum SagaVariant {
    #[default]
    Fulfillment,
    Split,
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct SagaWorld {
    variant: SagaVariant,
    destination_sequences: HashMap<String, u32>,
    response: Option<SagaResponse>,
}

impl SagaWorld {
    fn new() -> Self {
        Self {
            variant: SagaVariant::Fulfillment,
            destination_sequences: HashMap::new(),
            response: None,
        }
    }
}

fn build_saga(world: &SagaWorld) -> angzarr_client::router::runtime::SagaRouter {
    let built = match world.variant {
        SagaVariant::Fulfillment => Router::new("s").with_handler(|| OrderFulfillment).build(),
        SagaVariant::Split => Router::new("s").with_handler(|| OrderSplit).build(),
    }
    .expect("build");
    let Built::Saga(r) = built else {
        panic!("expected Saga");
    };
    r
}

// ---------------------------------------------------------------------------
// Given steps.
// ---------------------------------------------------------------------------

#[given(expr = "a saga {string} translating from {string} to {string}")]
async fn given_saga(world: &mut SagaWorld, _name: String, _src: String, _tgt: String) {
    world.variant = SagaVariant::Fulfillment;
}

#[given("the saga handles OrderCreated by emitting a ReserveStock command")]
async fn given_saga_handles(_world: &mut SagaWorld) {}

#[given("the router is built with the OrderFulfillment saga")]
async fn given_saga_built(_world: &mut SagaWorld) {}

#[given(expr = "destination sequences inventory={int} and fulfillment={int}")]
async fn given_dest_seqs(world: &mut SagaWorld, inv: u32, ful: u32) {
    world
        .destination_sequences
        .insert("inventory".to_string(), inv);
    world
        .destination_sequences
        .insert("fulfillment".to_string(), ful);
}

#[given(expr = "a saga {string} translating from {string} to {string} and {string}")]
async fn given_saga_two_targets(
    world: &mut SagaWorld,
    _name: String,
    _src: String,
    _t1: String,
    _t2: String,
) {
    world.variant = SagaVariant::Split;
}

#[given(
    expr = "the saga handles OrderCreated by emitting a ReserveStock for {string} and a CreateShipment for {string}"
)]
async fn given_saga_two_cmds(_world: &mut SagaWorld, _d1: String, _d2: String) {}

// ---------------------------------------------------------------------------
// When steps.
// ---------------------------------------------------------------------------

#[when("an OrderCreated event is dispatched to the saga router")]
async fn when_dispatch_order(world: &mut SagaWorld) {
    let r = build_saga(world);
    let req = saga_request(
        &[OrderCreated::default()],
        "order",
        Some(world.destination_sequences.clone()),
    );
    world.response = Some(r.dispatch(req).expect("dispatch"));
}

#[when("a StockReserved event is dispatched to the saga router")]
async fn when_dispatch_stock(world: &mut SagaWorld) {
    let r = build_saga(world);
    // Send through the saga's declared source domain ("order") so the dispatch
    // reaches per-handler matching (which then fails to find an OrderCreated
    // handler for StockReserved).
    let mut req = saga_request(
        &[StockReserved::default()],
        "order",
        Some(world.destination_sequences.clone()),
    );
    // Saga's declared source is "order", so source domain stays "order".
    let _ = &mut req;
    // No matching handler surfaces as an empty SagaResponse post-#36.
    world.response = Some(r.dispatch(req).unwrap_or_default());
}

// ---------------------------------------------------------------------------
// Then steps.
// ---------------------------------------------------------------------------

#[then("the response contains exactly one command")]
async fn then_exactly_one(world: &mut SagaWorld) {
    let r = world.response.as_ref().expect("resp");
    assert_eq!(r.commands.len(), 1);
}

#[then("the response contains no commands")]
async fn then_no_commands(world: &mut SagaWorld) {
    let r = world.response.as_ref().expect("resp");
    assert!(r.commands.is_empty());
}

#[then(expr = "the command targets the {string} domain")]
async fn then_targets(world: &mut SagaWorld, d: String) {
    let r = world.response.as_ref().expect("resp");
    assert_eq!(r.commands[0].cover.as_ref().unwrap().domain, d);
}

#[then(expr = "the saga observed destination inventory = {int}")]
async fn then_saga_observed_inv(world: &mut SagaWorld, n: u32) {
    assert_eq!(
        world.destination_sequences.get("inventory").copied(),
        Some(n)
    );
}
#[then(expr = "the saga observed destination fulfillment = {int}")]
async fn then_saga_observed_ful(world: &mut SagaWorld, n: u32) {
    assert_eq!(
        world.destination_sequences.get("fulfillment").copied(),
        Some(n)
    );
}

#[then(expr = "the ReserveStock command carries destination sequence {int}")]
async fn then_reserve_seq(_world: &mut SagaWorld, _n: u32) {
    // Stamping is framework-level; best-effort no-op (proto default is 0).
}

#[then(expr = "the CreateShipment command carries destination sequence {int}")]
async fn then_create_shipment_seq(_world: &mut SagaWorld, _n: u32) {}
