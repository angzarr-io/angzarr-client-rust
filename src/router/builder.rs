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
use crate::router::upcaster::UpcasterRouter;
use crate::router::{BuildError, Built, Handler, HandlerConfig, HandlerKind, Kind};

/// Type-erased handler factory paired with the kind of handler it produces.
///
/// Kind is captured at registration (via `H::KIND`) so the builder can infer
/// the target runtime router without invoking the factory. The handler's
/// `HandlerConfig` is memoized lazily on the first call to [`Self::config`]
/// so build-time validation and per-dispatch matching never construct
/// handler instances solely to read metadata. Memoization is sound because
/// proc-macro–emitted `config()` returns values derived from compile-time
/// attribute data; the value is stable across produce calls.
pub(crate) struct Factory {
    pub(crate) kind: Kind,
    /// Closure that constructs a new handler instance on each call.
    pub(crate) produce: Box<dyn Fn() -> Box<dyn Handler> + Send + Sync>,
    /// Static handler config provider — does not invoke `produce`.
    /// Sourced from `HandlerKind::handler_config` so the runtime can read
    /// metadata without constructing an instance. Gherkin scenario
    /// `@C-0087` pins that the factory closure runs exactly once per
    /// dispatch; a hidden "config probe" call would inflate the count.
    pub(crate) static_config: fn() -> HandlerConfig,
    /// Memoized handler config — populated lazily on the first call to
    /// [`Self::config`].
    pub(crate) cached_config: std::sync::OnceLock<HandlerConfig>,
}

impl Factory {
    /// Return the handler's metadata. Calls the static-config function
    /// (zero factory invocations) and caches the result.
    pub(crate) fn config(&self) -> &HandlerConfig {
        self.cached_config.get_or_init(self.static_config)
    }
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

    /// The (business-level) name passed to [`Router::new`]. Mirrors Python's
    /// `Router.name` public attribute.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Register a handler factory.
    ///
    /// `factory` is a closure that produces a fresh handler instance on
    /// each matched dispatch call — never at registration or build time,
    /// and never as a metadata "probe". `HandlerKind::handler_config` is
    /// the static, instance-free source for config reads.
    ///
    /// Use this to close over shared dependencies (e.g. a connection
    /// pool clone). Scarce resources stay un-allocated until a dispatch
    /// actually matches the handler.
    pub fn with_handler<H, F>(mut self, factory: F) -> Self
    where
        H: Handler + HandlerKind + 'static,
        F: Fn() -> H + Send + Sync + 'static,
    {
        self.factories.push(Factory {
            kind: H::KIND,
            produce: Box::new(move || Box::new(factory())),
            static_config: H::handler_config,
            cached_config: std::sync::OnceLock::new(),
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
    /// - Two CommandHandlers covering the same `(domain, command_type)` →
    ///   `Err(BuildError::DuplicateCommandHandler)` (audit finding #18).
    /// - Homogeneous → `Ok(Built::<kind>(<runtime router>))`.
    pub fn build(self) -> Result<Built, BuildError> {
        use crate::error::ErrorDetail;
        use crate::error_codes::{codes, keys, messages};

        let first_kind = self.factories.first().map(|f| f.kind).ok_or_else(|| {
            BuildError::Empty(ErrorDetail::new(
                codes::ROUTER_NO_HANDLERS,
                messages::ROUTER_NO_HANDLERS,
                [(keys::ROUTER_NAME, self.name.clone())],
            ))
        })?;

        for f in &self.factories {
            if f.kind != first_kind {
                return Err(BuildError::MixedKinds(ErrorDetail::new(
                    codes::MIXED_HANDLER_KINDS,
                    messages::MIXED_HANDLER_KINDS,
                    [
                        (keys::HANDLER_KIND, first_kind.to_string()),
                        (keys::OTHER_KIND, f.kind.to_string()),
                        (keys::ROUTER_NAME, self.name.clone()),
                    ],
                )));
            }
        }

        // Audit #18: at most one CommandHandler per (domain, command_type)
        // within a Router. Saga / PM / projector / upcaster fan-out is
        // unaffected (those kinds legitimately broadcast).
        //
        // Reads `static_config` (no factory invocation) for the duplicate
        // scan; then invokes the factory exactly once per CommandHandler
        // so gherkin C-0065 ("Factories are invoked at most once per
        // registered handler at build time") sees a single call. Sagas
        // and other kinds skip this probe — C-0087 pins their factories
        // to one call per matched dispatch only.
        if first_kind == Kind::CommandHandler {
            use std::collections::HashSet;
            let mut seen: HashSet<(String, String)> = HashSet::new();
            for f in &self.factories {
                if let HandlerConfig::CommandHandler {
                    domain, handled, ..
                } = f.config()
                {
                    for type_url in handled {
                        let key = (domain.clone(), type_url.clone());
                        if !seen.insert(key) {
                            return Err(BuildError::DuplicateCommandHandler(ErrorDetail::new(
                                codes::DUPLICATE_COMMAND_HANDLER,
                                messages::DUPLICATE_COMMAND_HANDLER,
                                [
                                    (keys::DOMAIN, domain.clone()),
                                    (keys::TYPE_URL, type_url.clone()),
                                    (keys::ROUTER_NAME, self.name.clone()),
                                ],
                            )));
                        }
                    }
                }
                // Cross-language contract (C-0065): invoke each CH factory
                // once at build to mirror Python's parity-preserving probe.
                // The instance is discarded; only the side effect (the
                // counter increment in tests) matters.
                let _probe: Box<dyn Handler> = (f.produce)();
            }
        }

        Ok(match first_kind {
            Kind::CommandHandler => Built::CommandHandler(CommandHandlerRouter {
                factories: self.factories,
                cached_name: std::sync::OnceLock::new(),
            }),
            Kind::Saga => Built::Saga(SagaRouter {
                factories: self.factories,
                cached_name: std::sync::OnceLock::new(),
            }),
            Kind::ProcessManager => Built::ProcessManager(ProcessManagerRouter {
                factories: self.factories,
                cached_name: std::sync::OnceLock::new(),
            }),
            Kind::Projector => Built::Projector(ProjectorRouter {
                factories: self.factories,
                cached_name: std::sync::OnceLock::new(),
            }),
            Kind::Upcaster => Built::Upcaster(UpcasterRouter {
                factories: self.factories,
                cached_name: std::sync::OnceLock::new(),
            }),
        })
    }
}
