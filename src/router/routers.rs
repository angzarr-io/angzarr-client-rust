//! Typed runtime routers — output of [`Router::build`].
//!
//! Each wraps one validated [`angzarr_router::router::Router`] holding the
//! registered components' dispatch tables, plus the static handler
//! metadata the gRPC adapters and readiness probes read.
//!
//! [`Router::build`]: crate::router::Router::build

use crate::proto::{
    BusinessResponse, ContextualCommand, Cover, EventBook, FactRequest,
    ProcessManagerHandleRequest, ProcessManagerHandleResponse, Projection, ReplayRequest,
    ReplayResponse, SagaHandleRequest, SagaResponse, UpcastRequest, UpcastResponse,
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
    /// Route a contextual command (or a Notification delivery) to its
    /// aggregate.
    pub fn dispatch(&self, cmd: ContextualCommand) -> Result<BusinessResponse, ClientError> {
        begin_dispatch();
        let cover = cmd.command.as_ref().and_then(|c| c.cover.clone());
        self.inner
            .dispatch_command(&cmd)
            .map_err(|e| stamp_cover(from_coded(e), cover.as_ref()))
    }

    /// Route facts to the aggregate of the facts' cover domain.
    pub fn dispatch_fact(&self, request: FactRequest) -> Result<EventBook, ClientError> {
        begin_dispatch();
        self.inner.handle_fact(&request).map_err(from_coded)
    }

    /// Compute the state a `Replay` request describes, through the
    /// aggregate that opted in with `supports_replay = true` (an empty
    /// response when none did; the gRPC adapter gates on
    /// [`Self::supports_replay`]).
    pub fn dispatch_replay(&self, request: ReplayRequest) -> Result<ReplayResponse, ClientError> {
        begin_dispatch();
        match self.replay_domain() {
            Some(domain) => self.inner.replay(domain, &request).map_err(from_coded),
            None => Ok(ReplayResponse::default()),
        }
    }

    /// True if any registered aggregate declares a `#[handles_fact]` method.
    pub fn supports_handle_fact(&self) -> bool {
        self.configs.iter().any(|c| {
            matches!(c, HandlerConfig::CommandHandler { handles_fact, .. } if !handles_fact.is_empty())
        })
    }

    /// True if a registered aggregate opted into `Replay`.
    pub fn supports_replay(&self) -> bool {
        self.replay_domain().is_some()
    }

    /// The domain of the first aggregate that opted into `Replay`.
    fn replay_domain(&self) -> Option<&str> {
        self.configs.iter().find_map(|c| match c {
            HandlerConfig::CommandHandler {
                domain,
                supports_replay: true,
                ..
            } => Some(domain.as_str()),
            _ => None,
        })
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
    /// Run every saga consuming the source domain; their output merges in
    /// registration order and emitted commands are deferred.
    pub fn dispatch(&self, request: SagaHandleRequest) -> Result<SagaResponse, ClientError> {
        begin_dispatch();
        let cover = request.source.as_ref().and_then(|s| s.cover.clone());
        self.inner
            .dispatch_saga(&request)
            .map_err(|e| stamp_cover(from_coded(e), cover.as_ref()))
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

    /// Targets some saga addresses synchronously (`sync = true`).
    pub fn sync_output_domains(&self) -> Vec<String> {
        let targets: Vec<Vec<String>> = self
            .configs
            .iter()
            .filter_map(|c| match c {
                HandlerConfig::Saga {
                    target, sync: true, ..
                } => Some(vec![target.clone()]),
                _ => None,
            })
            .collect();
        union(targets.iter().map(Vec::as_slice))
    }

    /// True if some saga publishes to its target through the async bus.
    pub fn has_async_outputs(&self) -> bool {
        self.configs.iter().any(
            |c| matches!(c, HandlerConfig::Saga { sync: false, target, .. } if !target.is_empty()),
        )
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

    /// Every registered process manager's `sync_targets`, deduplicated.
    pub fn sync_output_domains(&self) -> Vec<String> {
        union(self.configs.iter().filter_map(|c| match c {
            HandlerConfig::ProcessManager { sync_targets, .. } => Some(sync_targets.as_slice()),
            _ => None,
        }))
    }

    /// True if some process manager has a target outside its `sync_targets`.
    pub fn has_async_outputs(&self) -> bool {
        self.configs.iter().any(|c| match c {
            HandlerConfig::ProcessManager {
                targets,
                sync_targets,
                ..
            } => targets
                .iter()
                .any(|t| !t.is_empty() && !sync_targets.contains(t)),
            _ => false,
        })
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
    /// Projectors are side-effect only; the response carries the book's
    /// cover and next sequence.
    pub fn dispatch(&self, book: EventBook) -> Result<Projection, ClientError> {
        begin_dispatch();
        self.inner.dispatch_projectors(&book).map_err(from_coded)?;
        Ok(Projection {
            cover: book.cover,
            projector: String::new(),
            sequence: book.next_sequence,
            projection: None,
        })
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
