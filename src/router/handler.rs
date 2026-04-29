//! Core `Handler` trait and supporting types for the unified router.
//!
//! Handlers are produced by proc-macro expansion in `angzarr-macros`. End users
//! do not implement `Handler` by hand; they apply `#[command_handler]` / `#[saga]` /
//! `#[process_manager]` / `#[projector]` on an inherent impl, and the macro
//! emits `impl Handler for T`.

use crate::proto::{
    BusinessResponse, ContextualCommand, EventBook, FactRequest, ProcessManagerHandleRequest,
    ProcessManagerHandleResponse, Projection, ReplayRequest, ReplayResponse, SagaHandleRequest,
    SagaResponse, UpcastRequest, UpcastResponse,
};
use crate::ClientError;

/// The five handler kinds the unified router understands.
///
/// Stored in [`HandlerConfig`] for mode inference at build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    CommandHandler,
    Saga,
    ProcessManager,
    Projector,
    Upcaster,
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
        /// Audit #45: proto type URLs for fact events declared in
        /// `#[handles_fact]` methods. Empty → aggregate did not opt
        /// into the `HandleFact` RPC; the gRPC adapter returns
        /// UNIMPLEMENTED based on this metadata.
        handles_fact: Vec<String>,
        /// Audit #45: aggregate opted into the `Replay` RPC via
        /// `#[command_handler(supports_replay = true)]`. False → gRPC
        /// adapter returns UNIMPLEMENTED for `Replay`; the coordinator
        /// degrades MERGE_COMMUTATIVE to MERGE_STRICT.
        supports_replay: bool,
    },
    Saga {
        name: String,
        source: String,
        target: String,
        /// Audit #74: whether commands emitted to ``target`` ever use
        /// sync mode (SIMPLE / CASCADE / DECISION / ISOLATED). Drives
        /// readiness probing — only sync targets need their coordinator
        /// reachable for traffic to be safe.
        sync: bool,
        handled: Vec<String>,
        rejected: Vec<(String, String)>,
    },
    ProcessManager {
        name: String,
        pm_domain: String,
        sources: Vec<String>,
        targets: Vec<String>,
        /// Audit #74: subset of ``targets`` whose commands ever use sync
        /// mode. Drives readiness probing.
        sync_targets: Vec<String>,
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
    Upcaster {
        name: String,
        domain: String,
        /// `(from_type_url, to_type_url)` pairs — one per `#[upcasts]` method.
        upcasts: Vec<(String, String)>,
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
            Self::Upcaster { .. } => Kind::Upcaster,
        }
    }
}

/// Per-dispatch input for a handler.
///
/// One variant per kind; carries the transport-level request.
#[derive(Debug, Clone)]
pub enum HandlerRequest {
    CommandHandler(ContextualCommand),
    /// Audit #45: fact-event dispatch from the coordinator's
    /// `HandleFact` RPC. The aggregate opts in via `#[handles_fact]`.
    HandleFact(FactRequest),
    /// Audit #45: state replay for `MERGE_COMMUTATIVE` conflict
    /// detection. The aggregate opts in via
    /// `#[command_handler(supports_replay = true)]`.
    Replay(ReplayRequest),
    Saga(SagaHandleRequest),
    ProcessManager(ProcessManagerHandleRequest),
    Projector(EventBook),
    Upcaster(UpcastRequest),
}

/// Per-dispatch output from a handler.
///
/// One variant per kind, wrapping the proto response type that the
/// corresponding gRPC service expects.
#[derive(Debug, Clone)]
pub enum HandlerResponse {
    CommandHandler(BusinessResponse),
    /// Audit #45: events emitted by `#[handles_fact]` methods, to be
    /// persisted on the aggregate.
    HandleFact(EventBook),
    /// Audit #45: resulting state after replay, packed into `Any`.
    Replay(ReplayResponse),
    Saga(SagaResponse),
    ProcessManager(ProcessManagerHandleResponse),
    Projector(Projection),
    Upcaster(UpcastResponse),
}

/// Errors raised by `Router::build()` or runtime router construction.
///
/// Variants are additive — expect more fields as later rounds add invariants.
///
/// Audit #72: each variant carries a [`crate::error::ErrorDetail`] following
/// the structural error model from audit #59 (`code: &'static str`,
/// static `message: &'static str`, `details: BTreeMap<String, String>`).
/// `Display` emits the static message verbatim; runtime context (router
/// name, conflicting kinds, duplicate (domain, type_url), etc.) rides in
/// `details` for cucumber assertions on `err.code` / `err.details["..."]`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    /// `Router::build()` called with zero registered handlers.
    /// `code = ROUTER_NO_HANDLERS`, `details["router_name"]`.
    #[error("{}", .0.message)]
    Empty(crate::error::ErrorDetail),

    /// `Router::build()` called with handlers of different kinds (e.g.
    /// a `command_handler` and a `saga` registered together).
    /// `code = MIXED_HANDLER_KINDS`,
    /// `details["handler_kind"]` (first kind), `details["other_kind"]`
    /// (conflicting kind), `details["router_name"]`.
    #[error("{}", .0.message)]
    MixedKinds(crate::error::ErrorDetail),

    /// Audit finding #51 (reframed as #18): two CommandHandlers register
    /// for the same `(domain, command_type_url)` pair within one Router.
    /// Multi-handler CH dispatch is forbidden — each `(domain, type)` key
    /// admits exactly one handler. Saga / PM / projector / upcaster fan-
    /// out is still allowed (those kinds legitimately broadcast).
    /// `code = DUPLICATE_COMMAND_HANDLER`,
    /// `details["domain"]`, `details["type_url"]`, `details["router_name"]`.
    #[error("{}", .0.message)]
    DuplicateCommandHandler(crate::error::ErrorDetail),
}

impl BuildError {
    /// SCREAMING_SNAKE error code. Use for cucumber-style assertions
    /// rather than message-substring matching.
    pub fn code(&self) -> &'static str {
        match self {
            BuildError::Empty(d) => d.code,
            BuildError::MixedKinds(d) => d.code,
            BuildError::DuplicateCommandHandler(d) => d.code,
        }
    }

    /// Static message (same string across languages for the same predicate).
    pub fn message(&self) -> &'static str {
        match self {
            BuildError::Empty(d) => d.message,
            BuildError::MixedKinds(d) => d.message,
            BuildError::DuplicateCommandHandler(d) => d.message,
        }
    }

    /// Structured runtime context (router name, conflicting kinds, etc.).
    pub fn details(&self) -> &std::collections::BTreeMap<String, String> {
        match self {
            BuildError::Empty(d) => &d.details,
            BuildError::MixedKinds(d) => &d.details,
            BuildError::DuplicateCommandHandler(d) => &d.details,
        }
    }
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
    Upcaster(crate::router::upcaster::UpcasterRouter),
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
