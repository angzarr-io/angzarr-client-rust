//! Step definitions for `features/client/process_manager.feature`.
//!
//! The Fulfillment PM is a real `#[process_manager]` type dispatched through
//! a `ProcessManagerRouter`; the state it observed is recorded on its probe.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use angzarr_client::proto::{EventBook, ProcessManagerHandleRequest, ProcessManagerHandleResponse};
use angzarr_client::router::runtime::ProcessManagerRouter;
use angzarr_client::router::{Built, Router};
use angzarr_client::{process_manager, CommandResult};
use cucumber::{given, then, when, World};

use super::deferred::{
    command_of, deferred_header, has_explicit_sequence, root_for, trigger_book, unsequenced_command,
};
use crate::common::fixtures::{OrderCompleted, OrderCreated, ReserveStock, StockReserved};

#[derive(Default)]
pub struct WorkflowState {
    orders_seen: u32,
}

#[derive(Debug, Default)]
pub struct PmProbe {
    observed_orders_seen: Mutex<Option<u32>>,
}

pub struct Fulfillment {
    probe: Arc<PmProbe>,
}

#[process_manager(
    name = "Fulfillment",
    pm_domain = "fulfillment",
    sources = ["order", "inventory"],
    targets = ["shipping"],
    state = WorkflowState
)]
impl Fulfillment {
    #[applies(OrderCompleted)]
    fn on_completed(state: &mut WorkflowState, _evt: OrderCompleted) {
        state.orders_seen += 1;
    }

    #[handles(OrderCreated)]
    fn on_order_created(
        &self,
        event: OrderCreated,
        state: &WorkflowState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        *self.probe.observed_orders_seen.lock().unwrap() = Some(state.orders_seen);
        Ok(ProcessManagerHandleResponse {
            commands: vec![unsequenced_command(
                &ReserveStock {
                    order_id: event.order_id,
                    sku: "sku-1".into(),
                    quantity: 1,
                },
                "shipping",
            )],
            ..Default::default()
        })
    }

    #[handles(StockReserved)]
    fn on_stock_reserved(
        &self,
        _event: StockReserved,
        state: &WorkflowState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        *self.probe.observed_orders_seen.lock().unwrap() = Some(state.orders_seen);
        Ok(ProcessManagerHandleResponse {
            commands: vec![unsequenced_command(&ReserveStock::default(), "shipping")],
            ..Default::default()
        })
    }
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct ProcessManagerWorld {
    probe: Arc<PmProbe>,
    process_state: Vec<OrderCompleted>,
    destination_sequences: HashMap<String, u32>,
    trigger_root: String,
    trigger_seq: u32,
    response: Option<ProcessManagerHandleResponse>,
}

impl ProcessManagerWorld {
    fn new() -> Self {
        Self {
            probe: Arc::default(),
            process_state: Vec::new(),
            destination_sequences: HashMap::new(),
            trigger_root: "order-1".into(),
            trigger_seq: 0,
            response: None,
        }
    }

    fn router(&self) -> ProcessManagerRouter {
        let probe = Arc::clone(&self.probe);
        let built = Router::new("fulfillment")
            .with_handler(move || Fulfillment {
                probe: Arc::clone(&probe),
            })
            .build()
            .expect("router builds");
        match built {
            Built::ProcessManager(r) => r,
            other => panic!("expected a PM router, got {other:?}"),
        }
    }

    fn dispatch<M: prost::Message + prost::Name>(&mut self, event: &M, domain: &str) {
        let pages = self
            .process_state
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut book = trigger_book(e, "fulfillment", "pm-1", i as u32);
                book.pages.remove(0)
            })
            .collect::<Vec<_>>();
        let request = ProcessManagerHandleRequest {
            trigger: Some(trigger_book(
                event,
                domain,
                &self.trigger_root,
                self.trigger_seq,
            )),
            process_state: Some(EventBook {
                next_sequence: pages.len() as u32,
                pages,
                ..Default::default()
            }),
            destination_sequences: self.destination_sequences.clone(),
        };
        self.response = Some(self.router().dispatch(request).expect("pm dispatch"));
    }

    fn response(&self) -> &ProcessManagerHandleResponse {
        self.response.as_ref().expect("pm response")
    }
}

// --- Given -----------------------------------------------------------------

#[given(expr = "a process manager {string} for the fulfillment domain")]
fn given_pm(world: &mut ProcessManagerWorld, name: String) {
    assert_eq!(name, "Fulfillment");
    let router = world.router();
    assert_eq!(router.name(), "Fulfillment");
}

