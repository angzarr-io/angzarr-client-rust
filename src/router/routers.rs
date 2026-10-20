//! Typed runtime routers — output of [`Router::build`].
//!
//! Each wraps one validated [`angzarr_router::router::Router`] holding the
//! registered components' dispatch tables, plus the static handler
//! metadata the gRPC adapters read.
//!
//! [`Router::build`]: crate::router::Router::build

use crate::proto::{
    CommandHandlerResponse, ContextualCommand, Cover, EventBook, ProcessManagerHandleRequest,
    ProcessManagerHandleResponse, Projection, SagaResponse, UpcastRequest, UpcastResponse,
};
use crate::router::component::{begin_dispatch, from_coded};
use crate::router::HandlerConfig;
use crate::ClientError;

/// Attach the request's cover to a propagating rejection so callers can
/// trace which (domain, root, correlation_id) produced it. No-op for
/// non-rejection errors and when the cover is missing or already set.
fn stamp_cover(err: ClientError, cover: Option<&Cover>) -> ClientError {
    match (err, cover) {
        (ClientError::Rejected(rej), Some(cover)) if rej.cover.is_none() => {
            ClientError::Rejected(rej.with_cover(cover.clone()))
        }
        (other, _) => other,
    }
}

/// Deduplicated, order-preserving union of string lists.
fn union<'a>(lists: impl IntoIterator<Item = &'a [String]>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for list in lists {
        for item in list {
            if !item.is_empty() && !out.contains(item) {
                out.push(item.clone());
            }
        }
    }
    out
}

/// Runtime router over one or more command handlers (aggregates).
pub struct CommandHandlerRouter {
    pub(crate) inner: angzarr_router::router::Router,
    pub(crate) configs: Vec<HandlerConfig>,
}

impl CommandHandlerRouter {
    /// Route a contextual command (its page's Command or Fact arm) to its
    /// aggregate.
    pub fn dispatch(&self, cmd: ContextualCommand) -> Result<CommandHandlerResponse, ClientError> {
        begin_dispatch();
        let cover = cmd.command.as_ref().and_then(|c| c.cover.clone());
        self.inner
            .dispatch_command(&cmd)
            .map_err(|e| stamp_cover(from_coded(e), cover.as_ref()))
    }

    /// The first registered aggregate's domain.
    pub fn name(&self) -> String {
        match self.configs.first() {
            Some(HandlerConfig::CommandHandler { domain, .. }) => domain.clone(),
            _ => String::new(),
        }
    }

    /// Aggregates emit events, not cross-domain commands: always empty.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }

    /// Number of registered handlers.
    pub fn handler_count(&self) -> usize {
        self.configs.len()
    }
}

/// Runtime router over one or more sagas.
pub struct SagaRouter {
    pub(crate) inner: angzarr_router::router::Router,
    pub(crate) configs: Vec<HandlerConfig>,
}

impl SagaRouter {
    /// Run every saga consuming the source book's domain; their commands
    /// merge in registration order.
    pub fn dispatch(&self, source: EventBook) -> Result<SagaResponse, ClientError> {
        begin_dispatch();
        self.inner
            .dispatch_saga(&source)
            .map_err(|e| stamp_cover(from_coded(e), source.cover.as_ref()))
    }

    /// The first registered saga's name.
    pub fn name(&self) -> String {
        match self.configs.first() {
            Some(HandlerConfig::Saga { name, .. }) => name.clone(),
            _ => String::new(),
        }
    }

    /// Every registered saga's target, deduplicated.
    pub fn output_domains(&self) -> Vec<String> {
        let targets: Vec<Vec<String>> = self
            .configs
            .iter()
            .filter_map(|c| match c {
                HandlerConfig::Saga { target, .. } => Some(vec![target.clone()]),
                _ => None,
            })
            .collect();
        union(targets.iter().map(Vec::as_slice))
    }

    /// Number of registered handlers.
    pub fn handler_count(&self) -> usize {
        self.configs.len()
    }
}

/// Runtime router over one or more process managers.
pub struct ProcessManagerRouter {
    pub(crate) inner: angzarr_router::router::Router,
    pub(crate) configs: Vec<HandlerConfig>,
}

