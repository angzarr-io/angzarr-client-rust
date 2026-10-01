//! Step definitions for `features/client/saga.feature`.
//!
//! Sagas are real `#[saga]` types dispatched through a `SagaRouter`; what a
//! saga observed during dispatch is recorded on its [`SagaProbe`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use angzarr_client::proto::{SagaHandleRequest, SagaResponse};
use angzarr_client::router::runtime::SagaRouter;
use angzarr_client::router::{Built, Router};
use angzarr_client::{saga, CommandResult};
use cucumber::{given, then, when, World};

use super::deferred::{
    command_of, deferred_header, has_explicit_sequence, root_for, trigger_book, unsequenced_command,
};
use crate::common::fixtures::{CreateShipment, OrderCreated, ReserveStock, StockReserved};

/// What a saga saw while handling one event.
#[derive(Debug, Default)]
pub struct SagaProbe {
    /// Destination heads the handler observed, by domain.
    observed_destinations: Mutex<Option<HashMap<String, u32>>>,
}

pub struct OrderFulfillment {
    probe: Arc<SagaProbe>,
}

#[saga(name = "OrderFulfillment", source = "order", target = "inventory")]
impl OrderFulfillment {
    #[handles(OrderCreated)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![unsequenced_command(
                &ReserveStock {
                    order_id: event.order_id,
                    sku: "sku-1".into(),
                    quantity: 1,
                },
                "inventory",
            )],
            events: vec![],
        })
    }
}

pub struct OrderSplit {
    probe: Arc<SagaProbe>,
}

#[saga(name = "OrderSplit", source = "order", target = "inventory")]
impl OrderSplit {
    #[handles(OrderCreated)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![
                unsequenced_command(
                    &ReserveStock {
                        order_id: event.order_id.clone(),
                        sku: "sku-1".into(),
                        quantity: 1,
                    },
                    "inventory",
                ),
                unsequenced_command(
                    &CreateShipment {
                        order_id: event.order_id,
                        address: "addr".into(),
                    },
                    "fulfillment",
                ),
            ],
            events: vec![],
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Variant {
    #[default]
    Fulfillment,
    Split,
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct SagaWorld {
    variant: Variant,
    probe: Arc<SagaProbe>,
    destination_sequences: HashMap<String, u32>,
    trigger_root: String,
    trigger_seq: u32,
    response: Option<SagaResponse>,
}

impl SagaWorld {
    fn new() -> Self {
        Self {
            variant: Variant::Fulfillment,
            probe: Arc::default(),
            destination_sequences: HashMap::new(),
            trigger_root: "order-1".into(),
            trigger_seq: 0,
            response: None,
        }
    }

    fn router(&self) -> SagaRouter {
        let probe = Arc::clone(&self.probe);
        let built = match self.variant {
            Variant::Fulfillment => Router::new("sagas")
                .with_handler(move || OrderFulfillment {
                    probe: Arc::clone(&probe),
                })
                .build(),
            Variant::Split => Router::new("sagas")
                .with_handler(move || OrderSplit {
                    probe: Arc::clone(&probe),
                })
                .build(),
        }
        .expect("router builds");
        match built {
            Built::Saga(r) => r,
            other => panic!("expected a saga router, got {other:?}"),
        }
    }

    fn dispatch<M: prost::Message + prost::Name>(&mut self, event: &M) {
        let request = SagaHandleRequest {
            source: Some(trigger_book(
                event,
                "order",
                &self.trigger_root,
                self.trigger_seq,
            )),
            destination_sequences: self.destination_sequences.clone(),
            ..Default::default()
        };
        self.response = Some(self.router().dispatch(request).expect("saga dispatch"));
    }

    fn response(&self) -> &SagaResponse {
        self.response.as_ref().expect("saga response")
    }
}

// --- Given -----------------------------------------------------------------

#[given(expr = "a saga {string} translating from {string} to {string}")]
fn given_saga(world: &mut SagaWorld, name: String, source: String, target: String) {
    assert_eq!(
        (name.as_str(), source.as_str(), target.as_str()),
        ("OrderFulfillment", "order", "inventory")
    );
    world.variant = Variant::Fulfillment;
}

#[given(expr = "a saga {string} translating from {string} to {string} and {string}")]
fn given_split_saga(world: &mut SagaWorld, name: String, source: String, t1: String, t2: String) {
    assert_eq!(
        (name.as_str(), source.as_str(), t1.as_str(), t2.as_str()),
        ("OrderSplit", "order", "inventory", "fulfillment")
    );
    world.variant = Variant::Split;
}

#[given("the saga handles OrderCreated by emitting a ReserveStock command")]
fn given_handles(world: &mut SagaWorld) {
    assert_eq!(world.variant, Variant::Fulfillment);
}

#[given(
    expr = "the saga handles OrderCreated by emitting a ReserveStock for {string} and a CreateShipment for {string}"
)]
fn given_handles_split(world: &mut SagaWorld, d1: String, d2: String) {
    assert_eq!((d1.as_str(), d2.as_str()), ("inventory", "fulfillment"));
    assert_eq!(world.variant, Variant::Split);
}

