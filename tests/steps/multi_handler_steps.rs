//! Step definitions for `features/client/multi_handler.feature`.
//!
//! Command-handler uniqueness is checked at `Router::build`; saga, PM and
//! projector fan-out is observed by dispatching through the built runtime
//! routers and counting what each handler did.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use angzarr_client::error_codes::codes;
use angzarr_client::proto::{
    EventBook, ProcessManagerHandleRequest, ProcessManagerHandleResponse, SagaHandleRequest,
    SagaResponse,
};
use angzarr_client::router::{BuildError, Built, Router};
use angzarr_client::{command_handler, process_manager, projector, saga, CommandResult};
use cucumber::{given, then, when, World};

use super::deferred::{command_type_url, trigger_book, unsequenced_command};
use crate::common::fixtures::{CreateOrder, CreateShipment, OrderCreated, ReserveStock};

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AddItem {
    #[prost(string, tag = "1")]
    pub sku: ::prost::alloc::string::String,
}

impl ::prost::Name for AddItem {
    const NAME: &'static str = "AddItem";
    const PACKAGE: &'static str = "examples";
}

/// Invocation log shared by the handlers of one scenario.
type Log = Arc<Mutex<Vec<String>>>;

fn record(log: &Log, entry: &str) {
    log.lock().unwrap().push(entry.to_string());
}

#[derive(Default)]
pub struct S;

// --- command handlers ------------------------------------------------------

pub struct AlphaOrder;

