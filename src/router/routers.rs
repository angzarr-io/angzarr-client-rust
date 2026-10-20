//! Typed runtime routers — output of [`Router::build`].
//!
//! Each wraps the one component a router serves: its angzarr-router
//! dispatch table, built for the hosting binary's rules version, and the
//! static handler metadata the gRPC adapters read.
//!
//! [`Router::build`]: crate::router::Router::build

use crate::proto::{
    CommandHandlerResponse, ComponentDescriptor, ContextualCommand, Cover, EventBook,
    ProcessManagerHandleRequest, ProcessManagerHandleResponse, Projection, SagaResponse,
    UpcastRequest, UpcastResponse,
};
use crate::router::component::{begin_dispatch, from_coded, Tables};
use crate::router::HandlerConfig;
use crate::ClientError;

/// Attach the request's cover to a propagating rejection so callers can
/// trace which stream (domain, edition, root) produced it. No-op for
/// non-rejection errors and when the cover is missing or already set.
fn stamp_cover(err: ClientError, cover: Option<&Cover>) -> ClientError {
    match (err, cover) {
        (ClientError::Rejected(rej), Some(cover)) if rej.cover.is_none() => {
            ClientError::Rejected(rej.with_cover(cover.clone()))
        }
        (other, _) => other,
    }
}

/// Runtime router over one command handler (aggregate).
pub struct CommandHandlerRouter {
    pub(crate) tables: Tables,
    pub(crate) config: HandlerConfig,
}

impl CommandHandlerRouter {
    /// Route a contextual command (its page's Command or Fact arm) to the
    /// aggregate.
    pub fn dispatch(&self, cmd: ContextualCommand) -> Result<CommandHandlerResponse, ClientError> {
        begin_dispatch();
        let cover = cmd.command.as_ref().and_then(|c| c.cover.clone());
        self.tables
            .inner
            .dispatch_command(&cmd)
            .map_err(|e| stamp_cover(from_coded(e), cover.as_ref()))
    }

    /// The aggregate's domain.
    pub fn name(&self) -> String {
        match &self.config {
            HandlerConfig::CommandHandler { domain, .. } => domain.clone(),
            _ => String::new(),
        }
    }

    /// Aggregates emit events, not cross-domain commands: always empty.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Runtime router over one saga.
pub struct SagaRouter {
    pub(crate) tables: Tables,
    pub(crate) config: HandlerConfig,
}

impl SagaRouter {
    /// Run the saga over every page of the source book.
    pub fn dispatch(&self, source: EventBook) -> Result<SagaResponse, ClientError> {
        begin_dispatch();
        self.tables
            .inner
            .dispatch_saga(&source)
            .map_err(|e| stamp_cover(from_coded(e), source.cover.as_ref()))
    }

    /// The saga's name.
    pub fn name(&self) -> String {
        match &self.config {
            HandlerConfig::Saga { name, .. } => name.clone(),
            _ => String::new(),
        }
    }

    /// The saga's one destination domain (decision 14.24): a command for
    /// any other domain fails INTERNAL (SAGA_COMMAND_OUTSIDE_DESTINATION).
    pub fn output_domain(&self) -> String {
        match &self.config {
            HandlerConfig::Saga { output_domain, .. } => output_domain.clone(),
            _ => String::new(),
        }
    }
}

/// Runtime router over one process manager.
pub struct ProcessManagerRouter {
    pub(crate) tables: Tables,
    pub(crate) config: HandlerConfig,
}

impl ProcessManagerRouter {
    /// Run the process manager on the request's newest trigger event.
    pub fn dispatch(
        &self,
        request: ProcessManagerHandleRequest,
    ) -> Result<ProcessManagerHandleResponse, ClientError> {
        begin_dispatch();
        let cover = request.trigger.as_ref().and_then(|t| t.cover.clone());
        self.tables
            .inner
            .dispatch_process_manager(&request)
            .map_err(|e| stamp_cover(from_coded(e), cover.as_ref()))
    }

    /// The process manager's name.
    pub fn name(&self) -> String {
        match &self.config {
            HandlerConfig::ProcessManager { name, .. } => name.clone(),
            _ => String::new(),
        }
    }

    /// The domains the process manager issues commands to.
    pub fn output_domains(&self) -> Vec<String> {
        match &self.config {
            HandlerConfig::ProcessManager { output_domains, .. } => output_domains.clone(),
            _ => Vec::new(),
        }
    }
}

/// Runtime router over one projector.
pub struct ProjectorRouter {
    pub(crate) tables: Tables,
    pub(crate) config: HandlerConfig,
}

impl ProjectorRouter {
    /// Run the projector over the book (it filters by its domains) and
    /// answer its `Projection`.
    pub fn dispatch(&self, book: EventBook) -> Result<Projection, ClientError> {
        begin_dispatch();
        self.tables
            .inner
            .dispatch_projector(&book)
            .map_err(from_coded)
    }

    /// The projector's name.
    pub fn name(&self) -> String {
        match &self.config {
            HandlerConfig::Projector { name, .. } => name.clone(),
            _ => String::new(),
        }
    }

    /// Projectors are read-side; no outbound destinations.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }
}

macro_rules! impl_component_descriptor {
    ($($ty:ident),*) => {$(
        impl $ty {
            /// The component's description (types.proto,
            /// ComponentDescriptor), built by angzarr-router from its
            /// table: what `ComponentService.Describe` answers. Refused when
            /// it has no rules version (MISSING_RULES_VERSION) or, for an
            /// aggregate or process manager, its state is not a protobuf
            /// message (MISSING_STATE_TYPE).
            pub fn component_descriptor(&self) -> Result<ComponentDescriptor, ClientError> {
                self.tables.inner.component_descriptor().map_err(from_coded)
            }

            /// True when the component has an upcaster chain
            /// ([`crate::router::Router::with_upcaster`]).
            pub fn has_upcaster(&self) -> bool {
                self.tables.has_upcaster()
            }

            /// Run the pages through the component's upcaster chain when the
            /// request is for the chain's domain (C-0136); otherwise, or
            /// with no chain, the pages pass through unchanged.
            pub fn upcast(&self, request: UpcastRequest) -> Result<UpcastResponse, ClientError> {
                begin_dispatch();
                self.tables.inner.upcast(&request).map_err(from_coded)
            }
        }
    )*};
}

impl_component_descriptor!(
    CommandHandlerRouter,
    SagaRouter,
    ProcessManagerRouter,
    ProjectorRouter
);

macro_rules! impl_debug {
    ($($ty:ident),*) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty))
                    .field("config", &self.config)
                    .finish_non_exhaustive()
            }
        }
    )*};
}

impl_debug!(
    CommandHandlerRouter,
    SagaRouter,
    ProcessManagerRouter,
    ProjectorRouter
);
