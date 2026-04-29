//! R1 + R2 — proc macro metadata tests.
//!
//! R1 covers kind + domain detection.
//! R2 covers method-level metadata — `#[handles]`, `#[rejected]`, `#[applies]`,
//! `#[state_factory]` — recoverable through `Handler::config()`.

use angzarr_client::command_handler;
use angzarr_client::proto::EventBook;
use angzarr_client::router::{Handler, HandlerConfig, Kind};
use angzarr_client::CommandResult;

// Test-local proto stubs. These only need to implement `prost::Name` so the
// aggregate macro can call `<T as ::prost::Name>::type_url()` on them.
// We don't need real wire encoding — only the type-URL shape.
macro_rules! test_proto {
    ($name:ident) => {
        #[derive(Clone, PartialEq, ::prost::Message)]
        struct $name {}

        impl ::prost::Name for $name {
            const NAME: &'static str = stringify!($name);
            const PACKAGE: &'static str = "test";
        }
    };
}

test_proto!(RegisterPlayer);
test_proto!(DepositFunds);
test_proto!(PlayerRegistered);
test_proto!(FundsDeposited);

#[derive(Default)]
struct PlayerState {
    #[allow(dead_code)]
    exists: bool,
}

struct Player;

#[command_handler(domain = "player", state = PlayerState)]
impl Player {
    #[allow(dead_code)]
    fn new() -> Self {
        Self
    }

    #[state_factory]
    #[allow(dead_code)]
    fn empty() -> PlayerState {
        PlayerState::default()
    }

    #[applies(PlayerRegistered)]
    #[allow(unused_variables, dead_code)]
    fn apply_registered(state: &mut PlayerState, evt: PlayerRegistered) {}

    #[applies(FundsDeposited)]
    #[allow(unused_variables, dead_code)]
    fn apply_deposited(state: &mut PlayerState, evt: FundsDeposited) {}

    #[handles(RegisterPlayer)]
    #[allow(unused_variables, dead_code)]
    fn register(
        &self,
        cmd: RegisterPlayer,
        state: &PlayerState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }

    #[handles(DepositFunds)]
    #[allow(unused_variables, dead_code)]
    fn deposit(
        &self,
        cmd: DepositFunds,
        state: &PlayerState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }

    #[rejected(domain = "payment", command = "ProcessPayment")]
    #[allow(unused_variables, dead_code)]
    fn on_payment_rejected(
        &self,
        notif: &angzarr_client::proto::Notification,
        state: &PlayerState,
    ) -> CommandResult<angzarr_client::proto::BusinessResponse> {
        Ok(angzarr_client::proto::BusinessResponse::default())
    }
}

// ----------------------------------------------------------------------------
// R1 — kind + domain
// ----------------------------------------------------------------------------

#[test]
fn aggregate_config_reports_command_handler_kind() {
    let cfg = Player.config();
    assert_eq!(cfg.kind(), Kind::CommandHandler);
}

#[test]
fn aggregate_config_carries_declared_domain() {
    match Player.config() {
        HandlerConfig::CommandHandler { domain, .. } => {
            assert_eq!(domain, "player");
        }
        other => panic!("expected CommandHandler, got {:?}", other),
    }
}

// ----------------------------------------------------------------------------
// R2 — method-level metadata
// ----------------------------------------------------------------------------

#[test]
fn handles_stashes_type_urls_in_declaration_order() {
    match Player.config() {
        HandlerConfig::CommandHandler { handled, .. } => {
            assert_eq!(
                handled,
                vec![
                    "type.googleapis.com/test.RegisterPlayer".to_string(),
                    "type.googleapis.com/test.DepositFunds".to_string(),
                ]
            );
        }
        other => panic!("expected CommandHandler, got {:?}", other),
    }
}

#[test]
fn applies_stashes_event_type_urls_in_declaration_order() {
    match Player.config() {
        HandlerConfig::CommandHandler { applies, .. } => {
            assert_eq!(
                applies,
                vec![
                    "type.googleapis.com/test.PlayerRegistered".to_string(),
                    "type.googleapis.com/test.FundsDeposited".to_string(),
                ]
            );
        }
        other => panic!("expected CommandHandler, got {:?}", other),
    }
}

