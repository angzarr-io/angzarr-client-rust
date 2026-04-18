//! Unified `Router` builder for the Tier 5 handler runtime.
//!
//! Users call [`Router::new`], register handler factories via
//! [`Router::with_handler`], and call [`Router::build`] to obtain a typed
//! runtime router. Factories are closures (`Fn() -> H`) invoked per dispatch
//! so handler state is isolated per request; they are *not* invoked at
//! registration or build time.

use crate::router::runtime::{
    CommandHandlerRouter, ProcessManagerRouter, ProjectorRouter, SagaRouter,
};
use crate::router::{BuildError, Built, Handler, HandlerKind, Kind};

/// Type-erased handler factory paired with the kind of handler it produces.
///
/// Kind is captured at registration (via `H::KIND`) so the builder can infer
/// the target runtime router without invoking the factory.
pub(crate) struct Factory {
    pub(crate) kind: Kind,
    /// Closure that constructs a new handler instance on each call.
    ///
    /// Read by dispatch starting in R6; storage-only in R3/R4.
    #[allow(dead_code)]
    pub(crate) produce: Box<dyn Fn() -> Box<dyn Handler> + Send + Sync>,
}

impl std::fmt::Debug for Factory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Factory")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Builder that accumulates handler factories before dispatch.
pub struct Router {
    #[allow(dead_code)] // surfaced via `.name()` once runtime routers carry it in R6+
    name: String,
    factories: Vec<Factory>,
}

impl Router {
    /// Start a new router with the given (business-level) name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            factories: Vec::new(),
        }
    }

    /// Register a handler factory.
    ///
    /// `factory` is a closure that produces a fresh handler instance on each
    /// dispatch call. It is **not** invoked at registration or build time.
    /// Use this to close over shared dependencies (e.g. a connection pool) or
    /// to hand in a pool checkout.
    pub fn with_handler<H, F>(mut self, factory: F) -> Self
    where
        H: Handler + HandlerKind + 'static,
        F: Fn() -> H + Send + Sync + 'static,
    {
        self.factories.push(Factory {
            kind: H::KIND,
            produce: Box::new(move || Box::new(factory())),
        });
        self
    }

    /// Number of handler factories registered.
    pub fn handler_count(&self) -> usize {
        self.factories.len()
    }

    /// Finalize the router.
    ///
    /// - Empty → `Err(BuildError::Empty)`.
    /// - Mixed kinds → `Err(BuildError::MixedKinds)`.
    /// - Homogeneous → `Ok(Built::<kind>(<runtime router>))`.
    pub fn build(self) -> Result<Built, BuildError> {
        let first_kind = self
            .factories
            .first()
            .map(|f| f.kind)
            .ok_or(BuildError::Empty)?;

        for f in &self.factories {
            if f.kind != first_kind {
                return Err(BuildError::MixedKinds(first_kind, f.kind));
            }
        }

        Ok(match first_kind {
            Kind::CommandHandler => Built::CommandHandler(CommandHandlerRouter {
                factories: self.factories,
            }),
            Kind::Saga => Built::Saga(SagaRouter {
                factories: self.factories,
            }),
            Kind::ProcessManager => Built::ProcessManager(ProcessManagerRouter {
                factories: self.factories,
            }),
            Kind::Projector => Built::Projector(ProjectorRouter {
                factories: self.factories,
            }),
        })
    }
}
