//! Step definitions for `features/client/router.feature`.
//!
//! Every scenario drives macro-declared components through
//! `Router::new().with_handler(..).build()` and the runtime router's
//! `dispatch` / `dispatch_replay`. Handlers record what they observed into
//! per-scenario logs captured by their factory closures, so assertions read
//! what the library actually delivered.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use angzarr_client::proto::{
    business_response, command_page, event_page, page_header, BusinessResponse, CommandBook,
    CommandPage, ContextualCommand, Cover, EventBook, EventPage, Notification, PageHeader,
    ProcessManagerHandleRequest, ProcessManagerHandleResponse, Projection, RejectionNotification,
    ReplayRequest, SagaHandleRequest, SagaResponse,
};
use angzarr_client::router::runtime::{
    CommandHandlerRouter, ProcessManagerRouter, ProjectorRouter, SagaRouter,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{
    command_handler, error_codes, full_type_url, process_manager, projector, saga, ClientError,
    CommandRejectedError, CommandResult,
};
use cucumber::{given, then, when, World};
use prost::{Message, Name};
use prost_types::Any;

// ---------------------------------------------------------------------------
// Local protos (package `router`).
// ---------------------------------------------------------------------------

macro_rules! named {
    ($ty:ident, $name:literal) => {
        impl Name for $ty {
            const PACKAGE: &'static str = "router";
            const NAME: &'static str = $name;
        }
    };
}

#[derive(Clone, PartialEq, Message)]
pub struct CreateOrder {
    #[prost(string, tag = "1")]
    pub order_id: String,
    #[prost(string, tag = "2")]
    pub customer_id: String,
}
named!(CreateOrder, "CreateOrder");

#[derive(Clone, PartialEq, Message)]
pub struct AddItem {
    #[prost(string, tag = "1")]
    pub item: String,
}
named!(AddItem, "AddItem");

#[derive(Clone, PartialEq, Message)]
pub struct EmitTwo {}
named!(EmitTwo, "EmitTwo");

#[derive(Clone, PartialEq, Message)]
pub struct UnknownCommand {}
named!(UnknownCommand, "UnknownCommand");

#[derive(Clone, PartialEq, Message)]
pub struct OrderCreated {
    #[prost(string, tag = "1")]
    pub order_id: String,
    #[prost(string, tag = "2")]
    pub customer_id: String,
}
named!(OrderCreated, "OrderCreated");

#[derive(Clone, PartialEq, Message)]
pub struct ItemAdded {
    #[prost(string, tag = "1")]
    pub item: String,
}
named!(ItemAdded, "ItemAdded");

#[derive(Clone, PartialEq, Message)]
pub struct OrderShipped {}
named!(OrderShipped, "OrderShipped");

#[derive(Clone, PartialEq, Message)]
pub struct InventoryReserved {}
named!(InventoryReserved, "InventoryReserved");

#[derive(Clone, PartialEq, Message)]
pub struct ReserveStock {
    #[prost(string, tag = "1")]
    pub order_id: String,
}
named!(ReserveStock, "ReserveStock");

#[derive(Clone, PartialEq, Message)]
pub struct TypeA {}
named!(TypeA, "TypeA");

#[derive(Clone, PartialEq, Message)]
pub struct TypeB {}
named!(TypeB, "TypeB");

#[derive(Clone, PartialEq, Message)]
pub struct TypeC {}
named!(TypeC, "TypeC");

/// Aggregate state; a prost message so the Replay path can return it.
#[derive(Clone, PartialEq, Message)]
pub struct OrderState {
    #[prost(bool, tag = "1")]
    pub created: bool,
    #[prost(string, repeated, tag = "2")]
    pub items: Vec<String>,
}
named!(OrderState, "OrderState");

#[derive(Clone, PartialEq, Message)]
pub struct PmState {
    #[prost(uint32, tag = "1")]
    pub seen: u32,
}
named!(PmState, "PmState");

// ---------------------------------------------------------------------------
// Shared observation log.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Observed {
    /// Handler method names, in invocation order.
    calls: Vec<String>,
    /// State the command handler saw (copy).
    state: Option<OrderState>,
    /// Typed messages received by handlers.
    created: Vec<OrderCreated>,
    commands: Vec<CreateOrder>,
    /// `(instance id, per-instance invocation count)` for saga statelessness.
    saga_instances: Vec<(usize, u32)>,
    /// Notifications delivered to a saga `#[rejected]` handler.
    rejections: Vec<Notification>,
}

type Log = Arc<Mutex<Observed>>;

fn record(log: &Log, call: &str) {
    log.lock().unwrap().calls.push(call.to_string());
}

fn pack<M: Message + Name>(msg: &M) -> Any {
    Any {
        type_url: full_type_url::<M>(),
        value: msg.encode_to_vec(),
    }
}

fn event_page<M: Message + Name>(msg: &M, seq: u32) -> EventPage {
    EventPage {
        header: Some(PageHeader {
            sequence_type: Some(page_header::SequenceType::Sequence(seq)),
            sync_mode: None,
        }),
        payload: Some(event_page::Payload::Event(pack(msg))),
        ..Default::default()
    }
}