#[command_handler(domain = "order", state = S)]
impl AlphaOrder {
    #[handles(CreateOrder)]
    fn on_create(&self, _cmd: CreateOrder, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

pub struct BetaOrder;

#[command_handler(domain = "order", state = S)]
impl BetaOrder {
    #[handles(CreateOrder)]
    fn on_create(&self, _cmd: CreateOrder, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

pub struct AlphaOrderA;

#[command_handler(domain = "orderA", state = S)]
impl AlphaOrderA {
    #[handles(CreateOrder)]
    fn on_create(&self, _cmd: CreateOrder, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

pub struct BetaOrderB;

#[command_handler(domain = "orderB", state = S)]
impl BetaOrderB {
    #[handles(CreateOrder)]
    fn on_create(&self, _cmd: CreateOrder, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

pub struct OrderTwoTypes;

#[command_handler(domain = "order", state = S)]
impl OrderTwoTypes {
    #[handles(CreateOrder)]
    fn on_create(&self, _cmd: CreateOrder, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }

    #[handles(AddItem)]
    fn on_add(&self, _cmd: AddItem, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

// --- sagas -----------------------------------------------------------------

pub struct SagaA {
    log: Log,
}

#[saga(name = "SagaA", source = "order", target = "inventory")]
impl SagaA {
    #[handles(OrderCreated)]
    fn on_created(&self, _event: OrderCreated) -> CommandResult<SagaResponse> {
        record(&self.log, "SagaA");
        Ok(SagaResponse {
            commands: vec![unsequenced_command(&ReserveStock::default(), "inventory")],
            events: vec![],
        })
    }
}

pub struct SagaB {
    log: Log,
}

#[saga(name = "SagaB", source = "order", target = "fulfillment")]
impl SagaB {
    #[handles(OrderCreated)]
    fn on_created(&self, _event: OrderCreated) -> CommandResult<SagaResponse> {
        record(&self.log, "SagaB");
        Ok(SagaResponse {
            commands: vec![unsequenced_command(
                &CreateShipment::default(),
                "fulfillment",
            )],
            events: vec![],
        })
    }
}

// --- process managers ------------------------------------------------------

pub struct PMA {
    log: Log,
}

#[process_manager(
    name = "PMA",
    pm_domain = "pma",
    sources = ["order"],
    targets = ["inventory"],
    state = S
)]
impl PMA {
    #[handles(OrderCreated)]
    fn on_created(
        &self,
        _event: OrderCreated,
        _state: &S,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        record(&self.log, "PMA");
        Ok(ProcessManagerHandleResponse {
            commands: vec![unsequenced_command(&ReserveStock::default(), "inventory")],
            ..Default::default()
        })
    }
}

pub struct PMB {
    log: Log,
}

#[process_manager(
    name = "PMB",
    pm_domain = "pmb",
    sources = ["order"],
    targets = ["fulfillment"],
    state = S
)]
impl PMB {
    #[handles(OrderCreated)]
    fn on_created(
        &self,
        _event: OrderCreated,
        _state: &S,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        record(&self.log, "PMB");
        Ok(ProcessManagerHandleResponse {
            commands: vec![unsequenced_command(
                &CreateShipment::default(),
                "fulfillment",
            )],
            ..Default::default()
        })
    }
}

// --- projectors ------------------------------------------------------------

pub struct ProjA {
    logs: Arc<Mutex<HashMap<&'static str, Vec<String>>>>,
}

#[projector(name = "ProjA", domains = ["order"])]
impl ProjA {
    #[handles(OrderCreated)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<()> {
        self.logs
            .lock()
            .unwrap()
            .entry("ProjA")
            .or_default()
            .push(event.order_id);
        Ok(())
    }
}

pub struct ProjB {
    logs: Arc<Mutex<HashMap<&'static str, Vec<String>>>>,
}

#[projector(name = "ProjB", domains = ["order"])]
impl ProjB {
    #[handles(OrderCreated)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<()> {
        self.logs
            .lock()
            .unwrap()
            .entry("ProjB")
            .or_default()
            .push(event.order_id);
        Ok(())
    }
}

// --- world -----------------------------------------------------------------

#[derive(Debug, Default, World)]
pub struct MultiHandlerWorld {
    log: Log,
    projector_logs: Arc<Mutex<HashMap<&'static str, Vec<String>>>>,
    build_result: Option<Result<(), BuildError>>,
    saga_router: Option<angzarr_client::router::SagaRouter>,
    pm_router: Option<angzarr_client::router::ProcessManagerRouter>,
    projector_router: Option<angzarr_client::router::ProjectorRouter>,
    saga_response: Option<SagaResponse>,
    pm_response: Option<ProcessManagerHandleResponse>,
}

impl MultiHandlerWorld {
    fn record_build(&mut self, result: Result<Built, BuildError>) {
        self.build_result = Some(result.map(|built| {
            assert!(matches!(built, Built::CommandHandler(_)), "got {built:?}");
        }));
    }

    fn command_domains(&self) -> Vec<String> {
        self.saga_response
            .as_ref()
            .map(|r| &r.commands)
            .or(self.pm_response.as_ref().map(|r| &r.commands))
            .expect("a dispatch response")
            .iter()
            .map(|c| {
                c.cover
                    .as_ref()
                    .map(|c| c.domain.clone())
                    .unwrap_or_default()
            })
            .collect()
    }
}

// --- Given -----------------------------------------------------------------

#[given(expr = "two command handlers Alpha and Beta for domain {string}")]
fn given_alpha_beta(_world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(domain, "order");
}

#[given("both handle CreateOrder")]
fn given_both_handle(_world: &mut MultiHandlerWorld) {
    // AlphaOrder and BetaOrder both declare `#[handles(CreateOrder)]`.
}

#[given(expr = "a command handler Alpha for domain {string} handling CreateOrder")]
fn given_alpha_domain(_world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(domain, "orderA");
}

#[given(expr = "a command handler Beta for domain {string} handling CreateOrder")]
fn given_beta_domain(_world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(domain, "orderB");
}

#[given(expr = "a command handler Order for domain {string} handling CreateOrder and AddItem")]
fn given_two_types(_world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(domain, "order");
}

#[given(expr = "two sagas SagaA and SagaB both listening to source {string} for OrderCreated")]
fn given_two_sagas(_world: &mut MultiHandlerWorld, source: String) {
    assert_eq!(source, "order");
}

#[given(expr = "SagaA emits a ReserveStock command for {string}")]
fn given_saga_a(_world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(domain, "inventory");
}

#[given(expr = "SagaB emits a CreateShipment command for {string}")]
fn given_saga_b(_world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(domain, "fulfillment");
}

#[given("the saga router is built with SagaA then SagaB")]
fn given_saga_router(world: &mut MultiHandlerWorld) {
    let (la, lb) = (Arc::clone(&world.log), Arc::clone(&world.log));
    let built = Router::new("sagas")
        .with_handler(move || SagaA {
            log: Arc::clone(&la),
        })
        .with_handler(move || SagaB {
            log: Arc::clone(&lb),
        })
        .build()
        .expect("saga router builds");
    let Built::Saga(router) = built else {
        panic!("expected a saga router");
    };
    world.saga_router = Some(router);
}

#[given(
    expr = "two process managers PMA and PMB both sourcing from {string} and handling OrderCreated"
)]
fn given_two_pms(_world: &mut MultiHandlerWorld, source: String) {
    assert_eq!(source, "order");
}

#[given("PMA emits a ReserveStock command")]
fn given_pma(_world: &mut MultiHandlerWorld) {
    // PMA's OrderCreated handler emits ReserveStock to inventory.
}

#[given("PMB emits a CreateShipment command")]
fn given_pmb(_world: &mut MultiHandlerWorld) {
    // PMB's OrderCreated handler emits CreateShipment to fulfillment.
}

#[given("the PM router is built with PMA then PMB")]
fn given_pm_router(world: &mut MultiHandlerWorld) {
    let (la, lb) = (Arc::clone(&world.log), Arc::clone(&world.log));
    let built = Router::new("pms")
        .with_handler(move || PMA {
            log: Arc::clone(&la),
        })
        .with_handler(move || PMB {
            log: Arc::clone(&lb),
        })
        .build()
        .expect("PM router builds");
    let Built::ProcessManager(router) = built else {
        panic!("expected a PM router");
    };
    world.pm_router = Some(router);
}

#[given(expr = "two projectors ProjA and ProjB both consuming domain {string}")]
fn given_two_projectors(_world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(domain, "order");
}

#[given("ProjA appends to a log on OrderCreated")]
fn given_proj_a(_world: &mut MultiHandlerWorld) {
    // ProjA records under its own key.
}

#[given("ProjB appends to a different log on OrderCreated")]
fn given_proj_b(_world: &mut MultiHandlerWorld) {
    // ProjB records under its own key.
}

#[given("the projector router is built with ProjA then ProjB")]
fn given_projector_router(world: &mut MultiHandlerWorld) {
    let (la, lb) = (
        Arc::clone(&world.projector_logs),
        Arc::clone(&world.projector_logs),
    );
    let built = Router::new("projectors")
        .with_handler(move || ProjA {
            logs: Arc::clone(&la),
        })
        .with_handler(move || ProjB {
            logs: Arc::clone(&lb),
        })
        .build()
        .expect("projector router builds");
    let Built::Projector(router) = built else {
        panic!("expected a projector router");
    };
    world.projector_router = Some(router);
}

// --- When ------------------------------------------------------------------

#[when("the router is built with Alpha then Beta")]
fn when_build_dup(world: &mut MultiHandlerWorld) {
    let result = Router::new("order")
        .with_handler(|| AlphaOrder)
        .with_handler(|| BetaOrder)
        .build();
    world.record_build(result);
}

#[when("the router is built with Alpha then Beta across domains")]
fn when_build_domains(world: &mut MultiHandlerWorld) {
    let result = Router::new("orders")
        .with_handler(|| AlphaOrderA)
        .with_handler(|| BetaOrderB)
        .build();
    world.record_build(result);
}

#[when("the router is built with Order")]
fn when_build_order(world: &mut MultiHandlerWorld) {
    let result = Router::new("order").with_handler(|| OrderTwoTypes).build();
    world.record_build(result);
}

#[when("an OrderCreated event is dispatched to the saga router")]
fn when_saga(world: &mut MultiHandlerWorld) {
    let router = world.saga_router.as_ref().expect("saga router");
    let response = router
        .dispatch(SagaHandleRequest {
            source: Some(trigger_book(
                &OrderCreated::default(),
                "order",
                "order-1",
                0,
            )),
            ..Default::default()
        })
        .expect("saga dispatch");
    world.saga_response = Some(response);
}

#[when("an OrderCreated trigger is dispatched to the PM router")]
fn when_pm(world: &mut MultiHandlerWorld) {
    let router = world.pm_router.as_ref().expect("PM router");
    let response = router
        .dispatch(ProcessManagerHandleRequest {
            trigger: Some(trigger_book(
                &OrderCreated::default(),
                "order",
                "order-1",
                0,
            )),
            ..Default::default()
        })
        .expect("PM dispatch");
    world.pm_response = Some(response);
}

#[when("an EventBook with one OrderCreated event is dispatched")]
fn when_projectors(world: &mut MultiHandlerWorld) {
    let router = world.projector_router.as_ref().expect("projector router");
    let book = trigger_book(
        &OrderCreated {
            order_id: "o-1".into(),
            ..Default::default()
        },
        "order",
        "order-1",
        0,
    );
    router.dispatch(book).expect("projector dispatch");
}

// --- Then ------------------------------------------------------------------

#[then(
    expr = "registration is rejected because two command handlers claim CreateOrder in {string}"
)]
fn then_dup_rejected(world: &mut MultiHandlerWorld, domain: String) {
    let err = match world.build_result.take() {
        Some(Err(e)) => e,
        other => panic!("expected a build error, got {other:?}"),
    };
    assert_eq!(err.code(), codes::DUPLICATE_COMMAND_HANDLER);
    assert_eq!(err.details().get("domain"), Some(&domain));
    assert_eq!(
        err.details().get("type_url"),
        Some(&angzarr_client::full_type_url::<CreateOrder>())
    );
}

