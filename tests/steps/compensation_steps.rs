//! Step definitions for `features/client/compensation.feature`.
//!
//! A rejected saga/PM command carries `PageHeader.angzarr_deferred`
//! (source cover, source_seq, source_component, command_index). The
//! coordinator delivers its rejection to the source aggregate as a
//! `Notification` wrapping a `RejectionNotification`, inside a CommandBook
//! addressed to that source (types.proto, RejectionNotification). The
//! scenarios build that delivery as fixture data and exercise the client
//! library on it: `CompensationContext::from_notification`, and the
//! command-handler / process-manager routers dispatching the delivery to
//! `#[rejected]` handlers, which record what they received. A saga never
//! receives a rejection: its source aggregate compensates (C-0483).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use angzarr_client::proto::{
    command_page, event_page, page_header, AngzarrDeferredSequence, BusinessResponse, CommandBook,
    CommandPage, ContextualCommand, Cover, EventBook, EventPage, Notification, PageHeader,
    ProcessManagerHandleRequest, ProcessManagerHandleResponse, RejectionNotification,
    SagaHandleRequest, SagaResponse, Uuid as ProtoUuid,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{
    command_handler, emit_compensation_events, full_type_url, process_manager, saga, ClientError,
    CommandRejectedError, CommandResult, CompensationContext,
};
use cucumber::{given, then, when, World};
use prost::{Message, Name};
use prost_types::{Any, Timestamp};

// ---------------------------------------------------------------------------
// Local protos (package `compensation`).
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Message)]
pub struct ReserveStock {
    #[prost(string, tag = "1")]
    pub order_id: String,
    #[prost(uint32, tag = "2")]
    pub quantity: u32,
}
impl Name for ReserveStock {
    const PACKAGE: &'static str = "compensation";
    const NAME: &'static str = "ReserveStock";
}

#[derive(Clone, PartialEq, Message)]
pub struct CreateShipment {
    #[prost(string, tag = "1")]
    pub order_id: String,
}
impl Name for CreateShipment {
    const PACKAGE: &'static str = "compensation";
    const NAME: &'static str = "CreateShipment";
}

#[derive(Clone, PartialEq, Message)]
pub struct OrderCreated {}
impl Name for OrderCreated {
    const PACKAGE: &'static str = "compensation";
    const NAME: &'static str = "OrderCreated";
}

#[derive(Default)]
pub struct NoState;

// ---------------------------------------------------------------------------
// Observation log shared with the `#[rejected]` handlers.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Received {
    /// Component that handled the rejection.
    handler: Option<&'static str>,
    notification: Option<Notification>,
    context: Option<Result<CompensationContext, String>>,
}

type Log = Arc<Mutex<Received>>;

fn receive(log: &Log, handler: &'static str, notification: &Notification) {
    let mut r = log.lock().unwrap();
    r.handler = Some(handler);
    r.notification = Some(notification.clone());
    r.context =
        Some(CompensationContext::from_notification(notification).map_err(|e| e.to_string()));
}

/// Source aggregate in domain "orders": compensates ReserveStock rejected by
/// inventory.
struct OrdersAggregate {
    log: Log,
}

#[command_handler(domain = "orders", state = NoState)]
impl OrdersAggregate {
    #[rejected(domain = "inventory", command = "ReserveStock")]
    fn on_reserve_rejected(
        &self,
        notification: &Notification,
        _state: &NoState,
    ) -> CommandResult<BusinessResponse> {
        receive(&self.log, "orders", notification);
        Ok(emit_compensation_events(EventBook::default()))
    }
}

/// Source aggregate in domain "fulfillment": compensates CreateShipment
/// rejected by shipping (the inner leg of a nested saga chain).
struct FulfillmentAggregate {
    log: Log,
}

#[command_handler(domain = "fulfillment", state = NoState)]
impl FulfillmentAggregate {
    #[rejected(domain = "shipping", command = "CreateShipment")]
    fn on_shipment_rejected(
        &self,
        notification: &Notification,
        _state: &NoState,
    ) -> CommandResult<BusinessResponse> {
        receive(&self.log, "fulfillment", notification);
        Ok(emit_compensation_events(EventBook::default()))
    }
}

