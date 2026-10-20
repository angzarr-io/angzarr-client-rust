//! `Router::build()` mode inference: the one component's kind picks the
//! `Built::*` variant. A router with no component, or a second one, does
//! not compile (`stacking::routers_and_hosts_take_exactly_one_component`).
//!
//! Uses minimal macro-declared handlers, one per kind.

use angzarr_client::proto::{EventBook, ProcessManagerHandleResponse, SagaResponse};
#[allow(unused_imports)]
use angzarr_client::router::{command_handler, handles, process_manager, projector, saga};
use angzarr_client::router::{Built, Router};
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
    fn on_ping(&self, _cmd: Ping, _state: &NoState, _seq: u64) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

struct StubSaga;
#[saga(name = "stub-saga", input_domain = "src", output_domain = "tgt")]
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
fn router_name_and_kind_are_kept() {
    let r = Router::new("my-router-name");
    assert_eq!(r.name(), "my-router-name");
    let r = r.with_handler(|| StubPm);
    assert_eq!(r.name(), "my-router-name");
    assert_eq!(r.kind(), angzarr_client::router::Kind::ProcessManager);
}