#[then("the configuration is accepted")]
fn then_accepted(world: &mut MultiHandlerWorld) {
    match world.build_result.take() {
        Some(Ok(())) => {}
        other => panic!("expected the build to succeed, got {other:?}"),
    }
}

#[then("the response contains two commands in registration order")]
fn then_two_in_order(world: &mut MultiHandlerWorld) {
    let commands = world
        .saga_response
        .as_ref()
        .map(|r| r.commands.clone())
        .or(world.pm_response.as_ref().map(|r| r.commands.clone()))
        .expect("a dispatch response");
    let urls: Vec<String> = commands.iter().map(command_type_url).collect();
    assert_eq!(
        urls,
        vec![
            angzarr_client::full_type_url::<ReserveStock>(),
            angzarr_client::full_type_url::<CreateShipment>()
        ]
    );
    let expected_log = if world.saga_response.is_some() {
        vec!["SagaA", "SagaB"]
    } else {
        vec!["PMA", "PMB"]
    };
    assert_eq!(*world.log.lock().unwrap(), expected_log);
}

#[then(expr = "the first command targets the {string} domain")]
fn then_first(world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(world.command_domains()[0], domain);
}

#[then(expr = "the second command targets the {string} domain")]
fn then_second(world: &mut MultiHandlerWorld, domain: String) {
    assert_eq!(world.command_domains()[1], domain);
}

#[then(expr = "ProjA's log has {int} entry")]
fn then_proj_a(world: &mut MultiHandlerWorld, n: usize) {
    let logs = world.projector_logs.lock().unwrap();
    assert_eq!(logs.get("ProjA").map(Vec::len), Some(n));
}

#[then(expr = "ProjB's log has {int} entry")]
fn then_proj_b(world: &mut MultiHandlerWorld, n: usize) {
    let logs = world.projector_logs.lock().unwrap();
    assert_eq!(logs.get("ProjB").map(Vec::len), Some(n));
}

#[then("each saga handles the event exactly once")]
fn then_each_once(world: &mut MultiHandlerWorld) {
    assert_eq!(*world.log.lock().unwrap(), vec!["SagaA", "SagaB"]);
    assert_eq!(
        world.saga_response.as_ref().map(|r| r.commands.len()),
        Some(2)
    );
}