#[test]
fn rejected_stashes_domain_and_command_pairs() {
    match Player.config() {
        HandlerConfig::CommandHandler { rejected, .. } => {
            assert_eq!(
                rejected,
                vec![("payment".to_string(), "ProcessPayment".to_string())]
            );
        }
        other => panic!("expected CommandHandler, got {:?}", other),
    }
}

#[test]
fn state_factory_records_method_name() {
    match Player.config() {
        HandlerConfig::CommandHandler { state_factory, .. } => {
            assert_eq!(state_factory, Some("empty".to_string()));
        }
        other => panic!("expected CommandHandler, got {:?}", other),
    }
}

/// A second aggregate without `#[state_factory]` should report `None`,
/// signalling to the runtime that `Default::default()` is used.
struct PlayerNoFactory;

#[command_handler(domain = "player_bare", state = PlayerState)]
impl PlayerNoFactory {
    #[handles(RegisterPlayer)]
    #[allow(unused_variables, dead_code)]
    fn register(
        &self,
        cmd: RegisterPlayer,
        state: &PlayerState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[test]
fn state_factory_is_none_when_absent() {
    match PlayerNoFactory.config() {
        HandlerConfig::CommandHandler { state_factory, .. } => {
            assert_eq!(state_factory, None);
        }
        other => panic!("expected CommandHandler, got {:?}", other),
    }
}

// ----------------------------------------------------------------------------
// Audit #74: readiness sync-target metadata.
// ----------------------------------------------------------------------------

use angzarr_client::router::{Built, Router};
use angzarr_client::{process_manager, saga};

test_proto!(InventoryReserved);
test_proto!(SecondEvt);

struct AsyncSaga;

#[saga(name = "saga-async", source = "order", target = "inventory")]
impl AsyncSaga {
    #[handles(InventoryReserved)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, evt: InventoryReserved) -> CommandResult<angzarr_client::proto::SagaResponse> {
        Ok(angzarr_client::proto::SagaResponse::default())
    }
}

struct SyncSaga;

#[saga(name = "saga-sync", source = "order", target = "inventory", sync = true)]
impl SyncSaga {
    #[handles(InventoryReserved)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, evt: InventoryReserved) -> CommandResult<angzarr_client::proto::SagaResponse> {
        Ok(angzarr_client::proto::SagaResponse::default())
    }
}

#[test]
fn saga_sync_defaults_to_false() {
    match AsyncSaga.config() {
        HandlerConfig::Saga { sync, target, .. } => {
            assert_eq!(target, "inventory");
            assert!(!sync, "sync must default to false");
        }
        other => panic!("expected Saga, got {:?}", other),
    }
}

#[test]
fn saga_sync_true_carries_through_config() {
    match SyncSaga.config() {
        HandlerConfig::Saga { sync, .. } => assert!(sync),
        other => panic!("expected Saga, got {:?}", other),
    }
}

#[test]
fn saga_router_sync_output_domains_empty_for_async() {
    let built = Router::new("saga-async")
        .with_handler(|| AsyncSaga)
        .build()
        .expect("build");
    let Built::Saga(r) = built else {
        panic!("expected SagaRouter")
    };
    assert_eq!(r.output_domains(), vec!["inventory".to_string()]);
    assert!(r.sync_output_domains().is_empty());
    assert!(r.has_async_outputs());
}

#[test]
fn saga_router_sync_output_domains_includes_sync_target() {
    let built = Router::new("saga-sync")
        .with_handler(|| SyncSaga)
        .build()
        .expect("build");
    let Built::Saga(r) = built else {
        panic!("expected SagaRouter")
    };
    assert_eq!(r.sync_output_domains(), vec!["inventory".to_string()]);
    assert!(!r.has_async_outputs());
}

struct SyncToInv;

#[saga(name = "saga-sync-inv", source = "order", target = "inventory", sync = true)]
impl SyncToInv {
    #[handles(InventoryReserved)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, evt: InventoryReserved) -> CommandResult<angzarr_client::proto::SagaResponse> {
        Ok(angzarr_client::proto::SagaResponse::default())
    }
}

struct AsyncToInv;

