//! `Router::build()` mode-inference + structural error coverage.
//!
//! Mirrors the build-time invariants exercised by Python's
//! `tests/router/test_mode_inference.py`:
//!
//! * empty router → `BuildError::Empty`
//! * mixed kinds → `BuildError::MixedKinds` (carries both kinds in `details`)
//! * single-kind register → returns the matching `Built::*` variant
//! * `handler_count()` reports the registered factory count
//!
//! Uses minimal macro-declared handlers, one per kind.

use angzarr_client::proto::{EventBook, ProcessManagerHandleResponse, SagaResponse};
#[allow(unused_imports)]
use angzarr_client::router::{command_handler, handles, process_manager, projector, saga};
use angzarr_client::router::{BuildError, Built, Router};
#[allow(unused_imports)]
use angzarr_client::CommandResult;

// --- Minimal handlers, one per kind. -------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct Ping {}
impl ::prost::Name for Ping {
    const NAME: &'static str = "Ping";
    const PACKAGE: &'static str = "mode";
}

#[derive(Default)]
struct NoState;

struct StubCh;
#[command_handler(domain = "stub", state = NoState)]
impl StubCh {
    #[handles(Ping)]
    fn on_ping(&self, _cmd: Ping, _state: &NoState, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

struct StubSaga;
#[saga(name = "stub-saga", source = "src", target = "tgt")]
impl StubSaga {
    #[handles(Ping)]
    fn on_ping(&self, _evt: Ping) -> CommandResult<SagaResponse> {
        Ok(SagaResponse::default())
    }
}

struct StubPm;
#[process_manager(name = "stub-pm", pm_domain = "pm", state = NoState, sources = ["a"], targets = ["b"])]
impl StubPm {
    #[handles(Ping)]
    fn on_ping(&self, _evt: Ping, _state: &NoState) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
}

struct StubProjector;
#[projector(name = "stub-proj", domains = ["d"])]
impl StubProjector {
    #[handles(Ping)]
    fn on_ping(&self, _evt: Ping) -> CommandResult<()> {
        Ok(())
    }
}

// --- Build outcome assertions ------------------------------------------

#[test]
fn empty_router_yields_build_error_empty() {
    let err = Router::new("empty")
        .build()
        .expect_err("empty router rejected");
    assert!(matches!(err, BuildError::Empty(_)), "got {:?}", err);
}

#[test]
fn single_command_handler_returns_command_handler_built() {
    let built = Router::new("single-ch")
        .with_handler(|| StubCh)
        .build()
        .expect("build should succeed");
    assert!(matches!(built, Built::CommandHandler(_)), "got {:?}", built);
}

#[test]
fn single_saga_returns_saga_built() {
    let built = Router::new("single-saga")
        .with_handler(|| StubSaga)
        .build()
        .expect("build should succeed");
    assert!(matches!(built, Built::Saga(_)), "got {:?}", built);
}

#[test]
fn single_process_manager_returns_process_manager_built() {
    let built = Router::new("single-pm")
        .with_handler(|| StubPm)
        .build()
        .expect("build should succeed");
    assert!(matches!(built, Built::ProcessManager(_)), "got {:?}", built);
}

#[test]
fn single_projector_returns_projector_built() {
    let built = Router::new("single-proj")
        .with_handler(|| StubProjector)
        .build()
        .expect("build should succeed");
    assert!(matches!(built, Built::Projector(_)), "got {:?}", built);
}

#[test]
fn mixing_command_handler_and_saga_yields_mixed_kinds() {
    let err = Router::new("mixed-ch-saga")
        .with_handler(|| StubCh)
        .with_handler(|| StubSaga)
        .build()
        .expect_err("mixed kinds rejected");
    let detail = match err {
        BuildError::MixedKinds(d) => d,
        other => panic!("expected MixedKinds, got {:?}", other),
    };
    let blob = format!("{:?}", detail);
    assert!(blob.contains("CommandHandler"), "details: {}", blob);
    assert!(blob.contains("Saga"), "details: {}", blob);
}

#[test]
fn mixing_saga_and_process_manager_yields_mixed_kinds() {
    let err = Router::new("mixed-saga-pm")
        .with_handler(|| StubSaga)
        .with_handler(|| StubPm)
        .build()
        .expect_err("mixed kinds rejected");
    assert!(matches!(err, BuildError::MixedKinds(_)));
}

#[test]
fn mixing_projector_and_command_handler_yields_mixed_kinds() {
    let err = Router::new("mixed-proj-ch")
        .with_handler(|| StubProjector)
        .with_handler(|| StubCh)
        .build()
        .expect_err("mixed kinds rejected");
    assert!(matches!(err, BuildError::MixedKinds(_)));
}

#[test]
fn handler_count_reports_registered_factories() {
    let r = Router::new("count")
        .with_handler(|| StubProjector)
        .with_handler(|| StubProjector);
    assert_eq!(r.handler_count(), 2);
}

#[test]
fn router_name_is_stored_verbatim() {
    let r = Router::new("my-router-name");
    assert_eq!(r.name(), "my-router-name");
}
