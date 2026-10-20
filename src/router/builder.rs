//! The `Router` builder.
//!
//! A router serves exactly one component (one coordinator = one component =
//! one business process). Users call [`Router::new`], register the
//! component's handler factory with [`Router::with_handler`], and call
//! [`Router::build`] to obtain a typed runtime router. The type system
//! enforces "exactly one": a second `with_handler` and a `build` with none do
//! not compile. The component becomes an angzarr-router dispatch table
//! whose handler methods run on a fresh instance from the factory per
//! dispatch; `build` invokes a command-handler factory exactly once (C-0065)
//! and no other kind's. The component may also have one upcaster chain
//! ([`Router::with_upcaster`]); a second does not compile either.

use std::sync::Arc;

use std::marker::PhantomData;

use crate::router::component::{Table, Tables, UpcasterTable};
use crate::router::routers::{
    CommandHandlerRouter, ProcessManagerRouter, ProjectorRouter, SagaRouter,
};
use crate::router::{BuildError, Built, Handler, HandlerConfig, HandlerKind, Kind, UpcasterKind};

/// A router with no component yet: [`Router::with_handler`] gives it one.
#[derive(Debug, Default)]
pub struct NoComponent;

/// A router's one component: kind and metadata read statically, the
/// dispatch table built on demand.
pub struct OneComponent {
    kind: Kind,
    config: HandlerConfig,
    /// Invokes the factory once (the command-handler build probe).
    probe: Box<dyn Fn() + Send + Sync>,
    /// Builds the component's dispatch table for a rules version.
    table: Table,
}

impl std::fmt::Debug for OneComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneComponent")
            .field("kind", &self.kind)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// A component with no upcaster chain yet: [`Router::with_upcaster`] gives
/// it one.
#[derive(Debug, Default)]
pub struct NoUpcaster;

/// A component with its one upcaster chain.
#[derive(Debug, Default)]
pub struct OneUpcaster;

/// Builder for the router serving one component (and its upcaster chain,
/// when it has one).
pub struct Router<C = NoComponent, U = NoUpcaster> {
    name: String,
    component: C,
    upcaster: Option<(HandlerConfig, UpcasterTable)>,
    chain: PhantomData<U>,
}

impl<C: std::fmt::Debug, U> std::fmt::Debug for Router<C, U> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("name", &self.name)
            .field("component", &self.component)
            .field(
                "upcaster",
                &self.upcaster.as_ref().map(|(config, _)| config),
            )
            .finish()
    }
}

impl Router {
    /// Start a new router with the given (business-level) name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            component: NoComponent,
            upcaster: None,
            chain: PhantomData,
        }
    }

    /// Register the component's handler factory.
    ///
    /// `factory` produces a fresh handler instance for each dispatch. It is
    /// never invoked at registration; `build` invokes a command-handler
    /// factory once (C-0065).
    pub fn with_handler<H, F>(self, factory: F) -> Router<OneComponent>
    where
        H: Handler + HandlerKind,
        F: Fn() -> H + Send + Sync + 'static,
    {
        let factory: crate::router::component::Factory<H> = Arc::new(factory);
        let probe_factory = Arc::clone(&factory);
        Router {
            name: self.name,
            upcaster: None,
            chain: PhantomData,
            component: OneComponent {
                kind: H::KIND,
                config: H::handler_config(),
                probe: Box::new(move || {
                    let _instance = probe_factory();
                }),
                table: Arc::new(move |rules_version: &str| {
                    H::component(Arc::clone(&factory), rules_version)
                }),
            },
        }
    }
}

impl<C, U> Router<C, U> {
    /// The (business-level) name passed to [`Router::new`].
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Router<OneComponent> {
    /// Give the component its upcaster chain (upcaster.proto): the
    /// component's host serves `UpcasterService` from it. `factory` names
    /// the chain's type; upcasts are associated functions, so it is never
    /// invoked.
    pub fn with_upcaster<X, F>(self, factory: F) -> Router<OneComponent, OneUpcaster>
    where
        X: UpcasterKind + Handler,
        F: Fn() -> X + Send + Sync + 'static,
    {
        let _ = factory;
        Router {
            name: self.name,
            component: self.component,
            upcaster: Some((X::handler_config(), std::sync::Arc::new(X::upcaster))),
            chain: PhantomData,
        }
    }
}

impl<U> Router<OneComponent, U> {
    /// The kind of the router's component.
    pub fn kind(&self) -> Kind {
        self.component.kind
    }

    /// Finalize the router: `Built::<kind>(<runtime router>)`, or
    /// `Err(BuildError::InvalidComponent)` when angzarr-router refuses the
    /// component's table.
    pub fn build(self) -> Result<Built, BuildError> {
        let OneComponent {
            kind,
            config,
            probe,
            table,
        } = self.component;
        let upcaster = self.upcaster.map(|(_, table)| table);
        if kind == Kind::CommandHandler {
            // C-0065: a command handler is instantiated once at build.
            probe();
        }
        // The table reports no rules version until a host stamps its own.
        let tables = Tables::assemble(table, upcaster, "")?;
        Ok(match kind {
            Kind::CommandHandler => Built::CommandHandler(CommandHandlerRouter { tables, config }),
            Kind::Saga => Built::Saga(SagaRouter { tables, config }),
            Kind::ProcessManager => Built::ProcessManager(ProcessManagerRouter { tables, config }),
            Kind::Projector => Built::Projector(ProjectorRouter { tables, config }),
        })
    }
}
