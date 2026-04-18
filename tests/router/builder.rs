//! R3 — Router builder collects factories.
//!
//! Verifies:
//! - Empty router fails to build
//! - `with_handler` stores factories type-erased
//! - Factories are not invoked at registration or build time

use angzarr_client::aggregate;
use angzarr_client::proto::EventBook;
use angzarr_client::router::{
    BuildError, Built, Handler, HandlerConfig, HandlerKind, HandlerRequest, HandlerResponse, Kind,
    Router,
};
use angzarr_client::{ClientError, CommandResult};

// Test-local proto stubs.
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

test_proto!(Ping);

#[derive(Default)]
struct State;

struct Agg;

#[aggregate(domain = "d", state = State)]
impl Agg {
    #[handles(Ping)]
    #[allow(unused_variables, dead_code)]
    fn on_ping(&self, cmd: Ping, state: &State, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[test]
fn empty_router_fails_to_build_with_empty_error() {
    let result = Router::new("x").build();
    assert!(
        matches!(result, Err(BuildError::Empty)),
        "expected Err(BuildError::Empty), got {:?}",
        result.as_ref().err()
    );
}

#[test]
fn with_handler_stores_one_factory() {
    let router = Router::new("x").with_handler(|| Agg);
    assert_eq!(router.handler_count(), 1);
}

#[test]
fn with_handler_chained_stores_both() {
    let router = Router::new("x")
        .with_handler(|| Agg)
        .with_handler(|| Agg);
    assert_eq!(router.handler_count(), 2);
}

#[test]
fn factory_not_invoked_at_registration() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let calls = Arc::new(AtomicU32::new(0));
    let calls_closure = Arc::clone(&calls);
    let _router = Router::new("x").with_handler(move || {
        calls_closure.fetch_add(1, Ordering::SeqCst);
        Agg
    });
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "factory must not be invoked at registration"
    );
}

#[test]
fn single_aggregate_builds_into_command_handler_variant() {
    let built = Router::new("x").with_handler(|| Agg).build().unwrap();
    assert!(
        matches!(built, Built::CommandHandler(_)),
        "expected Built::CommandHandler, got {:?}",
        built
    );
}

// Hand-rolled saga-kind handler used only to exercise cross-kind mixing;
// the saga proc-macro is not yet Tier-5-ified (lands in R11).
struct FakeSaga;
impl HandlerKind for FakeSaga {
    const KIND: Kind = Kind::Saga;
}
impl Handler for FakeSaga {
    fn config(&self) -> HandlerConfig {
        HandlerConfig::Saga {
            name: "fake".into(),
            source: "s".into(),
            target: "t".into(),
            handled: vec![],
            rejected: vec![],
        }
    }
    fn dispatch(&self, _: HandlerRequest) -> Result<HandlerResponse, ClientError> {
        unimplemented!()
    }
}

#[test]
fn mixed_kinds_fails_with_mixed_kinds_error() {
    let result = Router::new("x")
        .with_handler(|| Agg)
        .with_handler(|| FakeSaga)
        .build();
    assert_eq!(
        result.err(),
        Some(BuildError::MixedKinds(Kind::CommandHandler, Kind::Saga))
    );
}

#[test]
fn single_saga_builds_into_saga_variant() {
    let built = Router::new("x").with_handler(|| FakeSaga).build().unwrap();
    assert!(
        matches!(built, Built::Saga(_)),
        "expected Built::Saga, got {:?}",
        built
    );
}

#[test]
fn homogeneous_multi_factory_preserves_all_factories_in_one_variant() {
    let built = Router::new("x")
        .with_handler(|| Agg)
        .with_handler(|| Agg)
        .with_handler(|| Agg)
        .build()
        .unwrap();
    match built {
        Built::CommandHandler(ch) => assert_eq!(ch.handler_count(), 3),
        other => panic!("expected CommandHandler, got {:?}", other),
    }
}

#[test]
fn factory_not_invoked_at_build() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let calls = Arc::new(AtomicU32::new(0));
    let calls_closure = Arc::clone(&calls);
    let _built = Router::new("x")
        .with_handler(move || {
            calls_closure.fetch_add(1, Ordering::SeqCst);
            Agg
        })
        .build();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "factory must not be invoked at build"
    );
}
