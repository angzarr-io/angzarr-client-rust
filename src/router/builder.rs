//! Unified `Router` builder.
//!
//! Users call [`Router::new`], register handler factories via
//! [`Router::with_handler`], and call [`Router::build`] to obtain a typed
//! runtime router. Each registered type becomes an angzarr-router dispatch
//! table whose handler methods run on a fresh instance from the factory per
//! dispatch. `build` invokes each command-handler factory exactly once
//! (C-0065) and no other kind's.

use std::sync::Arc;

use crate::router::component::{Table, Tables};
use crate::router::routers::{
    CommandHandlerRouter, ProcessManagerRouter, ProjectorRouter, SagaRouter, UpcasterRouter,
};
use crate::router::{BuildError, Built, Handler, HandlerConfig, HandlerKind, Kind};

/// One registration: kind and metadata read statically, the dispatch table
/// built on demand.
struct Registration {
    kind: Kind,
    config: HandlerConfig,
    /// Invokes the factory once (the command-handler build probe).
    probe: Box<dyn Fn() + Send + Sync>,
    /// Builds the component's dispatch table for a rules version.
    table: Table,
}

/// Builder that accumulates handler factories before dispatch.
pub struct Router {
    name: String,
    registrations: Vec<Registration>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("name", &self.name)
            .field("handlers", &self.registrations.len())
            .finish()
    }
}

impl Router {
    /// Start a new router with the given (business-level) name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registrations: Vec::new(),
        }
    }

    /// The (business-level) name passed to [`Router::new`].
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Register a handler factory.
    ///
    /// `factory` produces a fresh handler instance for each dispatch that
    /// reaches the handler. It is never invoked at registration; `build`
    /// invokes a command-handler factory once (C-0065).
    pub fn with_handler<H, F>(mut self, factory: F) -> Self
    where
        H: Handler + HandlerKind,
        F: Fn() -> H + Send + Sync + 'static,
    {
        let factory: crate::router::component::Factory<H> = Arc::new(factory);
        let probe_factory = Arc::clone(&factory);
        self.registrations.push(Registration {
            kind: H::KIND,
            config: H::handler_config(),
            probe: Box::new(move || {
                let _instance = probe_factory();
            }),
            table: std::sync::Arc::new(move |rules_version: &str| {
                H::component(Arc::clone(&factory), rules_version)
            }),
        });
        self
    }

    /// Number of handler factories registered.
    pub fn handler_count(&self) -> usize {
        self.registrations.len()
    }

    /// Finalize the router.
    ///
    /// - Empty → `Err(BuildError::Empty)`.
    /// - Mixed kinds → `Err(BuildError::MixedKinds)`.
    /// - Two command handlers covering the same `(domain, command_type)` →
    ///   `Err(BuildError::DuplicateCommandHandler)`.
    /// - A component table angzarr-router refuses →
    ///   `Err(BuildError::InvalidComponent)`.
    /// - Homogeneous → `Ok(Built::<kind>(<runtime router>))`.
    pub fn build(self) -> Result<Built, BuildError> {
        use crate::error::ErrorDetail;
        use crate::error_codes::{codes, keys, messages};

        let first_kind = self.registrations.first().map(|r| r.kind).ok_or_else(|| {
            BuildError::Empty(ErrorDetail::new(
                codes::ROUTER_NO_HANDLERS,
                messages::ROUTER_NO_HANDLERS,
                [(keys::ROUTER_NAME, self.name.clone())],
            ))
        })?;
        if let Some(other) = self.registrations.iter().find(|r| r.kind != first_kind) {
            return Err(BuildError::MixedKinds(ErrorDetail::new(
                codes::MIXED_HANDLER_KINDS,
                messages::MIXED_HANDLER_KINDS,
                [
                    (keys::HANDLER_KIND, first_kind.to_string()),
                    (keys::OTHER_KIND, other.kind.to_string()),
                    (keys::ROUTER_NAME, self.name.clone()),
                ],
            )));
        }

        if first_kind == Kind::CommandHandler {
            use std::collections::HashSet;
            let mut seen: HashSet<(String, String)> = HashSet::new();
            for r in &self.registrations {
                if let HandlerConfig::CommandHandler {
                    domain, handled, ..
                } = &r.config
                {
                    for type_url in handled {
                        if !seen.insert((domain.clone(), type_url.clone())) {
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
                // C-0065: each command handler is instantiated once at build.
                (r.probe)();
            }
        }

        let configs: Vec<HandlerConfig> = self
            .registrations
            .iter()
            .map(|r| r.config.clone())
            .collect();
        // Tables report no rules version until a host stamps its own.
        let tables = Tables::assemble(
            self.registrations.into_iter().map(|r| r.table).collect(),
            "",
        )?;

        Ok(match first_kind {
            Kind::CommandHandler => Built::CommandHandler(CommandHandlerRouter { tables, configs }),
            Kind::Saga => Built::Saga(SagaRouter { tables, configs }),
            Kind::ProcessManager => Built::ProcessManager(ProcessManagerRouter { tables, configs }),
            Kind::Projector => Built::Projector(ProjectorRouter { tables, configs }),
            Kind::Upcaster => Built::Upcaster(UpcasterRouter { tables, configs }),
        })
    }
}
