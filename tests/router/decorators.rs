//! R1 + R2 — proc macro metadata tests.
//!
//! R1 covers kind + domain detection.
//! R2 covers method-level metadata — `#[handles]`, `#[applies]`,
//! `#[state_factory]` — recoverable through `Handler::config()`.

use angzarr_client::proto::EventBook;
use angzarr_client::router::command_handler;
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
        seq: u64,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }

    #[handles(DepositFunds)]
    #[allow(unused_variables, dead_code)]
    fn deposit(
        &self,
        cmd: DepositFunds,
        state: &PlayerState,
        seq: u64,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
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
                    "/test.RegisterPlayer".to_string(),
                    "/test.DepositFunds".to_string(),
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
                    "/test.PlayerRegistered".to_string(),
                    "/test.FundsDeposited".to_string(),
                ]
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
        seq: u64,
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
// Saga output metadata.
// ----------------------------------------------------------------------------

use angzarr_client::router::saga;
use angzarr_client::router::{Built, Router};

test_proto!(InventoryReserved);

struct AsyncSaga;

#[saga(name = "saga-async", source = "order", target = "inventory")]
impl AsyncSaga {
    #[handles(InventoryReserved)]
    #[allow(unused_variables, dead_code)]
    fn on(&self, evt: InventoryReserved) -> CommandResult<angzarr_client::proto::SagaResponse> {
        Ok(angzarr_client::proto::SagaResponse::default())
    }
}

#[test]
fn saga_router_output_domains_is_its_target() {
    match AsyncSaga.config() {
        HandlerConfig::Saga { target, .. } => assert_eq!(target, "inventory"),
        other => panic!("expected Saga, got {:?}", other),
    }
    let built = Router::new("saga-async")
        .with_handler(|| AsyncSaga)
        .build()
        .expect("build");
    let Built::Saga(r) = built else {
        panic!("expected SagaRouter")
    };
    assert_eq!(r.output_domains(), vec!["inventory".to_string()]);
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
