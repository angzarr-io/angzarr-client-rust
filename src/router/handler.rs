//! Handler metadata and the traits the kind macros implement.
//!
//! Users apply `#[command_handler]` / `#[saga]` / `#[process_manager]` /
//! `#[projector]` / `#[upcaster]` to an inherent impl. The macro emits
//! [`HandlerKind`] (static metadata plus the component's angzarr-router
//! dispatch table) and [`Handler`] (the instance view of the metadata).

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

impl Kind {
    /// Stable string representation suitable for wire-visible metadata
    /// (cucumber assertions, error details). Pinned via tests so a
    /// future rename of an enum variant cannot silently change the
    /// string the dispatch layer emits.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Kind::CommandHandler => "CommandHandler",
            Kind::Saga => "Saga",
            Kind::ProcessManager => "ProcessManager",
            Kind::Projector => "Projector",
            Kind::Upcaster => "Upcaster",
        }
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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
        /// `compensates` entries of the `#[rejected]` methods: the rejected
        /// command's fully-qualified type, optionally `"domain:"`-qualified.
        compensates: Vec<String>,
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
        /// `compensates` entries of the `#[rejected]` methods.
        compensates: Vec<String>,
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

    /// A component's own table is invalid, as reported by angzarr-router
    /// (e.g. `AMBIGUOUS_COMPENSATION`: one command type compensated both
    /// unqualified and domain-qualified). `code` is the router's code.
    #[error("{}", .0.message)]
    InvalidComponent(crate::error::ErrorDetail),
}

impl BuildError {
    /// SCREAMING_SNAKE error code. Use for cucumber-style assertions
    /// rather than message-substring matching.
    pub fn code(&self) -> &'static str {
        match self {
            BuildError::Empty(d) => d.code,
            BuildError::MixedKinds(d) => d.code,
            BuildError::DuplicateCommandHandler(d) => d.code,
            BuildError::InvalidComponent(d) => d.code,
        }
    }

    /// Static message (same string across languages for the same predicate).
    pub fn message(&self) -> &'static str {
        match self {
            BuildError::Empty(d) => d.message,
            BuildError::MixedKinds(d) => d.message,
            BuildError::DuplicateCommandHandler(d) => d.message,
            BuildError::InvalidComponent(d) => d.message,
        }
    }

    /// Structured runtime context (router name, conflicting kinds, etc.).
    pub fn details(&self) -> &std::collections::BTreeMap<String, String> {
        match self {
            BuildError::Empty(d) => &d.details,
            BuildError::MixedKinds(d) => &d.details,
            BuildError::DuplicateCommandHandler(d) => &d.details,
            BuildError::InvalidComponent(d) => &d.details,
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
/// One variant per handler kind; `match` on it to obtain the concrete
/// runtime router, or hand it to [`crate::run_server`].
#[derive(Debug)]
pub enum Built {
    CommandHandler(crate::router::routers::CommandHandlerRouter),
    Saga(crate::router::routers::SagaRouter),
    ProcessManager(crate::router::routers::ProcessManagerRouter),
    Projector(crate::router::routers::ProjectorRouter),
    Upcaster(crate::router::routers::UpcasterRouter),
}

/// The instance view of a handler's metadata.
///
/// User code never implements `Handler` directly; the kind macros emit it.
pub trait Handler: Send + Sync {
    /// Describe this handler's kind and registered behaviors.
    fn config(&self) -> HandlerConfig;
}

#[cfg(test)]
mod kind_str_tests {
    use super::Kind;

    /// Pin the wire-visible string for each Kind so a future rename of
    /// an enum variant (CommandHandler → Aggregate, say) cannot
    /// silently change the string the dispatch layer emits in error
    /// details / cucumber assertions / metadata trailers.
    #[test]
    fn kind_as_str_pins_wire_strings() {
        assert_eq!(Kind::CommandHandler.as_str(), "CommandHandler");
        assert_eq!(Kind::Saga.as_str(), "Saga");
        assert_eq!(Kind::ProcessManager.as_str(), "ProcessManager");
        assert_eq!(Kind::Projector.as_str(), "Projector");
        assert_eq!(Kind::Upcaster.as_str(), "Upcaster");
    }

    #[test]
    fn kind_display_matches_as_str() {
        for kind in [
            Kind::CommandHandler,
            Kind::Saga,
            Kind::ProcessManager,
            Kind::Projector,
            Kind::Upcaster,
        ] {
            assert_eq!(kind.to_string(), kind.as_str());
        }
    }
}

/// Compile-time kind marker and dispatch-table constructor.
///
/// `Router::with_handler::<H, F>` reads `H::KIND` and `H::handler_config()`
/// without invoking the factory, and `Router::build` turns the factory into
/// the component's angzarr-router dispatch table through
/// [`HandlerKind::component`].
pub trait HandlerKind: Sized + Send + Sync + 'static {
    const KIND: Kind;

    /// Static handler config — no instance required.
    fn handler_config() -> HandlerConfig;

    /// The component's angzarr-router dispatch table. Handler methods run
    /// on a fresh instance from `factory` per dispatch.
    #[doc(hidden)]
    fn component(
        factory: crate::router::component::Factory<Self>,
    ) -> crate::router::component::Component;
}