/// Event page without a header — what a handler returns when it leaves
/// sequencing to the framework.
fn unsequenced_page<M: Message + Name>(msg: &M) -> EventPage {
    EventPage {
        payload: Some(event_page::Payload::Event(pack(msg))),
        ..Default::default()
    }
}

fn cover(domain: &str) -> Cover {
    Cover {
        domain: domain.to_string(),
        correlation_id: "corr-router".to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Components.
// ---------------------------------------------------------------------------

struct OrderAggregate {
    log: Log,
}

#[command_handler(domain = "order", state = OrderState, supports_replay = true)]
impl OrderAggregate {
    #[applies(OrderCreated)]
    fn on_created(state: &mut OrderState, _evt: OrderCreated) {
        state.created = true;
    }

    #[applies(ItemAdded)]
    fn on_item(state: &mut OrderState, evt: ItemAdded) {
        state.items.push(evt.item);
    }

    #[handles(CreateOrder)]
    fn create(&self, cmd: CreateOrder, state: &OrderState, _seq: u32) -> CommandResult<EventBook> {
        record(&self.log, "CreateOrder");
        {
            let mut obs = self.log.lock().unwrap();
            obs.state = Some(state.clone());
            obs.commands.push(cmd.clone());
        }
        if cmd.order_id.is_empty() {
            return Err(CommandRejectedError::invalid_argument(
                "ORDER_ID_REQUIRED",
                "order_id is required",
                [("field", "order_id")],
            ));
        }
        Ok(EventBook {
            pages: vec![unsequenced_page(&OrderCreated {
                order_id: cmd.order_id,
                customer_id: cmd.customer_id,
            })],
            ..Default::default()
        })
    }

    #[handles(AddItem)]
    fn add_item(&self, cmd: AddItem, state: &OrderState, _seq: u32) -> CommandResult<EventBook> {
        record(&self.log, "AddItem");
        self.log.lock().unwrap().state = Some(state.clone());
        if !state.created {
            return Err(CommandRejectedError::precondition_failed(
                "ORDER_NOT_FOUND",
                "order does not exist",
                [("domain", "order")],
            ));
        }
        if cmd.item.is_empty() {
            return Err(CommandRejectedError::invalid_argument(
                "ITEM_REQUIRED",
                "item is required",
                [("field", "item")],
            ));
        }
        Ok(EventBook {
            pages: vec![unsequenced_page(&ItemAdded { item: cmd.item })],
            ..Default::default()
        })
    }

    #[handles(EmitTwo)]
    fn emit_two(&self, _cmd: EmitTwo, _state: &OrderState, _seq: u32) -> CommandResult<EventBook> {
        record(&self.log, "EmitTwo");
        Ok(EventBook {
            pages: vec![
                unsequenced_page(&ItemAdded { item: "a".into() }),
                unsequenced_page(&ItemAdded { item: "b".into() }),
            ],
            ..Default::default()
        })
    }
}

/// Command handler with only CreateOrder (C-0249).
struct CreateOnly {
    log: Log,
}

#[command_handler(domain = "order", state = OrderState)]
impl CreateOnly {
    #[handles(CreateOrder)]
    fn create(
        &self,
        _cmd: CreateOrder,
        _state: &OrderState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        record(&self.log, "CreateOrder");
        Ok(EventBook::default())
    }
}

/// Command handler with three handled types (C-0257).
struct Typed {
    log: Log,
}

#[command_handler(domain = "typed", state = OrderState)]
impl Typed {
    #[handles(TypeA)]
    fn a(&self, _cmd: TypeA, _state: &OrderState, _seq: u32) -> CommandResult<EventBook> {
        record(&self.log, "TypeA");
        Ok(EventBook::default())
    }

    #[handles(TypeB)]
    fn b(&self, _cmd: TypeB, _state: &OrderState, _seq: u32) -> CommandResult<EventBook> {
        record(&self.log, "TypeB");
        Ok(EventBook::default())
    }

    #[handles(TypeC)]
    fn c(&self, _cmd: TypeC, _state: &OrderState, _seq: u32) -> CommandResult<EventBook> {
        record(&self.log, "TypeC");
        Ok(EventBook::default())
    }
}

/// Fails every command (C-0261).
struct Failing {
    log: Log,
}

#[command_handler(domain = "order", state = OrderState)]
impl Failing {
    #[handles(CreateOrder)]
    fn create(
        &self,
        _cmd: CreateOrder,
        _state: &OrderState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        record(&self.log, "CreateOrder");
        Err(CommandRejectedError::precondition_failed(
            "ORDER_LOCKED",
            "order is locked",
            std::iter::empty::<(String, String)>(),
        ))
    }
}

static NEXT_INSTANCE: AtomicUsize = AtomicUsize::new(0);

struct FulfillmentSaga {
    log: Log,
    id: usize,
    invocations: AtomicU32,
}

impl FulfillmentSaga {
    fn new(log: Log) -> Self {
        Self {
            log,
            id: NEXT_INSTANCE.fetch_add(1, Ordering::SeqCst),
            invocations: AtomicU32::new(0),
        }
    }
}

#[saga(
    name = "saga-order-fulfillment",
    source = "order",
    target = "inventory"
)]
impl FulfillmentSaga {
    #[handles(OrderCreated)]
    fn on_created(&self, evt: OrderCreated) -> CommandResult<SagaResponse> {
        record(&self.log, "OrderCreated");
        let n = self.invocations.fetch_add(1, Ordering::SeqCst) + 1;
        self.log.lock().unwrap().saga_instances.push((self.id, n));
        Ok(SagaResponse {
            commands: vec![CommandBook {
                cover: Some(cover("inventory")),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(pack(&ReserveStock {
                        order_id: evt.order_id,
                    }))),
                    ..Default::default()
                }],
            }],
            events: vec![],
        })
    }

    #[handles(OrderShipped)]
    fn on_shipped(&self, _evt: OrderShipped) -> CommandResult<SagaResponse> {
        record(&self.log, "OrderShipped");
        Ok(SagaResponse::default())
    }

    #[rejected(domain = "inventory", command = "ReserveStock")]
    #[allow(dead_code)]
    fn on_reserve_rejected(&self, notification: &Notification) -> CommandResult<SagaResponse> {
        record(&self.log, "ReserveStockRejected");
        self.log
            .lock()
            .unwrap()
            .rejections
            .push(notification.clone());
        Ok(SagaResponse::default())
    }
}

