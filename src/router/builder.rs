//! Unified `Router` builder.
//!
//! Users call [`Router::new`], register handler factories via
//! [`Router::with_handler`], and call [`Router::build`] to obtain a typed
//! runtime router. Each registered type becomes an angzarr-router dispatch
//! table whose handler methods run on a fresh instance from the factory per
//! dispatch. `build` invokes each command-handler factory exactly once
//! (C-0065) and no other kind's.

use std::sync::Arc;

use crate::router::component::{build_error, Component};
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
    component: Box<dyn FnOnce() -> Component + Send + Sync>,
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
            component: Box::new(move || H::component(factory)),
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
    /// - A component table angzarr-router refuses (e.g. ambiguous
    ///   `compensates` entries) → `Err(BuildError::InvalidComponent)`.
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
        let mut builder = angzarr_router::router::RouterBuilder::new();
        let mut replay = None;
        for r in self.registrations {
            builder = match (r.component)() {
                Component::CommandHandler { table, replay: r } => {
                    if replay.is_none() {
                        replay = r;
                    }
                    builder.aggregate(BoxedCommandHandler(table))
                }
                Component::Saga(saga) => builder.saga(saga),
                Component::ProcessManager(pm) => builder.process_manager(BoxedPm(pm)),
                Component::Projector(p) => builder.projector(BoxedProjector(p)),
                Component::Upcaster(u) => builder.upcaster(u),
            };
        }
        let inner = builder.build().map_err(build_error)?;

        Ok(match first_kind {
            Kind::CommandHandler => Built::CommandHandler(CommandHandlerRouter {
                inner,
                configs,
                replay,
            }),
            Kind::Saga => Built::Saga(SagaRouter { inner, configs }),
            Kind::ProcessManager => Built::ProcessManager(ProcessManagerRouter { inner, configs }),
            Kind::Projector => Built::Projector(ProjectorRouter { inner, configs }),
            Kind::Upcaster => Built::Upcaster(UpcasterRouter { inner, configs }),
        })
    }
}

/// Adapters from the boxed tables back to the router's component traits.
struct BoxedCommandHandler(Box<dyn angzarr_router::router::CommandHandler>);
struct BoxedPm(Box<dyn angzarr_router::router::ProcessManagerHandler>);
struct BoxedProjector(Box<dyn angzarr_router::router::ProjectorHandler>);

mod adapters {
    use super::{BoxedCommandHandler, BoxedPm, BoxedProjector};
    use angzarr_router::error::CodedError;
    use angzarr_router::pb;
    use angzarr_router::process_manager::ProcessManagerRoute;
    use angzarr_router::router::{CommandHandler, ProcessManagerHandler, ProjectorHandler};

    impl CommandHandler for BoxedCommandHandler {
        fn domain(&self) -> &str {
            self.0.domain()
        }
        fn command_types(&self) -> Vec<String> {
            self.0.command_types()
        }
        fn claims_notification(&self, notification_any: &prost_types::Any) -> bool {
            self.0.claims_notification(notification_any)
        }
        fn validate(&self) -> Result<(), CodedError> {
            self.0.validate()
        }
        fn dispatch(
            &self,
            req: &pb::ContextualCommand,
        ) -> Result<pb::BusinessResponse, CodedError> {
            self.0.dispatch(req)
        }
        fn handle_fact(&self, req: &pb::FactRequest) -> Result<pb::EventBook, CodedError> {
            self.0.handle_fact(req)
        }
    }

    impl ProcessManagerRoute for BoxedPm {
        fn name(&self) -> &str {
            self.0.name()
        }
        fn pm_domain(&self) -> &str {
            self.0.pm_domain()
        }
        fn consumes(&self, domain: &str) -> bool {
            self.0.consumes(domain)
        }
    }

    impl ProcessManagerHandler for BoxedPm {
        fn validate(&self) -> Result<(), CodedError> {
            self.0.validate()
        }
        fn dispatch(
            &self,
            req: &pb::ProcessManagerHandleRequest,
        ) -> Result<pb::ProcessManagerHandleResponse, CodedError> {
            self.0.dispatch(req)
        }
    }

    impl ProjectorHandler for BoxedProjector {
        fn dispatch(&self, events: &pb::EventBook) -> Result<pb::Projection, CodedError> {
            self.0.dispatch(events)
        }
    }
}
