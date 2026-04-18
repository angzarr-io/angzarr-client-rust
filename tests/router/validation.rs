//! R5 — build-time validation per kind.
//!
//! Each kind macro must reject impls missing their required attributes at
//! macro-parse time, and runtime `build()` must allow duplicate
//! `(domain, type_url)` across factories (the "call-both" multi-handler
//! design verified in later rounds).

use angzarr_client::aggregate;
use angzarr_client::proto::EventBook;
use angzarr_client::router::{Built, Router};
use angzarr_client::CommandResult;

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

test_proto!(SharedCmd);

#[derive(Default)]
struct SharedState;

struct AggA;
struct AggB;

#[aggregate(domain = "shared", state = SharedState)]
impl AggA {
    #[handles(SharedCmd)]
    #[allow(unused_variables, dead_code)]
    fn on_shared(
        &self,
        cmd: SharedCmd,
        state: &SharedState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[aggregate(domain = "shared", state = SharedState)]
impl AggB {
    #[handles(SharedCmd)]
    #[allow(unused_variables, dead_code)]
    fn on_shared(
        &self,
        cmd: SharedCmd,
        state: &SharedState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

#[test]
fn duplicate_domain_type_url_across_factories_is_allowed() {
    // Two aggregates claim the same (domain, type_url). Plan's "call-both"
    // design says this must build cleanly; dispatch fan-out is verified in R8.
    let built = Router::new("x")
        .with_handler(|| AggA)
        .with_handler(|| AggB)
        .build()
        .expect("duplicate (domain, type_url) keys must be allowed");
    match built {
        Built::CommandHandler(ch) => assert_eq!(ch.handler_count(), 2),
        other => panic!("expected CommandHandler, got {:?}", other),
    }
}

#[test]
fn aggregate_without_state_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/aggregate_without_state.rs");
}

#[test]
fn saga_without_target_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/saga_without_target.rs");
}

#[test]
fn process_manager_without_targets_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/process_manager_without_targets.rs");
}

#[test]
fn projector_without_domains_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/projector_without_domains.rs");
}