struct OutputProjector {
    log: Log,
}

#[projector(name = "output", domains = ["order"])]
impl OutputProjector {
    #[handles(OrderCreated)]
    fn on_created(&self, evt: OrderCreated) -> CommandResult<()> {
        record(&self.log, "OrderCreated");
        self.log.lock().unwrap().created.push(evt);
        Ok(())
    }
}

struct FulfillmentPm {
    log: Log,
}

#[process_manager(
    name = "pm-fulfillment",
    pm_domain = "fulfillment",
    state = PmState,
    sources = ["orders", "inventory"],
    targets = ["shipping"]
)]
impl FulfillmentPm {
    #[handles(OrderCreated)]
    fn on_created(
        &self,
        _evt: OrderCreated,
        _state: &PmState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        record(&self.log, "OrderCreated");
        Ok(ProcessManagerHandleResponse::default())
    }

    #[handles(InventoryReserved)]
    fn on_reserved(
        &self,
        _evt: InventoryReserved,
        _state: &PmState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        record(&self.log, "InventoryReserved");
        Ok(ProcessManagerHandleResponse::default())
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct RouterWorld {
    log: Log,
    ch: Option<CommandHandlerRouter>,
    saga: Option<SagaRouter>,
    projector: Option<ProjectorRouter>,
    pm: Option<ProcessManagerRouter>,
    prior: EventBook,
    destination_sequences: std::collections::HashMap<String, u32>,
    response: Option<BusinessResponse>,
    saga_responses: Vec<SagaResponse>,
    projection: Option<Projection>,
    error: Option<ClientError>,
    replayed: Option<OrderState>,
    sent: Option<CreateOrder>,
    sent_event: Option<OrderCreated>,
    rejected_command: Option<CommandBook>,
}

impl RouterWorld {
    fn new() -> Self {
        Self {
            log: Arc::new(Mutex::new(Observed::default())),
            ch: None,
            saga: None,
            projector: None,
            pm: None,
            prior: EventBook::default(),
            destination_sequences: Default::default(),
            response: None,
            saga_responses: vec![],
            projection: None,
            error: None,
            replayed: None,
            sent: None,
            sent_event: None,
            rejected_command: None,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().calls.clone()
    }

    fn ch(&self) -> &CommandHandlerRouter {
        self.ch.as_ref().expect("command-handler router built")
    }

    fn dispatch_command<C: Message + Name>(&mut self, cmd: &C, domain: &str) {
        let request = ContextualCommand {
            events: Some(self.prior.clone()),
            command: Some(CommandBook {
                cover: Some(cover(domain)),
                pages: vec![CommandPage {
                    header: Some(PageHeader {
                        sequence_type: Some(page_header::SequenceType::Sequence(
                            self.prior.next_sequence,
                        )),
                        sync_mode: None,
                    }),
                    payload: Some(command_page::Payload::Command(pack(cmd))),
                    ..Default::default()
                }],
            }),
        };
        match self.ch().dispatch(request) {
            Ok(r) => {
                self.response = Some(r);
                self.error = None;
            }
            Err(e) => {
                self.response = None;
                self.error = Some(e);
            }
        }
    }

    fn dispatch_saga_event(&mut self, page: EventPage) {
        let request = SagaHandleRequest {
            source: Some(EventBook {
                cover: Some(cover("order")),
                pages: vec![page],
                next_sequence: 1,
                ..Default::default()
            }),
            destination_sequences: self.destination_sequences.clone(),
            ..Default::default()
        };
        match self
            .saga
            .as_ref()
            .expect("saga router built")
            .dispatch(request)
        {
            Ok(r) => self.saga_responses.push(r),
            Err(e) => self.error = Some(e),
        }
    }

    fn emitted_pages(&self) -> Vec<EventPage> {
        match self.response.as_ref().and_then(|r| r.result.as_ref()) {
            Some(business_response::Result::Events(book)) => book.pages.clone(),
            other => panic!("expected an Events response, got {other:?}"),
        }
    }
}

fn build_ch<H, F>(factory: F) -> CommandHandlerRouter
where
    H: angzarr_client::Handler + angzarr_client::HandlerKind + 'static,
    F: Fn() -> H + Send + Sync + 'static,
{
    match Router::new("router-feature").with_handler(factory).build() {
        Ok(Built::CommandHandler(r)) => r,
        other => panic!("expected a command-handler router, got {other:?}"),
    }
}

fn order_router(log: &Log) -> CommandHandlerRouter {
    let log = log.clone();
    build_ch(move || OrderAggregate { log: log.clone() })
}

fn saga_router(log: &Log) -> SagaRouter {
    let log = log.clone();
    match Router::new("router-feature")
        .with_handler(move || FulfillmentSaga::new(log.clone()))
        .build()
    {
        Ok(Built::Saga(r)) => r,
        other => panic!("expected a saga router, got {other:?}"),
    }
}

fn projector_router(log: &Log) -> ProjectorRouter {
    let log = log.clone();
    match Router::new("router-feature")
        .with_handler(move || OutputProjector { log: log.clone() })
        .build()
    {
        Ok(Built::Projector(r)) => r,
        other => panic!("expected a projector router, got {other:?}"),
    }
}

fn history(events: &[&str]) -> EventBook {
    let pages: Vec<EventPage> = events
        .iter()
        .enumerate()
        .map(|(i, name)| match *name {
            "OrderCreated" => event_page(
                &OrderCreated {
                    order_id: "o-1".into(),
                    customer_id: "c-1".into(),
                },
                i as u32,
            ),
            "ItemAdded" => event_page(
                &ItemAdded {
                    item: format!("item-{i}"),
                },
                i as u32,
            ),
            other => panic!("unknown history event {other}"),
        })
        .collect();
    EventBook {
        cover: Some(cover("order")),
        next_sequence: pages.len() as u32,
        pages,
        ..Default::default()
    }
}

fn page_sequence(page: &EventPage) -> Option<u32> {
    match page.header.as_ref().and_then(|h| h.sequence_type.as_ref()) {
        Some(page_header::SequenceType::Sequence(s)) => Some(*s),
        _ => None,
    }
}

fn decode_event<M: Message + Name + Default>(page: &EventPage) -> M {
    match &page.payload {
        Some(event_page::Payload::Event(any)) => {
            assert_eq!(any.type_url, full_type_url::<M>());
            M::decode(any.value.as_slice()).expect("event decodes")
        }
        other => panic!("expected an event payload, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Aggregate router.
// ---------------------------------------------------------------------------

#[given(expr = "an aggregate router with handlers for {string} and {string}")]
fn given_ch_two(world: &mut RouterWorld, a: String, b: String) {
    assert_eq!((a.as_str(), b.as_str()), ("CreateOrder", "AddItem"));
    world.ch = Some(order_router(&world.log));
    // AddItem requires an existing order; seed the history so both handlers
    // are reachable.
    world.prior = history(&["OrderCreated"]);
}

#[given(expr = "an aggregate router with handlers for {string}")]
fn given_ch_one(world: &mut RouterWorld, a: String) {
    assert_eq!(a, "CreateOrder");
    let log = world.log.clone();
    world.ch = Some(build_ch(move || CreateOnly { log: log.clone() }));
}

#[given("an aggregate router")]
fn given_aggregate_router(world: &mut RouterWorld) {
    world.ch = Some(order_router(&world.log));
}

#[given("an aggregate with existing events")]
fn given_existing_events(world: &mut RouterWorld) {
    world.prior = history(&["OrderCreated", "ItemAdded", "ItemAdded"]);
}

#[when(expr = "I receive a {string} command")]
fn when_receive_command(world: &mut RouterWorld, name: String) {
    match name.as_str() {
        "CreateOrder" => world.dispatch_command(
            &CreateOrder {
                order_id: "o-1".into(),
                customer_id: "c-1".into(),
            },
            "order",
        ),
        "AddItem" => world.dispatch_command(&AddItem { item: "x".into() }, "order"),
        other => panic!("unsupported command {other}"),
    }
}

#[when(expr = "I receive an {string} command")]
fn when_receive_an_command(world: &mut RouterWorld, name: String) {
    assert_eq!(name, "UnknownCommand");
    world.dispatch_command(&UnknownCommand {}, "order");
}

#[then(expr = "the {word} handler should be invoked")]
fn then_handler_invoked(world: &mut RouterWorld, name: String) {
    assert!(
        world.calls().contains(&name),
        "{name} handler not invoked; calls = {:?}",
        world.calls()
    );
}

#[then(expr = "the {word} handler should NOT be invoked")]
fn then_handler_not_invoked(world: &mut RouterWorld, name: String) {
    assert!(
        !world.calls().contains(&name),
        "{name} handler invoked; calls = {:?}",
        world.calls()
    );
}

#[when("I receive a command for that aggregate")]
fn when_command_for_aggregate(world: &mut RouterWorld) {
    world.dispatch_command(&AddItem { item: "new".into() }, "order");
}

#[then("the handler should receive state reflecting all previously recorded events")]
fn then_state_reflects_history(world: &mut RouterWorld) {
    assert!(world.error.is_none(), "dispatch failed: {:?}", world.error);
    let state = world
        .log
        .lock()
        .unwrap()
        .state
        .clone()
        .expect("state observed");
    assert!(state.created, "OrderCreated not applied");
    assert_eq!(
        state.items,
        vec!["item-1".to_string(), "item-2".to_string()]
    );
}

#[when("a handler emits 2 events")]
fn when_handler_emits_two(world: &mut RouterWorld) {
    world.prior = history(&["OrderCreated", "ItemAdded", "ItemAdded"]);
    world.dispatch_command(&EmitTwo {}, "order");
}

#[then("the router should return those events")]
fn then_router_returns_events(world: &mut RouterWorld) {
    let pages = world.emitted_pages();
    let items: Vec<String> = pages
        .iter()
        .map(|p| decode_event::<ItemAdded>(p).item)
        .collect();
    assert_eq!(items, vec!["a".to_string(), "b".to_string()]);
}

#[then("the events should carry consecutive sequences continuing the aggregate's history")]
fn then_consecutive_sequences(world: &mut RouterWorld) {
    let next = world.prior.next_sequence;
    let seqs: Vec<Option<u32>> = world.emitted_pages().iter().map(page_sequence).collect();
    assert_eq!(seqs, vec![Some(next), Some(next + 1)]);
}

#[then("the router should return an error")]
fn then_router_error(world: &mut RouterWorld) {
    assert!(
        world.response.is_none(),
        "unexpected response {:?}",
        world.response
    );
    assert!(world.error.is_some(), "expected an error");
}

#[then("the error should indicate unknown command type")]
fn then_unknown_command(world: &mut RouterWorld) {
    let err = world.error.as_ref().expect("error");
    assert_eq!(err.code(), error_codes::codes::NO_HANDLER_REGISTERED);
    assert!(err.is_invalid_argument());
    match err {
        ClientError::InvalidArgument(d) => assert_eq!(
            d.details.get(error_codes::keys::TYPE_URL),
            Some(&full_type_url::<UnknownCommand>())
        ),
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Saga router.
// ---------------------------------------------------------------------------

#[given(expr = "a saga router with handlers for {string} and {string}")]
fn given_saga_two(world: &mut RouterWorld, a: String, b: String) {
    assert_eq!((a.as_str(), b.as_str()), ("OrderCreated", "OrderShipped"));
    world.saga = Some(saga_router(&world.log));
}

#[given("a saga router")]
fn given_saga_router(world: &mut RouterWorld) {
    world.saga = Some(saga_router(&world.log));
}

#[when(expr = "I receive an {string} event")]
fn when_receive_event(world: &mut RouterWorld, name: String) {
    assert_eq!(name, "OrderCreated");
    let evt = OrderCreated {
        order_id: "o-9".into(),
        customer_id: "c-9".into(),
    };
    world.sent_event = Some(evt.clone());
    if world.saga.is_some() {
        world.dispatch_saga_event(event_page(&evt, 0));
    } else {
        let book = EventBook {
            cover: Some(cover("order")),
            pages: vec![event_page(&evt, 0)],
            next_sequence: 1,
            ..Default::default()
        };
        match world
            .projector
            .as_ref()
            .expect("projector router")
            .dispatch(book)
        {
            Ok(p) => world.projection = Some(p),
            Err(e) => world.error = Some(e),
        }
    }
}

#[when(expr = "I receive an event that triggers command to {string}")]
fn when_event_triggers_command(world: &mut RouterWorld, domain: String) {
    assert_eq!(domain, "inventory");
    world.destination_sequences.insert("inventory".into(), 7);
    world.dispatch_saga_event(event_page(
        &OrderCreated {
            order_id: "o-1".into(),
            customer_id: "c-1".into(),
        },
        3,
    ));
}

#[then(expr = "the emitted command should be sequenced to follow the current history of {string}")]
fn then_command_sequenced(world: &mut RouterWorld, domain: String) {
    assert!(world.error.is_none(), "dispatch failed: {:?}", world.error);
    let resp = world.saga_responses.last().expect("saga response");
    let cmd = resp
        .commands
        .iter()
        .find(|c| c.cover.as_ref().map(|c| c.domain.as_str()) == Some(domain.as_str()))
        .expect("command for domain");
    let head = world.destination_sequences[&domain];
    for page in &cmd.pages {
        match page.header.as_ref().and_then(|h| h.sequence_type.as_ref()) {
            Some(page_header::SequenceType::AngzarrDeferred(d)) => {
                assert_eq!(d.basis_seq, head, "basis_seq should be the observed head");
            }
            other => panic!("expected an angzarr_deferred header, got {other:?}"),
        }
    }
}

#[given("a saga router with a rejected command")]
fn given_saga_rejected(world: &mut RouterWorld) {
    world.saga = Some(saga_router(&world.log));
    world.rejected_command = Some(CommandBook {
        cover: Some(cover("inventory")),
        pages: vec![CommandPage {
            payload: Some(command_page::Payload::Command(pack(&ReserveStock {
                order_id: "o-1".into(),
            }))),
            ..Default::default()
        }],
    });
}

#[when("the router processes the rejection")]
fn when_router_processes_rejection(world: &mut RouterWorld) {
    let rejection = RejectionNotification {
        rejected_command: world.rejected_command.clone(),
        rejection_reason: "out_of_stock".into(),
    };
    let notification = Notification {
        cover: Some(cover("order")),
        payload: Some(pack(&rejection)),
        sent_at: Some(angzarr_client::now()),
    };
    world.dispatch_saga_event(EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: full_type_url::<Notification>(),
            value: notification.encode_to_vec(),
        })),
        ..Default::default()
    });
}

#[then("a rejection notification should be emitted")]
fn then_rejection_notification_emitted(world: &mut RouterWorld) {
    let obs = world.log.lock().unwrap();
    let notification = obs.rejections.first().unwrap_or_else(|| {
        panic!(
            "saga #[rejected] handler not invoked; calls = {:?}",
            obs.calls
        )
    });
    let payload = notification.payload.as_ref().expect("payload");
    assert_eq!(payload.type_url, full_type_url::<RejectionNotification>());
}

#[then("compensation should be initiated for the rejected command")]
fn then_compensation_initiated(world: &mut RouterWorld) {
    let obs = world.log.lock().unwrap();
    assert!(
        obs.calls.contains(&"ReserveStockRejected".to_string()),
        "compensation handler not invoked; calls = {:?}",
        obs.calls
    );
    let notification = obs.rejections.first().expect("notification");
    let rejection = RejectionNotification::decode(
        notification
            .payload
            .as_ref()
            .expect("payload")
            .value
            .as_slice(),
    )
    .expect("rejection decodes");
    assert_eq!(rejection.rejected_command, world.rejected_command);
}

#[when("I process two events with same type")]
fn when_two_events(world: &mut RouterWorld) {
    for id in ["o-1", "o-2"] {
        world.dispatch_saga_event(event_page(
            &OrderCreated {
                order_id: id.into(),
                customer_id: "c".into(),
            },
            0,
        ));
    }
}

#[then("each should be processed independently")]
fn then_independent(world: &mut RouterWorld) {
    assert!(world.error.is_none(), "dispatch failed: {:?}", world.error);
    assert_eq!(world.saga_responses.len(), 2);
    let ids: Vec<String> = world
        .saga_responses
        .iter()
        .map(|r| {
            assert_eq!(r.commands.len(), 1);
            match &r.commands[0].pages[0].payload {
                Some(command_page::Payload::Command(a)) => {
                    ReserveStock::decode(a.value.as_slice()).unwrap().order_id
                }
                other => panic!("unexpected payload {other:?}"),
            }
        })
        .collect();
    assert_eq!(ids, vec!["o-1".to_string(), "o-2".to_string()]);
}

#[then("no state should carry over between events")]
fn then_no_state_carry(world: &mut RouterWorld) {
    let instances = world.log.lock().unwrap().saga_instances.clone();
    assert_eq!(instances.len(), 2);
    assert_ne!(instances[0].0, instances[1].0, "same saga instance reused");
    assert!(
        instances.iter().all(|(_, n)| *n == 1),
        "an instance saw a prior event: {instances:?}"
    );
}

// ---------------------------------------------------------------------------
// Projector router.
// ---------------------------------------------------------------------------

#[given(expr = "a projector router with handlers for {string}")]
fn given_projector_with(world: &mut RouterWorld, name: String) {
    assert_eq!(name, "OrderCreated");
    world.projector = Some(projector_router(&world.log));
}

#[given("a projector router")]
fn given_projector(world: &mut RouterWorld) {
    world.projector = Some(projector_router(&world.log));
}

#[when(expr = "I receive {int} events in a batch")]
fn when_batch(world: &mut RouterWorld, n: u32) {
    let pages = (0..n)
        .map(|i| {
            event_page(
                &OrderCreated {
                    order_id: format!("o-{i}"),
                    customer_id: "c".into(),
                },
                i,
            )
        })
        .collect();
    let book = EventBook {
        cover: Some(cover("order")),
        pages,
        next_sequence: n,
        ..Default::default()
    };
    match world
        .projector
        .as_ref()
        .expect("projector router")
        .dispatch(book)
    {
        Ok(p) => world.projection = Some(p),
        Err(e) => world.error = Some(e),
    }
}

#[then(expr = "all {int} events should be processed in order")]
fn then_processed_in_order(world: &mut RouterWorld, n: u32) {
    let ids: Vec<String> = world
        .log
        .lock()
        .unwrap()
        .created
        .iter()
        .map(|e| e.order_id.clone())
        .collect();
    let expected: Vec<String> = (0..n).map(|i| format!("o-{i}")).collect();
    assert_eq!(ids, expected);
}

#[then(expr = "the resulting projection should reflect all {int} events")]
fn then_projection_reflects(world: &mut RouterWorld, n: u32) {
    let projection = world.projection.as_ref().expect("projection");
    assert_eq!(projection.sequence, n);
    assert_eq!(
        projection.cover.as_ref().map(|c| c.domain.as_str()),
        Some("order")
    );
}

// ---------------------------------------------------------------------------
// Process manager router.
// ---------------------------------------------------------------------------

#[given(expr = "a PM router with handlers for {string} and {string}")]
fn given_pm(world: &mut RouterWorld, a: String, b: String) {
    assert_eq!(
        (a.as_str(), b.as_str()),
        ("OrderCreated", "InventoryReserved")
    );
    let log = world.log.clone();
    world.pm = match Router::new("router-feature")
        .with_handler(move || FulfillmentPm { log: log.clone() })
        .build()
    {
        Ok(Built::ProcessManager(r)) => Some(r),
        other => panic!("expected a PM router, got {other:?}"),
    };
}

#[when(expr = "I receive an {string} event from domain {string}")]
fn when_pm_event(world: &mut RouterWorld, name: String, domain: String) {
    let page = match name.as_str() {
        "OrderCreated" => event_page(&OrderCreated::default(), 0),
        "InventoryReserved" => event_page(&InventoryReserved {}, 0),
        other => panic!("unsupported event {other}"),
    };
    let request = ProcessManagerHandleRequest {
        trigger: Some(EventBook {
            cover: Some(cover(&domain)),
            pages: vec![page],
            next_sequence: 1,
            ..Default::default()
        }),
        ..Default::default()
    };
    if let Err(e) = world.pm.as_ref().expect("PM router").dispatch(request) {
        world.error = Some(e);
    }
}

// ---------------------------------------------------------------------------
// Handler registration / typed messages.
// ---------------------------------------------------------------------------

#[given("a router")]
fn given_router(world: &mut RouterWorld) {
    world.ch = Some(order_router(&world.log));
    world.projector = Some(projector_router(&world.log));
}

#[when(expr = "I register handlers for {string}, {string}, and {string}")]
fn when_register_three(world: &mut RouterWorld, a: String, b: String, c: String) {
    assert_eq!(
        (a.as_str(), b.as_str(), c.as_str()),
        ("TypeA", "TypeB", "TypeC")
    );
    let log = world.log.clone();
    world.ch = Some(build_ch(move || Typed { log: log.clone() }));
}

#[then("all three types should be routable")]
fn then_three_routable(world: &mut RouterWorld) {
    for (i, url) in [
        full_type_url::<TypeA>(),
        full_type_url::<TypeB>(),
        full_type_url::<TypeC>(),
    ]
    .into_iter()
    .enumerate()
    {
        let request = ContextualCommand {
            events: Some(EventBook::default()),
            command: Some(CommandBook {
                cover: Some(cover("typed")),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(Any {
                        type_url: url.clone(),
                        value: vec![],
                    })),
                    ..Default::default()
                }],
            }),
        };
        world
            .ch()
            .dispatch(request)
            .unwrap_or_else(|e| panic!("{url} not routable: {e:?}"));
        assert_eq!(world.calls().len(), i + 1);
    }
}

#[then("each should invoke its specific handler")]
fn then_specific_handler(world: &mut RouterWorld) {
    assert_eq!(
        world.calls(),
        vec![
            "TypeA".to_string(),
            "TypeB".to_string(),
            "TypeC".to_string()
        ]
    );
}

#[given("a router with handler for protobuf message type")]
fn given_typed_router(world: &mut RouterWorld) {
    world.projector = Some(projector_router(&world.log));
}

#[when("I receive an event with that type")]
fn when_typed_event(world: &mut RouterWorld) {
    let evt = OrderCreated {
        order_id: "o-42".into(),
        customer_id: "c-7".into(),
    };
    world.sent_event = Some(evt.clone());
    let book = EventBook {
        cover: Some(cover("order")),
        pages: vec![event_page(&evt, 0)],
        next_sequence: 1,
        ..Default::default()
    };
    match world.projector.as_ref().expect("projector").dispatch(book) {
        Ok(p) => world.projection = Some(p),
        Err(e) => world.error = Some(e),
    }
}

#[then("the handler should receive the message as its declared protobuf type")]
fn then_typed_message(world: &mut RouterWorld) {
    let created = world.log.lock().unwrap().created.clone();
    assert_eq!(created, vec![world.sent_event.clone().expect("sent event")]);
}

// ---------------------------------------------------------------------------
// State building (Replay).
// ---------------------------------------------------------------------------

#[given("events: OrderCreated, ItemAdded, ItemAdded")]
fn given_events(world: &mut RouterWorld) {
    world.prior = history(&["OrderCreated", "ItemAdded", "ItemAdded"]);
}

#[given("no events for the aggregate")]
fn given_no_events(world: &mut RouterWorld) {
    world.prior = EventBook::default();
}

#[when(regex = r"^I build state(?: from these events)?$")]
fn when_build_state(world: &mut RouterWorld) {
    let response = world
        .ch()
        .dispatch_replay(ReplayRequest {
            events: world.prior.pages.clone(),
            base_snapshot: None,
        })
        .expect("replay succeeds");
    let any = response.state.expect("replayed state");
    assert_eq!(any.type_url, full_type_url::<OrderState>());
    world.replayed = Some(OrderState::decode(any.value.as_slice()).expect("state decodes"));
}

#[then("the state should reflect all three events applied")]
fn then_state_three(world: &mut RouterWorld) {
    let state = world.replayed.as_ref().expect("state");
    assert!(state.created);
    assert_eq!(
        state.items,
        vec!["item-1".to_string(), "item-2".to_string()]
    );
}

#[then(expr = "the state should have {int} items")]
fn then_state_items(world: &mut RouterWorld, n: usize) {
    assert_eq!(world.replayed.as_ref().expect("state").items.len(), n);
}

#[then("the state should be the default/initial state")]
fn then_state_default(world: &mut RouterWorld) {
    assert_eq!(
        world.replayed.clone().expect("state"),
        OrderState::default()
    );
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

#[when("a handler returns an error")]
fn when_handler_error(world: &mut RouterWorld) {
    let log = world.log.clone();
    world.ch = Some(build_ch(move || Failing { log: log.clone() }));
    world.dispatch_command(&CreateOrder::default(), "order");
}

#[then("the caller should be informed of the failure")]
fn then_caller_informed(world: &mut RouterWorld) {
    assert_eq!(world.calls(), vec!["CreateOrder".to_string()]);
    let err = world.error.as_ref().expect("error");
    assert!(err.is_precondition_failed());
    assert_eq!(err.code(), "ORDER_LOCKED");
}

#[then(regex = r"^no events? should be emitted$")]
fn then_no_events(world: &mut RouterWorld) {
    assert!(world.error.is_some(), "expected the dispatch to fail");
    assert!(
        world.response.is_none(),
        "events returned: {:?}",
        world.response
    );
}

#[when("I receive an event with invalid payload")]
fn when_invalid_payload(world: &mut RouterWorld) {
    let book = EventBook {
        cover: Some(cover("order")),
        pages: vec![EventPage {
            header: Some(PageHeader {
                sequence_type: Some(page_header::SequenceType::Sequence(0)),
                sync_mode: None,
            }),
            payload: Some(event_page::Payload::Event(Any {
                type_url: full_type_url::<OrderCreated>(),
                value: vec![0xff, 0xff, 0xff],
            })),
            ..Default::default()
        }],
        next_sequence: 1,
        ..Default::default()
    };
    match world.projector.as_ref().expect("projector").dispatch(book) {
        Ok(p) => world.projection = Some(p),
        Err(e) => world.error = Some(e),
    }
}

#[then("the request should fail")]
fn then_request_fails(world: &mut RouterWorld) {
    assert!(world.projection.is_none(), "unexpected projection");
    assert!(world.error.is_some(), "expected an error");
    assert!(world.log.lock().unwrap().created.is_empty());
}

#[then("the failure should identify the malformed payload")]
fn then_malformed_identified(world: &mut RouterWorld) {
    match world.error.as_ref().expect("error") {
        ClientError::InvalidArgument(d) => {
            assert_eq!(d.code, error_codes::codes::ANY_DECODE_FAILED);
            assert_eq!(
                d.details.get(error_codes::keys::TYPE_URL),
                Some(&full_type_url::<OrderCreated>())
            );
        }
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Guard / validate / compute.
// ---------------------------------------------------------------------------

#[given("an aggregate with guard checking aggregate exists")]
fn given_guard(world: &mut RouterWorld) {
    world.ch = Some(order_router(&world.log));
}

#[when("I send command to non-existent aggregate")]
fn when_nonexistent(world: &mut RouterWorld) {
    world.prior = EventBook::default();
    world.dispatch_command(&AddItem { item: "x".into() }, "order");
}

#[then("guard should reject")]
fn then_guard_rejects(world: &mut RouterWorld) {
    let err = world.error.as_ref().expect("guard rejection");
    assert!(err.is_precondition_failed());
    assert_eq!(err.code(), "ORDER_NOT_FOUND");
}

#[given("an aggregate handler with validation")]
fn given_validation(world: &mut RouterWorld) {
    world.ch = Some(order_router(&world.log));
}

#[when("I send command with invalid data")]
fn when_invalid_data(world: &mut RouterWorld) {
    world.dispatch_command(&CreateOrder::default(), "order");
}

#[then("validate should reject")]
fn then_validate_rejects(world: &mut RouterWorld) {
    let err = world.error.as_ref().expect("validation rejection");
    assert!(err.is_invalid_argument());
    assert!(world.response.is_none());
}

#[then("rejection reason should describe the issue")]
fn then_reason_describes(world: &mut RouterWorld) {
    match world.error.as_ref().expect("error") {
        ClientError::Rejected(r) => {
            assert_eq!(r.code, "ORDER_ID_REQUIRED");
            assert_eq!(r.message, "order_id is required");
            assert_eq!(r.details.get("field").map(String::as_str), Some("order_id"));
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[given("an aggregate handler")]
fn given_aggregate_handler(world: &mut RouterWorld) {
    world.ch = Some(order_router(&world.log));
    world.prior = history(&["OrderCreated"]);
}

#[when("guard and validate pass")]
fn when_guard_validate_pass(world: &mut RouterWorld) {
    world.dispatch_command(
        &AddItem {
            item: "widget".into(),
        },
        "order",
    );
}

#[then("compute should produce events")]
fn then_compute_events(world: &mut RouterWorld) {
    assert!(world.error.is_none(), "dispatch failed: {:?}", world.error);
    assert_eq!(world.emitted_pages().len(), 1);
}

#[then("events should reflect the state change")]
fn then_events_reflect(world: &mut RouterWorld) {
    let evt: ItemAdded = decode_event(&world.emitted_pages()[0]);
    assert_eq!(evt.item, "widget");
}
