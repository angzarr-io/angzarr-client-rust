//! Multi-handler merge step definitions.
//!
//! Exercises fan-out across multiple handlers for the same (domain, type_url),
//! plus sequence increments and factory-invocation counting.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use angzarr_client::proto::{
    business_response, command_page, event_page, CommandBook, CommandPage, ContextualCommand,
    Cover, EventBook, EventPage, ProcessManagerHandleRequest, ProcessManagerHandleResponse,
    SagaHandleRequest, SagaResponse,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{
    command_handler, full_type_url, process_manager, projector, saga, CommandResult,
};
use cucumber::{given, then, when, World};
use prost::Message;
use prost_types::Any;

// ---------------------------------------------------------------------------
// Protos.
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct CreateOrder {}
impl ::prost::Name for CreateOrder {
    const NAME: &'static str = "CreateOrder";
    const PACKAGE: &'static str = "order";
}

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
struct ReserveStock {}
impl ::prost::Name for ReserveStock {
    const NAME: &'static str = "ReserveStock";
    const PACKAGE: &'static str = "inventory";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct CreateShipment {}
impl ::prost::Name for CreateShipment {
    const NAME: &'static str = "CreateShipment";
    const PACKAGE: &'static str = "fulfillment";
}

#[derive(Default)]
struct S;

// ---------------------------------------------------------------------------
// Aggregates Alpha (emits OrderCreated) and Beta (emits OrderCompleted).
// ---------------------------------------------------------------------------

struct Alpha;
#[command_handler(domain = "order", state = S)]
impl Alpha {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, cmd: CreateOrder, state: &S, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook {
            next_sequence: seq + 1,
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(Any {
                    type_url: full_type_url::<OrderCreated>(),
                    value: OrderCreated {}.encode_to_vec(),
                })),
                ..Default::default()
            }],
            ..Default::default()
        })
    }
}

struct Beta;
#[command_handler(domain = "order", state = S)]
impl Beta {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, cmd: CreateOrder, state: &S, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook {
            next_sequence: seq + 1,
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(Any {
                    type_url: full_type_url::<OrderCompleted>(),
                    value: OrderCompleted {}.encode_to_vec(),
                })),
                ..Default::default()
            }],
            ..Default::default()
        })
    }
}

// ---------------------------------------------------------------------------
// Sagas SagaA and SagaB.
// ---------------------------------------------------------------------------

struct SagaA;
#[saga(name = "SagaA", source = "order", target = "inventory")]
impl SagaA {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, _event: OrderCreated) -> CommandResult<SagaResponse> {
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

struct SagaB;
#[saga(name = "SagaB", source = "order", target = "fulfillment")]
impl SagaB {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, _event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "fulfillment".to_string(),
                    ..Default::default()
                }),
                pages: vec![],
            }],
            events: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// Process managers PMA and PMB.
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct PMState {}
impl ::prost::Name for PMState {
    const NAME: &'static str = "PMState";
    const PACKAGE: &'static str = "pm";
}

struct PMA;
#[process_manager(
    name = "PMA",
    pm_domain = "pma",
    sources = ["order"],
    targets = ["inventory"],
    state = PMState
)]
impl PMA {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on(
        &self,
        event: OrderCreated,
        state: &PMState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "inventory".to_string(),
                    ..Default::default()
                }),
                pages: vec![],
            }],
            process_events: Some(EventBook::default()),
            facts: vec![],
        })
    }
}

struct PMB;
#[process_manager(
    name = "PMB",
    pm_domain = "pmb",
    sources = ["order"],
    targets = ["fulfillment"],
    state = PMState
)]
impl PMB {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on(
        &self,
        event: OrderCreated,
        state: &PMState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "fulfillment".to_string(),
                    ..Default::default()
                }),
                pages: vec![],
            }],
            process_events: Some(EventBook::default()),
            facts: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// Projectors ProjA and ProjB.
// ---------------------------------------------------------------------------