#[given(expr = "the PM sources from {string} and {string}")]
fn given_sources(_world: &mut ProcessManagerWorld, a: String, b: String) {
    let config = <Fulfillment as angzarr_client::router::HandlerKind>::handler_config();
    let angzarr_client::router::HandlerConfig::ProcessManager { sources, .. } = config else {
        panic!("not a PM config");
    };
    assert_eq!(sources, vec![a, b]);
}

#[given(expr = "the PM targets {string}")]
fn given_targets(world: &mut ProcessManagerWorld, target: String) {
    assert_eq!(world.router().output_domains(), vec![target]);
}

#[given("the PM tracks the number of orders seen")]
fn given_tracks(world: &mut ProcessManagerWorld) {
    world.process_state.clear();
}

#[given("OrderCompleted advances the orders-seen count")]
fn given_applier(_world: &mut ProcessManagerWorld) {
    // `#[applies(OrderCompleted)]` on Fulfillment increments orders_seen.
}

#[given("the PM handles OrderCreated by emitting a ReserveStock command")]
fn given_handles(_world: &mut ProcessManagerWorld) {
    // `#[handles(OrderCreated)]` on Fulfillment emits ReserveStock to shipping.
}

#[given("Fulfillment is the active process manager")]
fn given_active(world: &mut ProcessManagerWorld) {
    assert_eq!(world.router().handler_count(), 1);
}

#[given("process state events: OrderCompleted, OrderCompleted")]
fn given_process_state(world: &mut ProcessManagerWorld) {
    world.process_state = vec![OrderCompleted::default(), OrderCompleted::default()];
}

#[given(expr = "destination sequences {word}={int}")]
fn given_head(world: &mut ProcessManagerWorld, domain: String, head: u32) {
    world.destination_sequences = HashMap::from([(domain, head)]);
}

#[given(expr = "the OrderCreated trigger is at sequence {int} of order root {string}")]
fn given_trigger_position(world: &mut ProcessManagerWorld, seq: u32, root: String) {
    world.trigger_seq = seq;
    world.trigger_root = root;
}

// --- When ------------------------------------------------------------------

#[when(regex = r"^(?:an|the) OrderCreated trigger is dispatched to the PM router$")]
fn when_order_created(world: &mut ProcessManagerWorld) {
    world.dispatch(
        &OrderCreated {
            order_id: "o-1".into(),
            ..Default::default()
        },
        "order",
    );
}

#[when("a StockReserved trigger with a domain outside sources is dispatched")]
fn when_outside_sources(world: &mut ProcessManagerWorld) {
    world.dispatch(&StockReserved::default(), "billing");
}

// --- Then ------------------------------------------------------------------

#[then("the response contains exactly one command")]
fn then_one(world: &mut ProcessManagerWorld) {
    let r = world.response();
    assert_eq!(r.commands.len(), 1);
    assert_eq!(
        r.commands[0].cover.as_ref().map(|c| c.domain.as_str()),
        Some("shipping")
    );
}

#[then("the response contains no commands")]
fn then_none(world: &mut ProcessManagerWorld) {
    assert!(world.response().commands.is_empty());
    assert_eq!(*world.probe.observed_orders_seen.lock().unwrap(), None);
}

#[then(expr = "the PM has seen {int} completed orders")]
fn then_seen(world: &mut ProcessManagerWorld, n: u32) {
    assert_eq!(*world.probe.observed_orders_seen.lock().unwrap(), Some(n));
}

#[then("the ReserveStock command carries an angzarr_deferred header")]
fn then_deferred(world: &mut ProcessManagerWorld) {
    deferred_header(command_of::<ReserveStock>(&world.response().commands));
}

#[then(expr = "the deferred source cover is domain {string} root {string}")]
fn then_source(world: &mut ProcessManagerWorld, domain: String, root: String) {
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
fn then_source_seq(world: &mut ProcessManagerWorld, seq: u32) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert_eq!(deferred_header(cmd).source_seq, seq);
}

#[then(expr = "the deferred command_index is {int}")]
fn then_index(world: &mut ProcessManagerWorld, index: u32) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert_eq!(deferred_header(cmd).command_index, index);
}

#[then(expr = "the deferred basis_seq is {int}")]
fn then_basis(world: &mut ProcessManagerWorld, basis: u32) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert_eq!(deferred_header(cmd).basis_seq, basis);
}

#[then("no page of the ReserveStock command has an explicit sequence")]
fn then_no_explicit(world: &mut ProcessManagerWorld) {
    let cmd = command_of::<ReserveStock>(&world.response().commands);
    assert!(!has_explicit_sequence(cmd));
}
