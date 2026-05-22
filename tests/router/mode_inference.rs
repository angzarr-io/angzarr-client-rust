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
//! Uses minimal hand-rolled `Handler` impls so we can probe the builder
//! without dragging in proc-macro scaffolding.

use angzarr_client::router::{
    BuildError, Built, Handler, HandlerConfig, HandlerKind, HandlerRequest, HandlerResponse, Kind,
    Router,
};
use angzarr_client::ClientError;

// --- Minimal hand-rolled handlers, one per kind. -------------------------

struct StubCh;
fn stub_ch_config() -> HandlerConfig {
    HandlerConfig::CommandHandler {
        domain: "stub".into(),
        handled: vec![],
        rejected: vec![],
        applies: vec![],
        state_factory: None,
        handles_fact: vec![],
        supports_replay: false,
    }
}
impl HandlerKind for StubCh {
    const KIND: Kind = Kind::CommandHandler;
    fn handler_config() -> HandlerConfig {
        stub_ch_config()
    }
}
impl Handler for StubCh {
    fn config(&self) -> HandlerConfig {
        stub_ch_config()
    }
    fn dispatch(&self, _request: HandlerRequest) -> Result<HandlerResponse, ClientError> {
        unreachable!("stub: build-time only")
    }
}

struct StubSaga;
fn stub_saga_config() -> HandlerConfig {
    HandlerConfig::Saga {
        name: "stub-saga".into(),
        source: "src".into(),
        target: "tgt".into(),
        sync: false,
        handled: vec![],
        rejected: vec![],
    }
}
impl HandlerKind for StubSaga {
    const KIND: Kind = Kind::Saga;
    fn handler_config() -> HandlerConfig {
        stub_saga_config()
    }
}
impl Handler for StubSaga {
    fn config(&self) -> HandlerConfig {
        stub_saga_config()
    }
    fn dispatch(&self, _request: HandlerRequest) -> Result<HandlerResponse, ClientError> {
        unreachable!("stub: build-time only")
    }
}

struct StubPm;
fn stub_pm_config() -> HandlerConfig {
    HandlerConfig::ProcessManager {
        name: "stub-pm".into(),
        pm_domain: "pm".into(),
        sources: vec!["a".into()],
        targets: vec!["b".into()],
        sync_targets: vec![],
        handled: vec![],
        rejected: vec![],
        applies: vec![],
        state_factory: None,
    }
}
impl HandlerKind for StubPm {
    const KIND: Kind = Kind::ProcessManager;
    fn handler_config() -> HandlerConfig {
        stub_pm_config()
    }
}
impl Handler for StubPm {
    fn config(&self) -> HandlerConfig {
        stub_pm_config()
    }
    fn dispatch(&self, _request: HandlerRequest) -> Result<HandlerResponse, ClientError> {
        unreachable!("stub: build-time only")
    }
}

struct StubProjector;
fn stub_projector_config() -> HandlerConfig {
    HandlerConfig::Projector {
        name: "stub-proj".into(),
        domains: vec!["d".into()],
        handled: vec![],
    }
}
impl HandlerKind for StubProjector {
    const KIND: Kind = Kind::Projector;
    fn handler_config() -> HandlerConfig {
        stub_projector_config()
    }
}
impl Handler for StubProjector {
    fn config(&self) -> HandlerConfig {
        stub_projector_config()
    }
    fn dispatch(&self, _request: HandlerRequest) -> Result<HandlerResponse, ClientError> {
        unreachable!("stub: build-time only")
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
