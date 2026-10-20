//! Handler metadata and the traits the kind macros implement.
//!
//! Users apply `#[command_handler]` / `#[saga]` / `#[process_manager]` /
//! `#[projector]` / `#[upcaster]` to an inherent impl. The macro emits
//! [`HandlerKind`] (static metadata plus the component's angzarr-router
//! dispatch table) and [`Handler`] (the instance view of the metadata).

/// The four component kinds: what a router and a host serve one of. An
/// upcaster chain is part of its component, not a kind of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    CommandHandler,
    Saga,
    ProcessManager,
    Projector,
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
        /// Proto type URLs for events declared in `#[applies]` methods.
        applies: Vec<String>,
        /// Method name of the `#[state_factory]` if one was declared.
        /// `None` → runtime rebuild uses `Default::default()`.
        state_factory: Option<String>,
        /// Audit #45: proto type URLs of the facts declared in
        /// `#[handles_fact]` methods. A fact of any other type is refused
        /// INVALID_ARGUMENT / NO_FACT_HANDLER.
        handles_fact: Vec<String>,
    },
    Saga {
        name: String,
        /// The one source domain (ComponentOptions.input_domain).
        input_domain: String,
        /// The one destination domain (ComponentOptions.output_domain).
        output_domain: String,
        handled: Vec<String>,
    },
    ProcessManager {
        name: String,
        /// The workflow stream it owns (ComponentOptions.domain).
        domain: String,
        /// Its command targets: `output_domain` (when set) together with
        /// every `output_domains` entry (ComponentOptions).
        output_domains: Vec<String>,
        /// `(domain, type_url)` of each trigger: the domains are per event
        /// (`#[handles(Event, domain = "...")]`), never an input_domain.
        triggers: Vec<(String, String)>,
        handled: Vec<String>,
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
    /// Which component kind this config represents; `None` for an
    /// upcaster chain, which is part of a component.
    pub fn kind(&self) -> Option<Kind> {
        match self {
            Self::CommandHandler { .. } => Some(Kind::CommandHandler),
            Self::Saga { .. } => Some(Kind::Saga),
            Self::ProcessManager { .. } => Some(Kind::ProcessManager),
            Self::Projector { .. } => Some(Kind::Projector),
            Self::Upcaster { .. } => None,
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
/// `Display` emits the static message verbatim; runtime context (the
/// router's extras) rides in `details` for cucumber assertions on
/// `err.code` / `err.details["..."]`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    /// A component's own table is invalid, as reported by angzarr-router.
    /// `code` is the router's code.
    #[error("{}", .0.message)]
    InvalidComponent(crate::error::ErrorDetail),
}

impl BuildError {
    /// SCREAMING_SNAKE error code. Use for cucumber-style assertions
    /// rather than message-substring matching.
    pub fn code(&self) -> &'static str {
        match self {
            BuildError::InvalidComponent(d) => d.code,
        }
    }

    /// Static message (same string across languages for the same predicate).
    pub fn message(&self) -> &'static str {
        match self {
            BuildError::InvalidComponent(d) => d.message,
        }
    }

    /// Structured runtime context (the router's extras).
    pub fn details(&self) -> &std::collections::BTreeMap<String, String> {
        match self {
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
/// runtime router, or hand it to [`crate::ComponentHost::with_router`].
#[derive(Debug)]
pub enum Built {
    CommandHandler(crate::router::routers::CommandHandlerRouter),
    Saga(crate::router::routers::SagaRouter),
    ProcessManager(crate::router::routers::ProcessManagerRouter),
    Projector(crate::router::routers::ProjectorRouter),
}

impl Built {
    /// Rebuild every table to report `rules_version` (the host's).
    pub(crate) fn stamp(&mut self, rules_version: &str) -> Result<(), BuildError> {
        match self {
            Built::CommandHandler(r) => r.tables.stamp(rules_version),
            Built::Saga(r) => r.tables.stamp(rules_version),
            Built::ProcessManager(r) => r.tables.stamp(rules_version),
            Built::Projector(r) => r.tables.stamp(rules_version),
        }
    }

    /// The component's description (types.proto, ComponentDescriptor).
    pub fn component_descriptor(
        &self,
    ) -> Result<crate::proto::ComponentDescriptor, crate::ClientError> {
        match self {
            Built::CommandHandler(r) => r.component_descriptor(),
            Built::Saga(r) => r.component_descriptor(),
            Built::ProcessManager(r) => r.component_descriptor(),
            Built::Projector(r) => r.component_descriptor(),
        }
    }

    /// The angzarr-router engine, shared with the upcaster service.
    pub(crate) fn tables(&self) -> &crate::router::component::Tables {
        match self {
            Built::CommandHandler(r) => &r.tables,
            Built::Saga(r) => &r.tables,
            Built::ProcessManager(r) => &r.tables,
            Built::Projector(r) => &r.tables,
        }
    }
}

/// The instance view of a handler's metadata.
///
/// User code never implements `Handler` directly; the kind macros emit it.
pub trait Handler: Send + Sync {
    /// Describe this handler's kind and registered behaviors.
    fn config(&self) -> HandlerConfig;
}

/// An upcaster chain's static metadata and constructor: the macro trait of
/// `#[upcaster]`. A chain is part of its component (`Router::with_upcaster`),
/// never a component itself, so it implements this instead of
/// [`HandlerKind`].
pub trait UpcasterKind: Sized + Send + Sync + 'static {
    /// Static handler config — no instance required.
    fn handler_config() -> HandlerConfig;

    /// The chain's angzarr-router table.
    #[doc(hidden)]
    fn upcaster() -> angzarr_router::upcaster::UpcasterDispatch;
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

    /// The component's angzarr-router dispatch table, reporting
    /// `rules_version` (the hosting binary's). Handler methods run on a
    /// fresh instance from `factory` per dispatch.
    #[doc(hidden)]
    fn component(
        factory: crate::router::component::Factory<Self>,
        rules_version: &str,
    ) -> crate::router::component::Component;
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
    }

    #[test]
    fn kind_display_matches_as_str() {
        for kind in [
            Kind::CommandHandler,
            Kind::Saga,
            Kind::ProcessManager,
            Kind::Projector,
        ] {
            assert_eq!(kind.to_string(), kind.as_str());
        }
    }
}