/// Inventory aggregate that rejects every reservation.
struct InventoryAggregate;

#[command_handler(domain = "inventory", state = NoState)]
impl InventoryAggregate {
    #[handles(ReserveStock)]
    fn reserve(&self, _cmd: ReserveStock, _state: &NoState, _seq: u32) -> CommandResult<EventBook> {
        Err(CommandRejectedError::precondition_failed(
            "INSUFFICIENT_STOCK",
            "insufficient stock",
            std::iter::empty::<(String, String)>(),
        ))
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct OrderCancelled {
    #[prost(string, tag = "1")]
    pub reason: String,
}
impl Name for OrderCancelled {
    const PACKAGE: &'static str = "compensation";
    const NAME: &'static str = "OrderCancelled";
}

/// Saga translating `order` OrderCreated into an `inventory` ReserveStock;
/// counts its invocations.
struct OrderFulfillmentSaga {
    calls: Arc<AtomicU32>,
}

#[saga(name = "OrderFulfillment", source = "order", target = "inventory")]
impl OrderFulfillmentSaga {
    #[handles(OrderCreated)]
    fn on_created(&self, _evt: OrderCreated) -> CommandResult<SagaResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(SagaResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "inventory".into(),
                    ..Default::default()
                }),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(pack(&ReserveStock {
                        order_id: "o-1".into(),
                        quantity: 1,
                    }))),
                    ..Default::default()
                }],
            }],
            events: vec![],
        })
    }
}

/// Order aggregate: the saga's source, compensating its rejected
/// ReserveStock.
struct OrderAggregate;

#[command_handler(domain = "order", state = NoState)]
impl OrderAggregate {
    #[rejected(domain = "inventory", command = "ReserveStock")]
    fn on_reserve_rejected(
        &self,
        _notification: &Notification,
        _state: &NoState,
    ) -> CommandResult<BusinessResponse> {
        Ok(emit_compensation_events(EventBook {
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(pack(&OrderCancelled {
                    reason: "stock rejected".into(),
                }))),
                ..Default::default()
            }],
            ..Default::default()
        }))
    }
}

struct WorkflowPm {
    log: Log,
}

#[process_manager(
    name = "pmg-order-workflow",
    pm_domain = "order-workflow",
    state = NoState,
    sources = ["orders"],
    targets = ["inventory"]
)]
impl WorkflowPm {
    #[handles(OrderCreated)]
    fn on_created(
        &self,
        _evt: OrderCreated,
        _state: &NoState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }

    #[rejected(domain = "inventory", command = "ReserveStock")]
    #[allow(dead_code)]
    fn on_reserve_rejected(
        &self,
        notification: &Notification,
        _state: &NoState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        receive(&self.log, "pm", notification);
        Ok(ProcessManagerHandleResponse::default())
    }
}

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn root_for(label: &str) -> ProtoUuid {
    ProtoUuid {
        value: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, label.as_bytes())
            .as_bytes()
            .to_vec(),
    }
}

/// Parameters of the rejected saga command.
#[derive(Debug, Clone)]
struct Origin {
    saga_name: String,
    source_domain: String,
    source_root: String,
    source_seq: u32,
    correlation_id: String,
    target_domain: String,
}

impl Default for Origin {
    fn default() -> Self {
        Self {
            saga_name: "order-fulfillment".into(),
            source_domain: "orders".into(),
            source_root: "order-1".into(),
            source_seq: 5,
            correlation_id: "workflow-123".into(),
            target_domain: "inventory".into(),
        }
    }
}

impl Origin {
    fn source_cover(&self) -> Cover {
        Cover {
            domain: self.source_domain.clone(),
            root: Some(root_for(&self.source_root)),
            correlation_id: self.correlation_id.clone(),
            ..Default::default()
        }
    }

    fn deferred_header(&self) -> PageHeader {
        PageHeader {
            sequence_type: Some(page_header::SequenceType::AngzarrDeferred(
                AngzarrDeferredSequence {
                    source: Some(self.source_cover()),
                    source_seq: self.source_seq,
                    source_component: self.saga_name.clone(),
                    command_index: 0,
                    ..Default::default()
                },
            )),
            sync_mode: None,
        }
    }