#[saga(name = "saga-async-inv", source = "order", target = "inventory")]
impl AsyncToInv {
    #[handles(SecondEvt)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, evt: SecondEvt) -> CommandResult<angzarr_client::proto::SagaResponse> {
        Ok(angzarr_client::proto::SagaResponse::default())
    }
}

#[test]
fn saga_router_has_async_outputs_per_handler_check() {
    // Two sagas share `target = "inventory"`, one sync and one async.
    // Naive set-difference over output_domains would call this all-sync;
    // the per-handler check correctly reports has_async = true.
    let built = Router::new("saga-mixed")
        .with_handler(|| SyncToInv)
        .with_handler(|| AsyncToInv)
        .build()
        .expect("build");
    let Built::Saga(r) = built else {
        panic!("expected SagaRouter")
    };
    assert_eq!(r.sync_output_domains(), vec!["inventory".to_string()]);
    assert_eq!(r.output_domains(), vec!["inventory".to_string()]);
    assert!(r.has_async_outputs());
}

#[derive(Default)]
struct PmState {}

struct PmAllAsync;

#[process_manager(
    name = "pm-async",
    pm_domain = "fulfillment",
    sources = ["order"],
    targets = ["inventory", "shipping"],
    state = PmState
)]
impl PmAllAsync {
    #[handles(InventoryReserved)]
    #[allow(unused_variables, dead_code)]
    fn on(
        &self,
        evt: InventoryReserved,
        state: &PmState,
    ) -> CommandResult<angzarr_client::proto::ProcessManagerHandleResponse> {
        Ok(angzarr_client::proto::ProcessManagerHandleResponse::default())
    }
}

struct PmMixed;

#[process_manager(
    name = "pm-mixed",
    pm_domain = "fulfillment",
    sources = ["order"],
    targets = ["inventory", "shipping"],
    sync_targets = ["inventory"],
    state = PmState
)]
impl PmMixed {
    #[handles(InventoryReserved)]
    #[allow(unused_variables, dead_code)]
    fn on(
        &self,
        evt: InventoryReserved,
        state: &PmState,
    ) -> CommandResult<angzarr_client::proto::ProcessManagerHandleResponse> {
        Ok(angzarr_client::proto::ProcessManagerHandleResponse::default())
    }
}

#[test]
fn pm_sync_targets_default_empty() {
    match PmAllAsync.config() {
        HandlerConfig::ProcessManager { sync_targets, .. } => {
            assert!(sync_targets.is_empty());
        }
        other => panic!("expected ProcessManager, got {:?}", other),
    }
}

#[test]
fn pm_sync_targets_subset_carries_through_config() {
    match PmMixed.config() {
        HandlerConfig::ProcessManager {
            targets,
            sync_targets,
            ..
        } => {
            assert_eq!(targets, vec!["inventory", "shipping"]);
            assert_eq!(sync_targets, vec!["inventory".to_string()]);
        }
        other => panic!("expected ProcessManager, got {:?}", other),
    }
}

#[test]
fn pm_router_sync_outputs_empty_when_no_sync_targets() {
    let built = Router::new("pm-async")
        .with_handler(|| PmAllAsync)
        .build()
        .expect("build");
    let Built::ProcessManager(r) = built else {
        panic!("expected ProcessManagerRouter")
    };
    assert!(r.sync_output_domains().is_empty());
    assert!(r.has_async_outputs());
}

#[test]
fn pm_router_sync_outputs_returns_sync_subset() {
    let built = Router::new("pm-mixed")
        .with_handler(|| PmMixed)
        .build()
        .expect("build");
    let Built::ProcessManager(r) = built else {
        panic!("expected ProcessManagerRouter")
    };
    assert_eq!(r.sync_output_domains(), vec!["inventory".to_string()]);
    assert!(r.has_async_outputs());
}

// ----------------------------------------------------------------------------
// Audit #42: cached_name on each runtime router.
// ----------------------------------------------------------------------------

#[test]
fn saga_router_name_caches_across_calls() {
    let built = Router::new("saga-async")
        .with_handler(|| AsyncSaga)
        .build()
        .expect("build");
    let Built::Saga(r) = built else {
        panic!("expected SagaRouter")
    };
    let first = r.name();
    let second = r.name();
    let third = r.name();
    assert_eq!(first, "saga-async");
    assert_eq!(first, second);
    assert_eq!(second, third);
}