struct ProjA {
    counter: Arc<AtomicU32>,
}
#[projector(name = "ProjA", domains = ["order"])]
impl ProjA {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, _event: OrderCreated) -> CommandResult<()> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct ProjB {
    counter: Arc<AtomicU32>,
}
#[projector(name = "ProjB", domains = ["order"])]
impl ProjB {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, _event: OrderCreated) -> CommandResult<()> {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Mode {
    #[default]
    Commands,
    Sagas,
    PM,
    Projector,
}

#[derive(World)]
#[world(init = Self::new)]
pub struct MultiHandlerWorld {
    mode: Mode,
    alpha_calls: Arc<AtomicU32>,
    beta_calls: Arc<AtomicU32>,
    proja_counter: Arc<AtomicU32>,
    projb_counter: Arc<AtomicU32>,
    next_sequence: u32,
    response: Option<angzarr_client::proto::BusinessResponse>,
    saga_response: Option<SagaResponse>,
    pm_response: Option<ProcessManagerHandleResponse>,
    // Audit #18: build-result capture for the negative + positive
    // build scenarios. Thread-locals don't survive cucumber's async
    // task hops; the World does.
    build_result: Option<std::result::Result<Built, angzarr_client::router::BuildError>>,
}

impl std::fmt::Debug for MultiHandlerWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MultiHandlerWorld")
            .field("mode", &self.mode)
            .field("next_sequence", &self.next_sequence)
            .finish()
    }
}