impl ProcessManagerRouter {
    /// Run the process managers the request is addressed to; their
    /// responses merge in registration order.
    pub fn dispatch(
        &self,
        request: ProcessManagerHandleRequest,
    ) -> Result<ProcessManagerHandleResponse, ClientError> {
        begin_dispatch();
        let cover = request.trigger.as_ref().and_then(|t| t.cover.clone());
        self.inner
            .dispatch_process_manager(&request)
            .map_err(|e| stamp_cover(from_coded(e), cover.as_ref()))
    }

    /// The first registered process manager's name.
    pub fn name(&self) -> String {
        match self.configs.first() {
            Some(HandlerConfig::ProcessManager { name, .. }) => name.clone(),
            _ => String::new(),
        }
    }

    /// Every registered process manager's targets, deduplicated.
    pub fn output_domains(&self) -> Vec<String> {
        union(self.configs.iter().filter_map(|c| match c {
            HandlerConfig::ProcessManager { targets, .. } => Some(targets.as_slice()),
            _ => None,
        }))
    }

    /// Number of registered handlers.
    pub fn handler_count(&self) -> usize {
        self.configs.len()
    }
}

/// Runtime router over one or more projectors.
pub struct ProjectorRouter {
    pub(crate) inner: angzarr_router::router::Router,
    pub(crate) configs: Vec<HandlerConfig>,
}

impl ProjectorRouter {
    /// Run every projector over the book (each filters by its domains).
    /// A sole projector answers its own `Projection`; several answer one
    /// carrying the book's cover, with no projector name and no
    /// projection. Either way `next_sequence` follows types.proto
    /// (`Projection`): the last page's sequence + 1, or the book's
    /// `next_sequence` when it has no pages.
    pub fn dispatch(&self, book: EventBook) -> Result<Projection, ClientError> {
        begin_dispatch();
        let mut projections = self.inner.dispatch_projectors(&book).map_err(from_coded)?;
        let next_sequence = book
            .pages
            .last()
            .map_or(book.next_sequence, |page| page.sequence + 1);
        let mut projection = if projections.len() == 1 {
            projections.remove(0)
        } else {
            Projection {
                cover: book.cover,
                ..Default::default()
            }
        };
        projection.next_sequence = next_sequence;
        Ok(projection)
    }

    /// The first registered projector's name.
    pub fn name(&self) -> String {
        match self.configs.first() {
            Some(HandlerConfig::Projector { name, .. }) => name.clone(),
            _ => String::new(),
        }
    }

    /// Projectors are read-side; no outbound destinations.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }

    /// Number of registered handlers.
    pub fn handler_count(&self) -> usize {
        self.configs.len()
    }
}

/// Runtime router over one or more upcasters.
pub struct UpcasterRouter {
    pub(crate) inner: angzarr_router::router::Router,
    pub(crate) configs: Vec<HandlerConfig>,
}

impl UpcasterRouter {
    /// Run every page through each upcaster of the request's domain, in
    /// registration order, each one's output feeding the next.
    pub fn dispatch(&self, request: UpcastRequest) -> Result<UpcastResponse, ClientError> {
        begin_dispatch();
        self.inner.upcast(&request).map_err(from_coded)
    }

    /// The first registered upcaster's name.
    pub fn name(&self) -> String {
        match self.configs.first() {
            Some(HandlerConfig::Upcaster { name, .. }) => name.clone(),
            _ => String::new(),
        }
    }

    /// Upcasters transform events in place; no outbound destinations.
    pub fn output_domains(&self) -> Vec<String> {
        Vec::new()
    }

    /// Number of registered handlers.
    pub fn handler_count(&self) -> usize {
        self.configs.len()
    }
}

macro_rules! impl_debug {
    ($($ty:ident),*) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty))
                    .field("configs", &self.configs)
                    .finish_non_exhaustive()
            }
        }
    )*};
}

impl_debug!(
    CommandHandlerRouter,
    SagaRouter,
    ProcessManagerRouter,
    ProjectorRouter,
    UpcasterRouter
);
