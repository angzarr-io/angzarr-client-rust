//! gRPC service adapters wrapping Tier 5 unified runtime routers.
//!
//! Each wrapper takes the matching `router::runtime::*Router` produced by
//! `Router::build().into_*()?` and exposes it as a `tonic` service.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::proto::{
    command_handler_service_server::CommandHandlerService,
    process_manager_service_server::ProcessManagerService,
    projector_service_server::ProjectorService, saga_service_server::SagaService,
    upcaster_service_server::UpcasterService, BusinessResponse, ContextualCommand, EventBook,
    ProcessManagerHandleRequest, ProcessManagerHandleResponse, ProcessManagerPrepareRequest,
    ProcessManagerPrepareResponse, Projection, SagaHandleRequest, SagaResponse, UpcastRequest,
    UpcastResponse,
};
use crate::router::runtime::{
    CommandHandlerRouter, ProcessManagerRouter, ProjectorRouter, SagaRouter,
};
use crate::ClientError;

/// gRPC command-handler service wrapping a [`CommandHandlerRouter`].
pub struct CommandHandlerGrpc {
    router: Arc<CommandHandlerRouter>,
}

impl CommandHandlerGrpc {
    pub fn new(router: CommandHandlerRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

impl Clone for CommandHandlerGrpc {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
        }
    }
}

#[tonic::async_trait]
impl CommandHandlerService for CommandHandlerGrpc {
    async fn handle(
        &self,
        request: Request<ContextualCommand>,
    ) -> Result<Response<BusinessResponse>, Status> {
        let cmd = request.into_inner();
        let response = self.router.dispatch(cmd).map_err(client_error_to_status)?;
        Ok(Response::new(response))
    }

    async fn handle_fact(
        &self,
        _request: Request<crate::proto::FactRequest>,
    ) -> Result<Response<EventBook>, Status> {
        // Fact handling is out of scope for the Tier 5 MVP.
        Err(Status::unimplemented(
            "handle_fact not implemented in Tier 5 runtime",
        ))
    }

    async fn replay(
        &self,
        _request: Request<crate::proto::ReplayRequest>,
    ) -> Result<Response<crate::proto::ReplayResponse>, Status> {
        // Replay support requires state packing hooks — deferred.
        Err(Status::unimplemented(
            "replay not implemented in Tier 5 runtime",
        ))
    }
}

/// gRPC saga service wrapping a [`SagaRouter`].
pub struct SagaHandler {
    router: Arc<SagaRouter>,
}

impl SagaHandler {
    pub fn new(router: SagaRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

impl Clone for SagaHandler {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
        }
    }
}

#[tonic::async_trait]
impl SagaService for SagaHandler {
    async fn handle(
        &self,
        request: Request<SagaHandleRequest>,
    ) -> Result<Response<SagaResponse>, Status> {
        let req = request.into_inner();
        let response = self.router.dispatch(req).map_err(client_error_to_status)?;
        Ok(Response::new(response))
    }
}

/// gRPC process-manager service wrapping a [`ProcessManagerRouter`].
pub struct ProcessManagerGrpcHandler {
    router: Arc<ProcessManagerRouter>,
}

impl ProcessManagerGrpcHandler {
    pub fn new(router: ProcessManagerRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

#[tonic::async_trait]
impl ProcessManagerService for ProcessManagerGrpcHandler {
    async fn prepare(
        &self,
        _request: Request<ProcessManagerPrepareRequest>,
    ) -> Result<Response<ProcessManagerPrepareResponse>, Status> {
        Err(Status::unimplemented(
            "PM prepare not implemented in Tier 5 runtime",
        ))
    }

    async fn handle(
        &self,
        request: Request<ProcessManagerHandleRequest>,
    ) -> Result<Response<ProcessManagerHandleResponse>, Status> {
        let req = request.into_inner();
        let response = self.router.dispatch(req).map_err(client_error_to_status)?;
        Ok(Response::new(response))
    }
}

/// gRPC projector service wrapping a [`ProjectorRouter`].
pub struct ProjectorHandler {
    router: Arc<ProjectorRouter>,
}

impl ProjectorHandler {
    pub fn new(router: ProjectorRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

#[tonic::async_trait]
impl ProjectorService for ProjectorHandler {
    async fn handle(&self, request: Request<EventBook>) -> Result<Response<Projection>, Status> {
        let book = request.into_inner();
        let projection = self.router.dispatch(book).map_err(client_error_to_status)?;
        Ok(Response::new(projection))
    }

    async fn handle_speculative(
        &self,
        request: Request<EventBook>,
    ) -> Result<Response<Projection>, Status> {
        self.handle(request).await
    }
}

fn client_error_to_status(err: ClientError) -> Status {
    match err {
        ClientError::InvalidArgument(msg) => Status::invalid_argument(msg),
        ClientError::Connection(msg) => Status::unavailable(msg),
        ClientError::Transport(e) => Status::unavailable(e.to_string()),
        ClientError::Grpc(s) => *s,
        ClientError::InvalidTimestamp(msg) => Status::invalid_argument(msg),
    }
}

// ---------------------------------------------------------------------------
// Upcaster wrappers — unchanged Tier 5 semantics.
// Upcaster stays outside the unified Router (plan: "separate system").
// ---------------------------------------------------------------------------

pub type UpcasterHandleFn = fn(&[crate::proto::EventPage]) -> Vec<crate::proto::EventPage>;

pub type UpcasterHandleClosureFn =
    Arc<dyn Fn(&[crate::proto::EventPage]) -> Vec<crate::proto::EventPage> + Send + Sync>;

enum UpcasterHandleType {
    Fn(UpcasterHandleFn),
    Closure(UpcasterHandleClosureFn),
}

pub struct UpcasterGrpcHandler {
    name: String,
    domain: String,
    handle_type: Option<UpcasterHandleType>,
}

impl UpcasterGrpcHandler {
    pub fn new(name: impl Into<String>, domain: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            domain: domain.into(),
            handle_type: None,
        }
    }

    pub fn with_handle(mut self, handle_fn: UpcasterHandleFn) -> Self {
        self.handle_type = Some(UpcasterHandleType::Fn(handle_fn));
        self
    }

    pub fn with_handle_fn<H>(mut self, handle_fn: H) -> Self
    where
        H: Fn(&[crate::proto::EventPage]) -> Vec<crate::proto::EventPage> + Send + Sync + 'static,
    {
        self.handle_type = Some(UpcasterHandleType::Closure(Arc::new(handle_fn)));
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn domain(&self) -> &str {
        &self.domain
    }
}

#[tonic::async_trait]
impl UpcasterService for UpcasterGrpcHandler {
    async fn upcast(
        &self,
        request: Request<UpcastRequest>,
    ) -> Result<Response<UpcastResponse>, Status> {
        let req = request.into_inner();
        let result = match &self.handle_type {
            Some(UpcasterHandleType::Fn(handle_fn)) => handle_fn(&req.events),
            Some(UpcasterHandleType::Closure(handle_fn)) => handle_fn(&req.events),
            None => req.events,
        };
        Ok(Response::new(UpcastResponse { events: result }))
    }
}

/// Re-export retained for back-compat in callers that might use it.
pub type StatePacker<S> = fn(&S) -> Result<prost_types::Any, Status>;