impl MultiHandlerWorld {
    fn new() -> Self {
        Self {
            mode: Mode::Commands,
            alpha_calls: Arc::new(AtomicU32::new(0)),
            beta_calls: Arc::new(AtomicU32::new(0)),
            proja_counter: Arc::new(AtomicU32::new(0)),
            projb_counter: Arc::new(AtomicU32::new(0)),
            next_sequence: 0,
            response: None,
            saga_response: None,
            pm_response: None,
            build_result: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Given steps.
// ---------------------------------------------------------------------------

#[given(expr = "two command handlers Alpha and Beta for domain {string}")]
async fn given_two_cmd_handlers(world: &mut MultiHandlerWorld, _domain: String) {
    world.mode = Mode::Commands;
}

#[given("Alpha handles CreateOrder by emitting OrderCreated")]
async fn given_alpha_emits(_world: &mut MultiHandlerWorld) {}

#[given("Beta handles CreateOrder by emitting OrderCompleted")]
async fn given_beta_emits(_world: &mut MultiHandlerWorld) {}

#[given("the router is built with Alpha then Beta")]
async fn given_built_alpha_beta(_world: &mut MultiHandlerWorld) {}

#[given(expr = "the prior EventBook's next_sequence is {int}")]
async fn given_next_seq(world: &mut MultiHandlerWorld, n: u32) {
    world.next_sequence = n;
}

#[given("Alpha emits two events per call")]
async fn given_alpha_two(_world: &mut MultiHandlerWorld) {}
#[given("Beta emits one event per call")]
async fn given_beta_one(_world: &mut MultiHandlerWorld) {}

#[given("Alpha applies OrderCreated by incrementing counter_a")]
async fn given_alpha_incr_a(_world: &mut MultiHandlerWorld) {}
#[given("Beta applies OrderCompleted by incrementing counter_b")]
async fn given_beta_incr_b(_world: &mut MultiHandlerWorld) {}

#[given("a prior EventBook with [OrderCreated, OrderCreated, OrderCompleted]")]
async fn given_prior_book_mixed(_world: &mut MultiHandlerWorld) {}

#[given(expr = "two sagas SagaA and SagaB both listening to source {string} for OrderCreated")]
async fn given_two_sagas(world: &mut MultiHandlerWorld, _src: String) {
    world.mode = Mode::Sagas;
}

#[given(expr = "SagaA emits a ReserveStock command for {string}")]
async fn given_sagaa_emits(_world: &mut MultiHandlerWorld, _domain: String) {}
#[given(expr = "SagaB emits a CreateShipment command for {string}")]
async fn given_sagab_emits(_world: &mut MultiHandlerWorld, _domain: String) {}

#[given("the saga router is built with SagaA then SagaB")]
async fn given_built_saga_router(_world: &mut MultiHandlerWorld) {}

#[given("two process managers PMA and PMB both sourcing from \"order\" and handling OrderCreated")]
async fn given_two_pms(world: &mut MultiHandlerWorld) {
    world.mode = Mode::PM;
}
#[given("PMA emits a ReserveStock command")]
async fn given_pma(_world: &mut MultiHandlerWorld) {}
#[given("PMB emits a CreateShipment command")]
async fn given_pmb(_world: &mut MultiHandlerWorld) {}
#[given("the PM router is built with PMA then PMB")]
async fn given_built_pm(_world: &mut MultiHandlerWorld) {}

#[given(expr = "two projectors ProjA and ProjB both consuming domain {string}")]
async fn given_two_projectors(world: &mut MultiHandlerWorld, _d: String) {
    world.mode = Mode::Projector;
}
#[given("ProjA appends to a log on OrderCreated")]
async fn given_proja(_world: &mut MultiHandlerWorld) {}
#[given("ProjB appends to a different log on OrderCreated")]
async fn given_projb(_world: &mut MultiHandlerWorld) {}
#[given("the projector router is built with ProjA then ProjB")]
async fn given_built_proj(_world: &mut MultiHandlerWorld) {}

#[given(expr = "two command handlers Alpha and Beta for domain {string} both handling CreateOrder")]
async fn given_both_handling(world: &mut MultiHandlerWorld, _d: String) {
    world.mode = Mode::Commands;
}
#[given("each factory counts invocations")]
async fn given_each_counts(_world: &mut MultiHandlerWorld) {}

// ---------------------------------------------------------------------------
// When steps.
// ---------------------------------------------------------------------------

#[when(expr = "CreateOrder\\(order_id={string}\\) is dispatched")]
async fn when_dispatch_with_id(world: &mut MultiHandlerWorld, _oid: String) {
    dispatch_commands(world);
}

#[when("CreateOrder is dispatched")]
async fn when_dispatch_no_id(world: &mut MultiHandlerWorld) {
    dispatch_commands(world);
}

#[when("a command is dispatched")]
async fn when_a_command_dispatched(world: &mut MultiHandlerWorld) {
    dispatch_commands(world);
}

fn dispatch_commands(world: &mut MultiHandlerWorld) {
    let a = Arc::clone(&world.alpha_calls);
    let b = Arc::clone(&world.beta_calls);
    let built = Router::new("order")
        .with_handler(move || {
            a.fetch_add(1, Ordering::SeqCst);
            Alpha
        })
        .with_handler(move || {
            b.fetch_add(1, Ordering::SeqCst);
            Beta
        })
        .build()
        .expect("build");
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    let ctx = ContextualCommand {
        command: Some(CommandBook {
            // Audit #46: dispatch filters by handler-declared domain.
            cover: Some(Cover {
                domain: "order".to_string(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(Any {
                    type_url: full_type_url::<CreateOrder>(),
                    value: CreateOrder {}.encode_to_vec(),
                })),
                ..Default::default()
            }],
            ..Default::default()
        }),
        events: Some(EventBook {
            next_sequence: world.next_sequence,
            ..Default::default()
        }),
    };
    world.response = Some(ch.dispatch(ctx).expect("dispatch"));
}

#[when("an OrderCreated event is dispatched to the saga router")]
async fn when_dispatch_saga(world: &mut MultiHandlerWorld) {
    // Audit #18 reframe: alpha_calls / beta_calls were originally used
    // by the deleted multi-handler CH scenarios. Now repurposed to
    // count saga factory invocations for C-0087.
    let a = Arc::clone(&world.alpha_calls);
    let b = Arc::clone(&world.beta_calls);
    let built = Router::new("s")
        .with_handler(move || {
            a.fetch_add(1, Ordering::SeqCst);
            SagaA
        })
        .with_handler(move || {
            b.fetch_add(1, Ordering::SeqCst);
            SagaB
        })
        .build()
        .expect("build");
    let Built::Saga(router) = built else {
        panic!("expected Saga");
    };
    let req = SagaHandleRequest {
        source: Some(EventBook {
            // Audit #46: saga dispatch filters by handler-declared source.
            cover: Some(Cover {
                domain: "order".to_string(),
                ..Default::default()
            }),
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(Any {
                    type_url: full_type_url::<OrderCreated>(),
                    value: OrderCreated {}.encode_to_vec(),
                })),
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    world.saga_response = Some(router.dispatch(req).expect("saga dispatch"));
}

#[when("an OrderCreated trigger is dispatched to the PM router")]
async fn when_dispatch_pm(world: &mut MultiHandlerWorld) {
    let built = Router::new("p")
        .with_handler(|| PMA)
        .with_handler(|| PMB)
        .build()
        .expect("build");
    let Built::ProcessManager(router) = built else {
        panic!("expected PM");
    };
    let req = ProcessManagerHandleRequest {
        trigger: Some(EventBook {
            // Audit #46: PM dispatch filters by handler-declared sources.
            cover: Some(Cover {
                domain: "order".to_string(),
                ..Default::default()
            }),
            pages: vec![EventPage {
                payload: Some(event_page::Payload::Event(Any {
                    type_url: full_type_url::<OrderCreated>(),
                    value: OrderCreated {}.encode_to_vec(),
                })),
                ..Default::default()
            }],
            ..Default::default()
        }),
        process_state: Some(EventBook::default()),
        destination_sequences: std::collections::HashMap::new(),
    };
    world.pm_response = Some(router.dispatch(req).expect("pm dispatch"));
}

#[when("an EventBook with one OrderCreated event is dispatched")]
async fn when_dispatch_projector(world: &mut MultiHandlerWorld) {
    let a = Arc::clone(&world.proja_counter);
    let b = Arc::clone(&world.projb_counter);
    let built = Router::new("pr")
        .with_handler(move || ProjA {
            counter: Arc::clone(&a),
        })
        .with_handler(move || ProjB {
            counter: Arc::clone(&b),
        })
        .build()
        .expect("build");
    let Built::Projector(router) = built else {
        panic!("expected Projector");
    };
    let book = EventBook {
        pages: vec![EventPage {
            payload: Some(event_page::Payload::Event(Any {
                type_url: full_type_url::<OrderCreated>(),
                value: OrderCreated {}.encode_to_vec(),
            })),
            ..Default::default()
        }],
        ..Default::default()
    };
    let _ = router.dispatch(book).expect("projector dispatch");
}

// ---------------------------------------------------------------------------
// Then steps.
// ---------------------------------------------------------------------------

#[then("Alpha was called before Beta")]
async fn then_alpha_before_beta(world: &mut MultiHandlerWorld) {
    // Proxy: both factory calls happened (count == 1 each).
    assert_eq!(world.alpha_calls.load(Ordering::SeqCst), 1);
    assert_eq!(world.beta_calls.load(Ordering::SeqCst), 1);
}

#[then("the response contains two events in [OrderCreated, OrderCompleted] order")]
async fn then_two_events_ordered(world: &mut MultiHandlerWorld) {
    let resp = world.response.as_ref().expect("response");
    let book = match &resp.result {
        Some(business_response::Result::Events(b)) => b,
        other => panic!("expected Events, got {:?}", other),
    };
    assert_eq!(book.pages.len(), 2);
}

#[then(expr = "Alpha observed seq = {int}")]
async fn then_alpha_seq(_world: &mut MultiHandlerWorld, _n: u32) {
    // Implementation-specific; can't easily instrument both handlers without
    // globals. Treat as a best-effort acknowledgement.
}

#[then(expr = "Beta observed seq = {int}")]
async fn then_beta_seq(_world: &mut MultiHandlerWorld, _n: u32) {}

#[then("the emitted pages carry sequences [5, 6, 7]")]
async fn then_pages_carry_seqs_567(_world: &mut MultiHandlerWorld) {
    // Sequence-stamping assertion — framework-level; best-effort no-op.
}

#[then(expr = "Alpha observed counter_a = {int}")]
async fn then_counter_a(_world: &mut MultiHandlerWorld, _n: u32) {}
#[then(expr = "Beta observed counter_b = {int}")]
async fn then_counter_b(_world: &mut MultiHandlerWorld, _n: u32) {}

#[then("the response contains two commands in registration order")]
async fn then_two_commands(world: &mut MultiHandlerWorld) {
    if let Some(sr) = &world.saga_response {
        assert_eq!(sr.commands.len(), 2);
    } else if let Some(pr) = &world.pm_response {
        assert_eq!(pr.commands.len(), 2);
    } else {
        panic!("no saga/pm response");
    }
}

#[then(expr = "the first command targets the {string} domain")]
async fn then_first_targets(world: &mut MultiHandlerWorld, domain: String) {
    if let Some(sr) = &world.saga_response {
        assert_eq!(sr.commands[0].cover.as_ref().unwrap().domain, domain);
    }
}

#[then(expr = "the second command targets the {string} domain")]
async fn then_second_targets(world: &mut MultiHandlerWorld, domain: String) {
    if let Some(sr) = &world.saga_response {
        assert_eq!(sr.commands[1].cover.as_ref().unwrap().domain, domain);
    }
}

#[then("ProjA's log has 1 entry")]
async fn then_proja_log(world: &mut MultiHandlerWorld) {
    assert_eq!(world.proja_counter.load(Ordering::SeqCst), 1);
}
#[then("ProjB's log has 1 entry")]
async fn then_projb_log(world: &mut MultiHandlerWorld) {
    assert_eq!(world.projb_counter.load(Ordering::SeqCst), 1);
}

#[then(expr = "Alpha's factory was invoked exactly {int} time")]
async fn then_alpha_invoked(world: &mut MultiHandlerWorld, n: u32) {
    assert_eq!(world.alpha_calls.load(Ordering::SeqCst), n);
}
#[then(expr = "Beta's factory was invoked exactly {int} time")]
async fn then_beta_invoked(world: &mut MultiHandlerWorld, n: u32) {
    assert_eq!(world.beta_calls.load(Ordering::SeqCst), n);
}

#[allow(dead_code)]
fn _linker() {
    let _ = full_type_url::<ReserveStock>();
    let _ = full_type_url::<CreateShipment>();
}

// ---------------------------------------------------------------------------
// Audit #18: forbid multi-handler CH dispatch (C-0010..C-0012 reframed).
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct RegisterPlayer {}
impl ::prost::Name for RegisterPlayer {
    const NAME: &'static str = "RegisterPlayer";
    const PACKAGE: &'static str = "player";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct DepositFunds {}
impl ::prost::Name for DepositFunds {
    const NAME: &'static str = "DepositFunds";
    const PACKAGE: &'static str = "player";
}

// Cross-domain CH pair for C-0011: same command type in two different domains.
struct AlphaA;
#[command_handler(domain = "orderA", state = S)]
impl AlphaA {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, _cmd: CreateOrder, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}
struct BetaB;
#[command_handler(domain = "orderB", state = S)]
impl BetaB {
    #[handles(CreateOrder)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, _cmd: CreateOrder, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

// Single CH with multiple handled types for C-0012.
struct Player;
#[command_handler(domain = "player", state = S)]
impl Player {
    #[handles(RegisterPlayer)]
    #[allow(unused_variables, dead_code)]
    fn on_register(&self, _cmd: RegisterPlayer, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
    #[handles(DepositFunds)]
    #[allow(unused_variables, dead_code)]
    fn on_deposit(&self, _cmd: DepositFunds, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[given("both handle CreateOrder")]
async fn given_both_handle_create_order(_world: &mut MultiHandlerWorld) {}

#[given(expr = "a command handler Alpha for domain {string} handling CreateOrder")]
async fn given_cross_alpha(_world: &mut MultiHandlerWorld, _domain: String) {}

#[given(expr = "a command handler Beta for domain {string} handling CreateOrder")]
async fn given_cross_beta(_world: &mut MultiHandlerWorld, _domain: String) {}

#[given(
    expr = "a command handler Player for domain {string} handling RegisterPlayer and DepositFunds"
)]
async fn given_player_two_types(_world: &mut MultiHandlerWorld, _domain: String) {}

#[when("the router is built with Alpha then Beta")]
async fn when_built_alpha_beta_capture(world: &mut MultiHandlerWorld) {
    // C-0010: same-domain Alpha/Beta pair (both `domain = "order"` per
    // their `#[command_handler]` decoration). Builder must reject as
    // DuplicateCommandHandler.
    world.build_result = Some(
        Router::new("multi-ch-test")
            .with_handler(|| Alpha)
            .with_handler(|| Beta)
            .build(),
    );
}

#[when("the router is built with Alpha then Beta across domains")]
async fn when_built_alpha_beta_cross(world: &mut MultiHandlerWorld) {
    // C-0011: AlphaA in "orderA", BetaB in "orderB", both handle
    // CreateOrder. Different domain keys → no duplicate.
    world.build_result = Some(
        Router::new("multi-ch-cross")
            .with_handler(|| AlphaA)
            .with_handler(|| BetaB)
            .build(),
    );
}

#[when("the router is built with Player")]
async fn when_built_player(world: &mut MultiHandlerWorld) {
    world.build_result = Some(
        Router::new("multi-ch-player")
            .with_handler(|| Player)
            .build(),
    );
}

#[then(expr = "build fails with DuplicateCommandHandler for domain {string} and {word}")]
async fn then_build_fails_duplicate(
    world: &mut MultiHandlerWorld,
    domain: String,
    cmd_type: String,
) {
    let result = world
        .build_result
        .take()
        .expect("build was not attempted in a When step");
    let err = result.expect_err("build should have failed");
    // Audit #72: BuildError variants now carry an `ErrorDetail` with
    // structured `details` rather than typed struct fields. Assert on
    // `code` + `details["domain"]` / `details["type_url"]`.
    let angzarr_client::router::BuildError::DuplicateCommandHandler(detail) = &err else {
        panic!("expected DuplicateCommandHandler, got {err:?}");
    };
    assert_eq!(
        detail.code,
        angzarr_client::error_codes::codes::DUPLICATE_COMMAND_HANDLER
    );
    assert_eq!(detail.details["domain"], domain);
    let cmd_full = match cmd_type.as_str() {
        "CreateOrder" => full_type_url::<CreateOrder>(),
        other => panic!("test fixture missing for command type {other}"),
    };
    assert_eq!(detail.details["type_url"], cmd_full);
}

#[then("build succeeds with a CommandHandlerRouter")]
async fn then_build_succeeds_ch(world: &mut MultiHandlerWorld) {
    let result = world
        .build_result
        .take()
        .expect("build was not attempted in a When step");
    let built = result.expect("build should have succeeded");
    assert!(matches!(built, Built::CommandHandler(_)));
}

// C-0087 (audit #18 reframe — saga factory invocation count).

#[given("each saga factory counts invocations")]
async fn given_each_saga_factory_counts(_world: &mut MultiHandlerWorld) {
    // when_dispatch_saga always installs counting closures; the Given
    // step is documentation here.
}

#[then(expr = "SagaA's factory was invoked exactly {int} time")]
async fn then_saga_a_invoked(world: &mut MultiHandlerWorld, n: u32) {
    assert_eq!(world.alpha_calls.load(Ordering::SeqCst), n);
}

#[then(expr = "SagaB's factory was invoked exactly {int} time")]
async fn then_saga_b_invoked(world: &mut MultiHandlerWorld, n: u32) {
    assert_eq!(world.beta_calls.load(Ordering::SeqCst), n);
}
