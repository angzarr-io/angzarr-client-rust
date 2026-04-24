//! Core `Handler` trait and supporting types for the unified router.
//!
//! Handlers are produced by proc-macro expansion in `angzarr-macros`. End users
//! do not implement `Handler` by hand; they apply `#[command_handler]` / `#[saga]` /
//! `#[process_manager]` / `#[projector]` on an inherent impl, and the macro
//! emits `impl Handler for T`.

use crate::proto::{
    BusinessResponse, ContextualCommand, EventBook, ProcessManagerHandleRequest,
    ProcessManagerHandleResponse, Projection, SagaHandleRequest, SagaResponse,
};
use crate::ClientError;

/// The four handler kinds the unified router understands.
///
/// Stored in [`HandlerConfig`] for mode inference at build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    CommandHandler,
    Saga,
    ProcessManager,
    Projector,
}

/// Metadata describing a handler produced by a kind macro.
///
/// Populated by proc-macro expansion. The builder inspects this to infer the
/// target runtime router type and validate configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerConfig {
    CommandHandler {
        domain: String,
        /// Proto type URLs accepted by this handler's `#[handles]` methods.
        handled: Vec<String>,
        /// `(domain, command_name)` keys covered by `#[rejected]` methods.
        rejected: Vec<(String, String)>,
        /// Proto type URLs for events declared in `#[applies]` methods.
        applies: Vec<String>,
        /// Method name of the `#[state_factory]` if one was declared.
        /// `None` → runtime rebuild uses `Default::default()`.
        state_factory: Option<String>,
    },
    Saga {
        name: String,
        source: String,
        target: String,
        handled: Vec<String>,
        rejected: Vec<(String, String)>,
    },
    ProcessManager {
        name: String,
        pm_domain: String,
        sources: Vec<String>,
        targets: Vec<String>,
        handled: Vec<String>,
        rejected: Vec<(String, String)>,
        applies: Vec<String>,
        state_factory: Option<String>,
    },
    Projector {
        name: String,
        domains: Vec<String>,
        handled: Vec<String>,
    },
}

impl HandlerConfig {
    /// Which kind this config represents.
    pub fn kind(&self) -> Kind {
        match self {
            Self::CommandHandler { .. } => Kind::CommandHandler,
            Self::Saga { .. } => Kind::Saga,
            Self::ProcessManager { .. } => Kind::ProcessManager,
            Self::Projector { .. } => Kind::Projector,
        }
    }
}

/// Per-dispatch input for a handler.
///
/// One variant per kind; carries the transport-level request.
#[derive(Debug, Clone)]
pub enum HandlerRequest {
    CommandHandler(ContextualCommand),
    Saga(SagaHandleRequest),
    ProcessManager(ProcessManagerHandleRequest),
    Projector(EventBook),
}

/// Per-dispatch output from a handler.
///
/// One variant per kind, wrapping the proto response type that the
/// corresponding gRPC service expects.
#[derive(Debug, Clone)]
pub enum HandlerResponse {
    CommandHandler(BusinessResponse),
    Saga(SagaResponse),
    ProcessManager(ProcessManagerHandleResponse),
    Projector(Projection),
}

/// Errors raised by `Router::build()` or runtime router construction.
///
/// Variants are additive — expect more fields as later rounds add invariants.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    #[error("router has no handlers")]
    Empty,
    #[error("cannot mix handler kinds in one router: found {0:?} and {1:?}")]
    MixedKinds(Kind, Kind),
    #[error("router built as {expected:?} but requested as {requested:?}")]
    WrongKind { expected: Kind, requested: Kind },
}

/// Error raised by runtime dispatch (handler routing, request translation).
///
/// Separate from [`ClientError`](crate::ClientError) so dispatch-layer
/// callers can distinguish routing failures from transport/grpc errors
/// without matching on a broad enum. A `From<DispatchError> for ClientError`
/// impl preserves single-type propagation where desired.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("dispatch error ({code:?}): {details}")]
pub struct DispatchError {
    pub code: tonic::Code,
    pub details: String,
}

impl DispatchError {
    pub fn new(code: tonic::Code, details: impl Into<String>) -> Self {
        Self {
            code,
            details: details.into(),
        }
    }
}

impl From<DispatchError> for crate::error::ClientError {
    fn from(err: DispatchError) -> Self {
        crate::error::ClientError::from(tonic::Status::new(err.code, err.details))
    }
}

/// Typed output of [`Router::build`][crate::router::Router::build].
///
/// One variant per handler kind. Obtain the concrete runtime router via the
/// matching `into_*()` accessor or a `match` on the returned value.
#[derive(Debug)]
pub enum Built {
    CommandHandler(crate::router::runtime::CommandHandlerRouter),
    Saga(crate::router::runtime::SagaRouter),
    ProcessManager(crate::router::runtime::ProcessManagerRouter),
    Projector(crate::router::runtime::ProjectorRouter),
}

/// Minimal contract every handler implements.
///
/// User code never implements `Handler` directly; macros emit the impl.
pub trait Handler: Send + Sync {
    /// Describe this handler's kind and registered behaviors.
    fn config(&self) -> HandlerConfig;

    /// Execute a dispatch request against this handler.
    ///
    /// R1 stub — real implementations land in R6 (command), R11 (saga),
    /// R12 (pm), R13 (projector).
    fn dispatch(&self, request: HandlerRequest) -> Result<HandlerResponse, ClientError>;
}

/// Compile-time kind marker.
///
/// Lives on a separate trait from [`Handler`] so that `Handler` stays
/// object-safe (`Box<dyn Handler>`) — associated constants on the main
/// trait would bar it from trait-object use.
///
/// The `with_handler::<H, F>` method captures `H::KIND` at registration
/// without invoking the factory, enabling mode inference at build time.
pub trait HandlerKind {
    const KIND: Kind;
}
