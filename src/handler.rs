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
    ProcessManagerHandleRequest, ProcessManagerHandleResponse, Projection, SagaHandleRequest,
    SagaResponse, UpcastRequest, UpcastResponse,
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
        request: Request<crate::proto::FactRequest>,
    ) -> Result<Response<EventBook>, Status> {
        // Audit #45: gate on metadata as high in the stack as
        // possible. No `#[handles_fact]` declared on any registered
        // handler → return UNIMPLEMENTED without invoking dispatch.
        // The coordinator's pass-through-persist fallback handles
        // facts for non-opted-in aggregates.
        if !self.router.supports_handle_fact() {
            return Err(Status::unimplemented(
                "no #[handles_fact] methods declared on registered command_handler",
            ));
        }
        let book = self
            .router
            .dispatch_fact(request.into_inner())
            .map_err(client_error_to_status)?;
        Ok(Response::new(book))
    }

    async fn replay(
        &self,
        request: Request<crate::proto::ReplayRequest>,
    ) -> Result<Response<crate::proto::ReplayResponse>, Status> {
        // Audit #45: gate on metadata. Aggregate did not opt in via
        // `#[command_handler(supports_replay = true)]` → return
        // UNIMPLEMENTED. Coordinator degrades MERGE_COMMUTATIVE to
        // MERGE_STRICT.
        if !self.router.supports_replay() {
            return Err(Status::unimplemented(
                "command_handler did not opt in via #[command_handler(supports_replay = true)]",
            ));
        }
        let resp = self
            .router
            .dispatch_replay(request.into_inner())
            .map_err(client_error_to_status)?;
        Ok(Response::new(resp))
    }
}

/// gRPC saga service wrapping a [`SagaRouter`].
pub struct SagaGrpc {
    router: Arc<SagaRouter>,
}

impl SagaGrpc {
    pub fn new(router: SagaRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

impl Clone for SagaGrpc {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
        }
    }
}

#[tonic::async_trait]
impl SagaService for SagaGrpc {
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
pub struct ProcessManagerGrpc {
    router: Arc<ProcessManagerRouter>,
}

impl ProcessManagerGrpc {
    pub fn new(router: ProcessManagerRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

#[tonic::async_trait]
impl ProcessManagerService for ProcessManagerGrpc {
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
pub struct ProjectorGrpc {
    router: Arc<ProjectorRouter>,
}

impl ProjectorGrpc {
    pub fn new(router: ProjectorRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

#[tonic::async_trait]
impl ProjectorService for ProjectorGrpc {
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
    // Audit #59: only the static `message` rides in `Status::message`.
    // Structured details remain client-language-internal for now.
    match err {
        ClientError::InvalidArgument(d) => Status::invalid_argument(d.message),
        ClientError::Connection(d) => Status::unavailable(d.message),
        ClientError::Transport(e) => Status::unavailable(e.to_string()),
        ClientError::Grpc(s) => *s,
        ClientError::InvalidTimestamp(d) => Status::invalid_argument(d.message),
        ClientError::Rejected(r) => r.into(),
    }
}

// ---------------------------------------------------------------------------
// Upcaster wrappers — unified-Router factory-based dispatch (R8b).
// ---------------------------------------------------------------------------

/// gRPC upcaster service wrapping an [`crate::router::UpcasterRouter`].
pub struct UpcasterGrpc {
    router: Arc<crate::router::upcaster::UpcasterRouter>,
}

impl UpcasterGrpc {
    pub fn new(router: crate::router::upcaster::UpcasterRouter) -> Self {
        Self {
            router: Arc::new(router),
        }
    }
}

#[tonic::async_trait]
impl UpcasterService for UpcasterGrpc {
    async fn upcast(
        &self,
        request: Request<UpcastRequest>,
    ) -> Result<Response<UpcastResponse>, Status> {
        let req = request.into_inner();
        let response = self.router.dispatch(req).map_err(client_error_to_status)?;
        Ok(Response::new(response))
    }
}

#[cfg(test)]
mod tests {
    //! Adapter-level error-mapping tests. Mirrors Python's
    //! `tests/router/test_grpc_adapters.py` coverage of how each
    //! `ClientError` variant projects onto a `tonic::Status`. Audit #59.
    use super::*;
    use crate::CommandRejectedError;

    #[test]
    fn invalid_argument_maps_to_invalid_argument() {
        let err = ClientError::invalid_argument(
            "BAD_INPUT",
            "value must be positive",
            std::iter::empty::<(String, String)>(),
        );
        let status = client_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert_eq!(status.message(), "value must be positive");
    }

    #[test]
    fn invalid_timestamp_maps_to_invalid_argument() {
        let err = ClientError::invalid_timestamp(
            "BAD_TS",
            "not RFC3339",
            std::iter::empty::<(String, String)>(),
        );
        let status = client_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn connection_error_maps_to_unavailable() {
        let err = ClientError::connection(
            "DOWN",
            "backend unreachable",
            std::iter::empty::<(String, String)>(),
        );
        let status = client_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert_eq!(status.message(), "backend unreachable");
    }

    #[test]
    fn grpc_passes_through_upstream_status_code() {
        let upstream = Status::resource_exhausted("quota");
        let err = ClientError::from(upstream);
        let status = client_error_to_status(err);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
        assert_eq!(status.message(), "quota");
    }

    #[test]
    fn rejected_invalid_argument_status_maps_to_invalid_argument() {
        let rej = CommandRejectedError::invalid_argument(
            "BAD",
            "bad input",
            std::iter::empty::<(String, String)>(),
        );
        let status = client_error_to_status(ClientError::Rejected(rej));
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn rejected_not_found_status_maps_to_not_found() {
        let rej = CommandRejectedError::not_found(
            "MISSING",
            "no such record",
            std::iter::empty::<(String, String)>(),
        );
        let status = client_error_to_status(ClientError::Rejected(rej));
        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    #[test]
    fn rejected_precondition_failed_default_maps_to_failed_precondition() {
        let rej = CommandRejectedError::precondition_failed(
            "CONFLICT",
            "out of order",
            std::iter::empty::<(String, String)>(),
        );
        let status = client_error_to_status(ClientError::Rejected(rej));
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }
}