    /// The saga-emitted command as the target aggregate rejected it.
    fn rejected_command(&self, payload: Any) -> CommandBook {
        CommandBook {
            cover: Some(Cover {
                domain: self.target_domain.clone(),
                root: Some(root_for("target-1")),
                correlation_id: self.correlation_id.clone(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                header: Some(self.deferred_header()),
                payload: Some(command_page::Payload::Command(payload)),
                ..Default::default()
            }],
        }
    }
}

fn pack<M: Message + Name>(msg: &M) -> Any {
    Any {
        type_url: full_type_url::<M>(),
        value: msg.encode_to_vec(),
    }
}

/// Coordinator delivery of a rejection: the Notification and the
/// CommandBook addressed to the source aggregate that carries it.
fn delivery(
    origin: &Origin,
    rejected: &CommandBook,
    reason: &str,
    sent_at: Timestamp,
) -> (Notification, CommandBook) {
    let notification = Notification {
        cover: Some(origin.source_cover()),
        payload: Some(pack(&RejectionNotification {
            rejected_command: Some(rejected.clone()),
            rejection_reason: reason.to_string(),
        })),
        sent_at: Some(sent_at),
    };
    let envelope = CommandBook {
        cover: Some(origin.source_cover()),
        pages: vec![CommandPage {
            header: Some(origin.deferred_header()),
            payload: Some(command_page::Payload::Command(pack(&notification))),
            ..Default::default()
        }],
    };
    (notification, envelope)
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct CompensationWorld {
    log: Log,
    origin: Origin,
    rejected: Option<CommandBook>,
    reason: String,
    context: Option<CompensationContext>,
    sent_at: Option<Timestamp>,
    envelope: Option<CommandBook>,
    dispatch_error: Option<ClientError>,
    saga_mode: Option<&'static str>,
    /// OrderFulfillment saga invocations (C-0483).
    saga_calls: Arc<AtomicU32>,
    /// Saga invocations when the rejection was dispatched (C-0483).
    saga_calls_at_rejection: Option<u32>,
    /// The order aggregate's compensation response (C-0483).
    source_response: Option<BusinessResponse>,
}

impl CompensationWorld {
    fn new() -> Self {
        Self {
            log: Arc::new(Mutex::new(Received::default())),
            origin: Origin::default(),
            rejected: None,
            reason: String::new(),
            context: None,
            sent_at: None,
            envelope: None,
            dispatch_error: None,
            saga_mode: None,
            saga_calls: Arc::new(AtomicU32::new(0)),
            saga_calls_at_rejection: None,
            source_response: None,
        }
    }

    fn ensure_rejected(&mut self) {
        if self.rejected.is_none() {
            self.rejected = Some(self.origin.rejected_command(pack(&ReserveStock {
                order_id: "o-1".into(),
                quantity: 1,
            })));
        }
        if self.reason.is_empty() {
            self.reason = "out_of_stock".into();
        }
    }

    fn rejected(&self) -> &CommandBook {
        self.rejected.as_ref().expect("rejected command")
    }

    fn notification(&mut self) -> Notification {
        self.ensure_rejected();
        let sent_at = angzarr_client::now();
        self.sent_at = Some(sent_at);
        let (notification, envelope) =
            delivery(&self.origin, self.rejected(), &self.reason, sent_at);
        self.envelope = Some(envelope);
        notification
    }

    /// Deliver the rejection to the source aggregate's command-handler router.
    fn deliver_to_source(&mut self) {
        self.notification();
        let log = self.log.clone();
        let built = match self.origin.source_domain.as_str() {
            "fulfillment" => Router::new("compensation")
                .with_handler(move || FulfillmentAggregate { log: log.clone() })
                .build(),
            _ => Router::new("compensation")
                .with_handler(move || OrdersAggregate { log: log.clone() })
                .build(),
        };
        let Ok(Built::CommandHandler(router)) = built else {
            panic!("expected a command-handler router");
        };
        let request = ContextualCommand {
            events: Some(EventBook::default()),
            command: self.envelope.clone(),
        };
        if let Err(e) = router.dispatch(request) {
            self.dispatch_error = Some(e);
        }
    }

    /// Context the `#[rejected]` handler derived from what it received.
    fn received_context(&self) -> CompensationContext {
        let r = self.log.lock().unwrap();
        assert!(
            self.dispatch_error.is_none(),
            "delivery failed: {:?}",
            self.dispatch_error
        );
        match r.context.clone() {
            Some(Ok(ctx)) => ctx,
            Some(Err(e)) => panic!("handler could not read the rejection: {e}"),
            None => panic!("no #[rejected] handler received the rejection"),
        }
    }

    fn received_notification(&self) -> Notification {
        self.log
            .lock()
            .unwrap()
            .notification
            .clone()
            .expect("no #[rejected] handler received the rejection")
    }
}

fn deferred_of(cmd: &CommandBook) -> AngzarrDeferredSequence {
    match cmd
        .pages
        .first()
        .and_then(|p| p.header.as_ref())
        .and_then(|h| h.sequence_type.as_ref())
    {
        Some(page_header::SequenceType::AngzarrDeferred(d)) => d.clone(),
        other => panic!("expected an angzarr_deferred header, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Givens.
// ---------------------------------------------------------------------------

#[given("a compensation handling context")]
fn given_handling_context(world: &mut CompensationWorld) {
    world.origin = Origin::default();
}

#[given("a saga command that was rejected")]
fn given_saga_command_rejected(world: &mut CompensationWorld) {
    world.reason = "out_of_stock".into();
    world.ensure_rejected();
}

#[given(expr = "a saga {string} triggered by {string} aggregate at sequence {int}")]
fn given_saga_triggered(world: &mut CompensationWorld, saga: String, domain: String, seq: u32) {
    world.origin.saga_name = saga;
    world.origin.source_domain = domain;
    world.origin.source_seq = seq;
}

#[given(regex = r"^the (?:saga )?command was rejected$")]
fn given_command_rejected(world: &mut CompensationWorld) {
    world.reason = "rejected_by_target".into();
    world.ensure_rejected();
}

#[given(expr = "a saga command with correlation ID {string}")]
fn given_correlation(world: &mut CompensationWorld, cid: String) {
    world.origin.correlation_id = cid;
}

#[given("a CompensationContext for rejected command")]
fn given_context_for_rejected(world: &mut CompensationWorld) {
    let notification = world.notification();
    world.context = Some(CompensationContext::from_notification(&notification).expect("context"));
}

#[given(expr = "a CompensationContext from {string} aggregate at sequence {int}")]
fn given_context_from_domain_seq(world: &mut CompensationWorld, domain: String, seq: u32) {
    world.origin.source_domain = domain;
    world.origin.source_seq = seq;
    given_context_for_rejected(world);
}

#[given(expr = "a CompensationContext from saga {string}")]
fn given_context_from_saga(world: &mut CompensationWorld, saga: String) {
    world.origin.saga_name = saga;
    given_context_for_rejected(world);
}

#[given(expr = "a CompensationContext from {string} aggregate root {string}")]
fn given_context_from_root(world: &mut CompensationWorld, domain: String, root: String) {
    world.origin.source_domain = domain;
    world.origin.source_root = root;
    given_context_for_rejected(world);
}

#[given(expr = "a command rejected with reason {string}")]
fn given_reason(world: &mut CompensationWorld, reason: String) {
    world.reason = reason;
    world.ensure_rejected();
}

const STRUCTURED_REASON: &str =
    r#"{"code":"INSUFFICIENT_STOCK","sku":"sku-9","requested":10,"available":3}"#;

#[given("a command rejected with structured reason")]
fn given_structured_reason(world: &mut CompensationWorld) {
    world.reason = STRUCTURED_REASON.into();
    world.ensure_rejected();
}

#[given("a saga command with specific payload")]
fn given_specific_payload(world: &mut CompensationWorld) {
    world.rejected = Some(world.origin.rejected_command(pack(&ReserveStock {
        order_id: "o-77".into(),
        quantity: 12,
    })));
}

#[given("a nested saga scenario")]
fn given_nested(world: &mut CompensationWorld) {
    // orders --(order-fulfillment)--> fulfillment --(fulfillment-shipping)--> shipping.
    // The inner command's source is the fulfillment aggregate; the workflow
    // correlation_id threads the whole chain back to the orders root cause.
    world.origin = Origin {
        saga_name: "fulfillment-shipping".into(),
        source_domain: "fulfillment".into(),
        source_root: "fulfillment-1".into(),
        source_seq: 10,
        correlation_id: "workflow-123".into(),
        target_domain: "shipping".into(),
    };
}

#[given("an inner saga command was rejected")]
fn given_inner_rejected(world: &mut CompensationWorld) {
    world.rejected = Some(world.origin.rejected_command(pack(&CreateShipment {
        order_id: "o-1".into(),
    })));
    world.reason = "carrier_unavailable".into();
}

#[given("a process manager router")]
fn given_pm_router(world: &mut CompensationWorld) {
    world.saga_mode = Some("pm");
    world.origin.saga_name = "pmg-order-workflow".into();
}

// ---------------------------------------------------------------------------
// Whens.
// ---------------------------------------------------------------------------

#[when("the compensation context is constructed from the rejection")]
fn when_context_constructed(world: &mut CompensationWorld) {
    let notification = world.notification();
    world.context = Some(
        CompensationContext::from_notification(&notification)
            .expect("context from a delivered rejection"),
    );
}

#[when(
    regex = r"^I build a (?:RejectionNotification|Notification from the context|Notification from a CompensationContext|notification CommandBook)$"
)]
fn when_deliver(world: &mut CompensationWorld) {
    world.deliver_to_source();
}

/// The target aggregate rejects the command; the rejection is delivered
/// back to the emitting saga / PM router.
fn reject_and_deliver_to_emitter(world: &mut CompensationWorld) {
    world.ensure_rejected();
    let Ok(Built::CommandHandler(inventory)) = Router::new("inventory")
        .with_handler(|| InventoryAggregate)
        .build()
    else {
        panic!("expected a command-handler router");
    };
    let err = inventory
        .dispatch(ContextualCommand {
            events: Some(EventBook::default()),
            command: world.rejected.clone(),
        })
        .expect_err("inventory rejects the reservation");
    assert!(err.is_precondition_failed(), "unexpected error {err:?}");
    world.reason = err.message();
    let notification = world.notification();
    let page = EventPage {
        header: Some(world.origin.deferred_header()),
        payload: Some(event_page::Payload::Event(pack(&notification))),
        ..Default::default()
    };
    let book = EventBook {
        cover: Some(world.origin.source_cover()),
        pages: vec![page],
        next_sequence: world.origin.source_seq + 1,
        ..Default::default()
    };
    let log = world.log.clone();
    let result = match world.saga_mode {
        Some("pm") => {
            let Ok(Built::ProcessManager(router)) = Router::new("pm")
                .with_handler(move || WorkflowPm { log: log.clone() })
                .build()
            else {
                panic!("expected a process-manager router");
            };
            router
                .dispatch(ProcessManagerHandleRequest {
                    trigger: Some(book),
                    ..Default::default()
                })
                .map(|_| ())
        }
        other => panic!("no emitter router configured: {other:?}"),
    };
    if let Err(e) = result {
        world.dispatch_error = Some(e);
    }
}

#[when("a PM command is rejected")]
fn when_pm_rejected(world: &mut CompensationWorld) {
    reject_and_deliver_to_emitter(world);
}

// ---------------------------------------------------------------------------
// Thens: CompensationContext built from the rejection.
// ---------------------------------------------------------------------------

fn context(world: &CompensationWorld) -> &CompensationContext {
    world.context.as_ref().expect("compensation context")
}

#[then("the context carries the rejected command")]
fn then_ctx_command(world: &mut CompensationWorld) {
    assert_eq!(
        context(world).rejected_command.as_ref(),
        world.rejected.as_ref()
    );
    assert_eq!(
        context(world).rejected_command_type(),
        full_type_url::<ReserveStock>()
    );
}

#[then("the context carries the rejection reason")]
fn then_ctx_reason(world: &mut CompensationWorld) {
    assert_eq!(context(world).rejection_reason, world.reason);
}

#[then("the context carries the saga origin")]
fn then_ctx_origin(world: &mut CompensationWorld) {
    let ctx = context(world);
    assert_eq!(ctx.source_aggregate, Some(world.origin.source_cover()));
    assert_eq!(ctx.source_event_sequence, world.origin.source_seq);
}

#[then("the saga origin is preserved")]
fn then_origin_preserved(world: &mut CompensationWorld) {
    let ctx = context(world);
    let source = ctx.source_aggregate.as_ref().expect("source aggregate");
    assert_eq!(source.domain, world.origin.source_domain);
    assert_eq!(ctx.source_event_sequence, world.origin.source_seq);
    let deferred = deferred_of(ctx.rejected_command.as_ref().expect("rejected command"));
    assert_eq!(deferred.source_component, world.origin.saga_name);
}

#[then("the correlation ID is preserved")]
fn then_correlation_preserved(world: &mut CompensationWorld) {
    let ctx = context(world);
    let cid = &world.origin.correlation_id;
    assert_eq!(
        ctx.rejected_command
            .as_ref()
            .and_then(|c| c.cover.as_ref())
            .map(|c| &c.correlation_id),
        Some(cid)
    );
    assert_eq!(
        ctx.source_aggregate.as_ref().map(|c| &c.correlation_id),
        Some(cid)
    );
}

// ---------------------------------------------------------------------------
// Thens: what the source aggregate's #[rejected] handler received.
// ---------------------------------------------------------------------------

#[then("the notification carries the rejected command")]
fn then_notification_command(world: &mut CompensationWorld) {
    assert_eq!(
        world.received_context().rejected_command.as_ref(),
        world.rejected.as_ref()
    );
}

#[then("the notification carries the rejection reason")]
fn then_notification_reason(world: &mut CompensationWorld) {
    assert_eq!(world.received_context().rejection_reason, world.reason);
}

#[then("the source aggregate and sequence are recorded")]
fn then_source_recorded(world: &mut CompensationWorld) {
    let ctx = world.received_context();
    assert_eq!(ctx.source_aggregate, Some(world.origin.source_cover()));
    assert_eq!(ctx.source_event_sequence, world.origin.source_seq);
}

#[then(expr = "the notification identifies the issuing saga as {string}")]
fn then_issuer(world: &mut CompensationWorld, saga: String) {
    let ctx = world.received_context();
    let deferred = deferred_of(ctx.rejected_command.as_ref().expect("rejected command"));
    assert_eq!(deferred.source_component, saga);
}

#[then("the notification has a cover")]
fn then_notification_cover(world: &mut CompensationWorld) {
    let cover = world
        .received_notification()
        .cover
        .expect("notification cover");
    assert_eq!(cover.domain, world.origin.source_domain);
    assert_eq!(cover.root, Some(root_for(&world.origin.source_root)));
}

#[then("the notification payload contains a RejectionNotification")]
fn then_payload_rejection(world: &mut CompensationWorld) {
    let payload = world.received_notification().payload.expect("payload");
    assert_eq!(payload.type_url, full_type_url::<RejectionNotification>());
    let ctx = world.received_context();
    assert_eq!(ctx.rejected_command.as_ref(), world.rejected.as_ref());
}

#[then("the notification carries its dispatch time")]
fn then_dispatch_time(world: &mut CompensationWorld) {
    let sent_at = world.received_notification().sent_at.expect("sent_at");
    assert_eq!(Some(sent_at), world.sent_at);
    assert!(sent_at.seconds > 0);
}

#[then("the command book targets the source aggregate")]
fn then_targets_source(world: &mut CompensationWorld) {
    let ctx = world.received_context();
    let handler = world.log.lock().unwrap().handler;
    assert_eq!(
        handler,
        Some(match world.origin.source_domain.as_str() {
            "fulfillment" => "fulfillment",
            _ => "orders",
        })
    );
    let source = ctx.source_aggregate.expect("source aggregate");
    assert_eq!(source.domain, world.origin.source_domain);
    assert_eq!(source.root, Some(root_for(&world.origin.source_root)));
    let envelope_cover = world
        .envelope
        .as_ref()
        .and_then(|e| e.cover.clone())
        .expect("envelope cover");
    assert_eq!(envelope_cover.domain, source.domain);
    assert_eq!(envelope_cover.root, source.root);
}

#[then("the command book preserves the correlation ID")]
fn then_command_book_correlation(world: &mut CompensationWorld) {
    let cover = world
        .received_notification()
        .cover
        .expect("notification cover");
    let rejected_cid = world
        .rejected()
        .cover
        .as_ref()
        .map(|c| c.correlation_id.clone())
        .expect("rejected cover");
    assert_eq!(cover.correlation_id, rejected_cid);
}

#[then(expr = "the rejection reason equals {string}")]
fn then_reason_equals(world: &mut CompensationWorld, reason: String) {
    assert_eq!(world.received_context().rejection_reason, reason);
}

#[then("the rejection reason carries the full error details")]
fn then_full_details(world: &mut CompensationWorld) {
    assert_eq!(world.received_context().rejection_reason, STRUCTURED_REASON);
}

#[then("the rejected command is the original command")]
fn then_original_command(world: &mut CompensationWorld) {
    assert_eq!(
        world.received_context().rejected_command.as_ref(),
        world.rejected.as_ref()
    );
}

#[then("all command fields are preserved")]
fn then_fields_preserved(world: &mut CompensationWorld) {
    let ctx = world.received_context();
    let cmd = ctx.rejected_command.as_ref().expect("rejected command");
    let payload = match cmd.pages.first().and_then(|p| p.payload.as_ref()) {
        Some(command_page::Payload::Command(a)) => a,
        other => panic!("expected a command payload, got {other:?}"),
    };
    assert_eq!(
        ReserveStock::decode(payload.value.as_slice()).expect("payload decodes"),
        ReserveStock {
            order_id: "o-77".into(),
            quantity: 12,
        }
    );
    assert_eq!(cmd.cover, world.rejected().cover);
    assert_eq!(cmd.pages[0].header, world.rejected().pages[0].header);
}

#[then("the full saga origin chain is preserved")]
fn then_chain_preserved(world: &mut CompensationWorld) {
    let ctx = world.received_context();
    let source = ctx.source_aggregate.as_ref().expect("source aggregate");
    assert_eq!(source.domain, "fulfillment");
    assert_eq!(ctx.source_event_sequence, 10);
    let deferred = deferred_of(ctx.rejected_command.as_ref().expect("rejected command"));
    assert_eq!(deferred.source_component, "fulfillment-shipping");
    assert_eq!(
        ctx.rejected_command_type(),
        full_type_url::<CreateShipment>()
    );
}

#[then("the root cause can be traced through the chain")]
fn then_root_cause(world: &mut CompensationWorld) {
    let ctx = world.received_context();
    let source_cid = ctx
        .source_aggregate
        .as_ref()
        .map(|c| c.correlation_id.clone())
        .expect("source aggregate");
    let command_cid = ctx
        .rejected_command
        .as_ref()
        .and_then(|c| c.cover.as_ref())
        .map(|c| c.correlation_id.clone())
        .expect("rejected command cover");
    assert_eq!(source_cid, "workflow-123");
    assert_eq!(command_cid, source_cid);
}

// ---------------------------------------------------------------------------
// Thens: rejections delivered back to sagas / process managers.
// ---------------------------------------------------------------------------

fn assert_emitter_received(world: &CompensationWorld, who: &'static str) {
    assert!(
        world.dispatch_error.is_none(),
        "delivery to the {who} failed: {:?}",
        world.dispatch_error
    );
    let handler = world.log.lock().unwrap().handler;
    assert_eq!(
        handler,
        Some(who),
        "the {who}'s #[rejected] handler did not run"
    );
    let ctx = world.received_context();
    assert_eq!(ctx.rejected_command.as_ref(), world.rejected.as_ref());
    assert_eq!(ctx.rejection_reason, "insufficient stock");
}

#[then("process manager rejections produce a compensation notification")]
fn then_pm_notification(world: &mut CompensationWorld) {
    assert_emitter_received(world, "pm");
}

// ---------------------------------------------------------------------------
// C-0483: the source aggregate compensates a rejected saga command.
// ---------------------------------------------------------------------------

#[given(
    expr = "a saga {string} translating OrderCreated from {string} into ReserveStock for {string}"
)]
fn given_fulfillment_saga(
    _world: &mut CompensationWorld,
    name: String,
    source: String,
    target: String,
) {
    let config = <OrderFulfillmentSaga as angzarr_client::router::HandlerKind>::handler_config();
    let angzarr_client::router::HandlerConfig::Saga {
        name: n,
        source: s,
        target: t,
        ..
    } = config
    else {
        panic!("OrderFulfillment is not a saga");
    };
    assert_eq!((n, s, t), (name, source, target));
}

#[given("the order aggregate compensates a rejected ReserveStock by emitting OrderCancelled")]
fn given_order_compensates(_world: &mut CompensationWorld) {
    let config = <OrderAggregate as angzarr_client::router::HandlerKind>::handler_config();
    let angzarr_client::router::HandlerConfig::CommandHandler {
        domain, rejected, ..
    } = config
    else {
        panic!("OrderAggregate is not a command handler");
    };
    assert_eq!(domain, "order");
    assert_eq!(
        rejected,
        vec![("inventory".to_string(), "ReserveStock".to_string())]
    );
}

#[when(
    expr = "a RejectionNotification for the saga's ReserveStock command with source {string} is dispatched to the order aggregate's router"
)]
fn when_rejection_to_source(world: &mut CompensationWorld, source: String) {
    // The saga emits the command from an OrderCreated in `source`...
    let calls = world.saga_calls.clone();
    let Ok(Built::Saga(saga)) = Router::new("saga")
        .with_handler(move || OrderFulfillmentSaga {
            calls: calls.clone(),
        })
        .build()
    else {
        panic!("expected a saga router");
    };
    let source_cover = Cover {
        domain: source.clone(),
        root: Some(root_for("order-1")),
        correlation_id: "corr-1".into(),
        ..Default::default()
    };
    let emitted = saga
        .dispatch(SagaHandleRequest {
            source: Some(EventBook {
                cover: Some(source_cover.clone()),
                pages: vec![EventPage {
                    header: Some(PageHeader {
                        sequence_type: Some(page_header::SequenceType::Sequence(0)),
                        sync_mode: None,
                    }),
                    payload: Some(event_page::Payload::Event(pack(&OrderCreated {}))),
                    ..Default::default()
                }],
                next_sequence: 1,
                ..Default::default()
            }),
            ..Default::default()
        })
        .expect("saga dispatch");
    let mut rejected = emitted
        .commands
        .into_iter()
        .next()
        .expect("saga emitted a command");
    angzarr_client::Destinations::new(["inventory"])
        .stamp_command(&mut rejected, "inventory", &source_cover, 0, 0)
        .expect("stamp deferred provenance");
    world.saga_calls_at_rejection = Some(world.saga_calls.load(Ordering::SeqCst));

    // ...inventory rejects it, and the rejection goes to its source.
    let notification = Notification {
        cover: Some(source_cover.clone()),
        payload: Some(pack(&RejectionNotification {
            rejected_command: Some(rejected),
            rejection_reason: "insufficient stock".into(),
        })),
        sent_at: None,
    };
    let Ok(Built::CommandHandler(order)) =
        Router::new("order").with_handler(|| OrderAggregate).build()
    else {
        panic!("expected a command-handler router");
    };
    let response = order
        .dispatch(ContextualCommand {
            command: Some(CommandBook {
                cover: Some(source_cover),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(pack(&notification))),
                    ..Default::default()
                }],
            }),
            events: None,
        })
        .expect("order aggregate dispatch");
    world.source_response = Some(response);
}

#[then("the response contains one OrderCancelled event")]
fn then_one_order_cancelled(world: &mut CompensationWorld) {
    let response = world.source_response.as_ref().expect("source response");
    let Some(angzarr_client::proto::business_response::Result::Events(book)) = &response.result
    else {
        panic!("expected events, got {response:?}");
    };
    assert_eq!(book.pages.len(), 1);
    match &book.pages[0].payload {
        Some(event_page::Payload::Event(any)) => {
            assert!(angzarr_client::type_url_is::<OrderCancelled>(&any.type_url));
        }
        other => panic!("expected an event, got {other:?}"),
    }
}

#[then("no OrderFulfillment saga handler is invoked")]
fn then_no_saga_invoked(world: &mut CompensationWorld) {
    let at_rejection = world.saga_calls_at_rejection.expect("rejection dispatched");
    assert_eq!(at_rejection, 1, "the saga emitted the command once");
    assert_eq!(world.saga_calls.load(Ordering::SeqCst), at_rejection);
}