#[given("the router is built with the OrderFulfillment saga")]
fn given_built(world: &mut SagaWorld) {
    let router = world.router();
    assert_eq!(router.name(), "OrderFulfillment");
    assert_eq!(router.handler_count(), 1);
}

#[given(expr = "destination sequences inventory={int} and fulfillment={int}")]
fn given_two_heads(world: &mut SagaWorld, inventory: u32, fulfillment: u32) {
    world.destination_sequences = HashMap::from([
        ("inventory".to_string(), inventory),
        ("fulfillment".to_string(), fulfillment),
    ]);
}

#[given(expr = "destination sequences {word}={int}")]
fn given_one_head(world: &mut SagaWorld, domain: String, head: u32) {
    world.destination_sequences = HashMap::from([(domain, head)]);
}

#[given(expr = "the OrderCreated event is at sequence {int} of order root {string}")]
fn given_trigger_position(world: &mut SagaWorld, seq: u32, root: String) {
    world.trigger_seq = seq;
    world.trigger_root = root;
}

// --- When ------------------------------------------------------------------

#[when(regex = r"^(?:an|the) OrderCreated event is dispatched to the saga router$")]
fn when_order_created(world: &mut SagaWorld) {
    world.dispatch(&OrderCreated {
        order_id: "o-1".into(),
        ..Default::default()
    });
}

#[when("a StockReserved event is dispatched to the saga router")]
fn when_stock_reserved(world: &mut SagaWorld) {
    world.dispatch(&StockReserved::default());
}

// --- Then ------------------------------------------------------------------

#[then("the response contains exactly one command")]
fn then_one_command(world: &mut SagaWorld) {
    assert_eq!(world.response().commands.len(), 1);
}

#[then("the response contains no commands")]
fn then_no_commands(world: &mut SagaWorld) {
    assert!(world.response().commands.is_empty());
    assert!(world.response().events.is_empty());
}

#[then(expr = "the command targets the {string} domain")]
fn then_command_domain(world: &mut SagaWorld, domain: String) {
    let cmd = &world.response().commands[0];
    assert_eq!(
        cmd.cover.as_ref().map(|c| c.domain.as_str()),
        Some(domain.as_str())
    );
}

#[then(expr = "the saga observed destination {word} = {int}")]
fn then_observed(world: &mut SagaWorld, domain: String, head: u32) {
    let observed = world.probe.observed_destinations.lock().unwrap();
    let observed = observed.as_ref().expect("saga observed destinations");
    assert_eq!(observed.get(&domain).copied(), Some(head));
}

#[then(expr = "the ReserveStock command carries an angzarr_deferred header with basis_seq {int}")]
fn then_reserve_basis(world: &mut SagaWorld, basis: u32) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert_eq!(deferred_header(cmd).basis_seq, basis);
}

#[then(expr = "the CreateShipment command carries an angzarr_deferred header with basis_seq {int}")]
fn then_shipment_basis(world: &mut SagaWorld, basis: u32) {
    let cmd = command_of::<CreateShipment>(&world.response().commands);
    assert_eq!(deferred_header(cmd).basis_seq, basis);
}

#[then("the ReserveStock command carries an angzarr_deferred header")]
fn then_reserve_deferred(world: &mut SagaWorld) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    deferred_header(cmd);
}

#[then(expr = "the deferred source cover is domain {string} root {string}")]
fn then_source_cover(world: &mut SagaWorld, domain: String, root: String) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    let source = deferred_header(cmd)
        .source
        .as_ref()
        .expect("deferred source");
    assert_eq!(source.domain, domain);
    assert_eq!(
        source.root.as_ref().map(|r| r.value.clone()),
        Some(root_for(&root))
    );
}

#[then(expr = "the deferred source_seq is {int}")]
fn then_source_seq(world: &mut SagaWorld, seq: u32) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert_eq!(deferred_header(cmd).source_seq, seq);
}

#[then(expr = "the deferred command_index is {int}")]
fn then_command_index(world: &mut SagaWorld, index: u32) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert_eq!(deferred_header(cmd).command_index, index);
}

#[then(expr = "the deferred basis_seq is {int}")]
fn then_basis(world: &mut SagaWorld, basis: u32) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert_eq!(deferred_header(cmd).basis_seq, basis);
}

#[then("no page of the ReserveStock command has an explicit sequence")]
fn then_no_explicit(world: &mut SagaWorld) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert!(!cmd.pages.is_empty());
    assert!(!has_explicit_sequence(cmd), "pages: {:?}", cmd.pages);
}

#[then(expr = "the ReserveStock command's deferred command_index is {int}")]
fn then_reserve_index(world: &mut SagaWorld, index: u32) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert_eq!(deferred_header(cmd).command_index, index);
}

#[then(expr = "the CreateShipment command's deferred command_index is {int}")]
fn then_shipment_index(world: &mut SagaWorld, index: u32) {
    let cmd = command_of::<CreateShipment>(&world.response().commands);
    assert_eq!(deferred_header(cmd).command_index, index);
}
