//! gRPC service adapters wrapping the unified runtime routers.
//!
//! Each wrapper takes the matching `router::routers::*Router` produced by
//! `Router::build()` (one `Built` variant per kind) and exposes it as a
//! `tonic` service. Handler dispatch is synchronous user code, so every
//! adapter runs it on tokio's blocking pool: a handler doing blocking I/O
//! occupies a blocking thread, never an async worker.

use std::sync::Arc;

use tonic::{Code, Request, Response, Status};

use crate::error::build_status_details;
use crate::error_codes::{codes, messages};
use crate::proto::{
    command_handler_service_server::CommandHandlerService,
    component_service_server::ComponentService,
    process_manager_service_server::ProcessManagerService,
    projector_service_server::ProjectorService, saga_service_server::SagaService,
    upcaster_service_server::UpcasterService, CommandHandlerResponse, ComponentDescriptor,
    ContextualCommand, DescribeRequest, EventBook, ProcessManagerHandleRequest,
    ProcessManagerHandleResponse, Projection, SagaResponse, UpcastRequest, UpcastResponse,
};
use crate::router::routers::{
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
    ) -> Result<Response<CommandHandlerResponse>, Status> {
        // A command or a fact (the page's body arm); a type no handler
        // claims is refused by the router with INVALID_ARGUMENT.
        let cmd = request.into_inner();
        let response = on_blocking_pool(&self.router, move |r| r.dispatch(cmd)).await?;
        Ok(Response::new(response))
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
    async fn handle(&self, request: Request<EventBook>) -> Result<Response<SagaResponse>, Status> {
        let req = request.into_inner();
        let response = on_blocking_pool(&self.router, move |r| r.dispatch(req)).await?;
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

impl Clone for ProcessManagerGrpc {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
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
        let response = on_blocking_pool(&self.router, move |r| r.dispatch(req)).await?;
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

impl Clone for ProjectorGrpc {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
        }
    }
}

#[tonic::async_trait]
impl ProjectorService for ProjectorGrpc {
    async fn handle(&self, request: Request<EventBook>) -> Result<Response<Projection>, Status> {
        let book = request.into_inner();
        let projection = on_blocking_pool(&self.router, move |r| r.dispatch(book)).await?;
        Ok(Response::new(projection))
    }
}

/// gRPC component service (component.proto): answers the coordinator's
/// `Describe` with the hosted component's description, built once when the
/// host starts. A description the router refused is answered as its error
/// status on every call; an upcaster, which has no description, is
/// answered UNIMPLEMENTED.
#[derive(Clone)]
pub struct ComponentGrpc {
    described: Arc<std::result::Result<ComponentDescriptor, Status>>,
}

impl ComponentGrpc {
    pub fn new(described: Result<Option<ComponentDescriptor>, ClientError>) -> Self {
        let described = match described {
            Ok(Some(descriptor)) => Ok(descriptor),
            Ok(None) => Err(with_canonical_details(
                Code::Unimplemented,
                messages::NO_COMPONENT_DESCRIPTION,
                codes::NO_COMPONENT_DESCRIPTION,
                None,
            )),
            Err(err) => Err(client_error_to_status(err)),
        };
        Self {
            described: Arc::new(described),
        }
    }
}

#[tonic::async_trait]
impl ComponentService for ComponentGrpc {
    async fn describe(
        &self,
        _request: Request<DescribeRequest>,
    ) -> Result<Response<ComponentDescriptor>, Status> {
        match self.described.as_ref() {
            Ok(described) => Ok(Response::new(described.clone())),
            Err(status) => Err(status.clone()),
        }
    }
}

/// Run synchronous router dispatch on tokio's blocking pool and map its
/// outcome to a gRPC status. A panicking handler becomes `INTERNAL`
/// (`HANDLER_PANICKED`) instead of tearing down the connection task.
async fn on_blocking_pool<R, T, F>(router: &Arc<R>, dispatch: F) -> Result<T, Status>
where
    R: Send + Sync + 'static,
    T: Send + 'static,
    F: FnOnce(&R) -> Result<T, ClientError> + Send + 'static,
{
    let router = Arc::clone(router);
    tokio::task::spawn_blocking(move || dispatch(&router))
        .await
        .map_err(|_| {
            with_canonical_details(
                Code::Internal,
                messages::HANDLER_PANICKED,
                codes::HANDLER_PANICKED,
                None,
            )
        })?
        .map_err(client_error_to_status)
}

fn client_error_to_status(err: ClientError) -> Status {
    // Audit #59: only the static `message` rides in `Status::message`.
    // Structured `code` + `details` (+ optional cover for rejections)
    // ride in `grpc-status-details-bin` as a canonical
    // `google.rpc.Status` carrying `google.rpc.ErrorInfo`. Polyglot
    // siblings read the trailer via their google.rpc bindings.
    //
    // The Rejected branch delegates to `From<CommandRejectedError>
    // for Status` so the direct Into-conversion path produces
    // identical wire output.
    let code = err.code();
    match err {
        ClientError::InvalidArgument(d) => {
            with_canonical_details(Code::InvalidArgument, d.message, code, Some(&d.details))
        }
        ClientError::Connection(d) => {
            with_canonical_details(Code::Unavailable, d.message, code, Some(&d.details))
        }
        ClientError::Transport(e) => {
            // The dynamic transport-level message rides in
            // Status::message verbatim — Box<tonic::transport::Error>
            // already produces a single-line summary.
            let m = e.to_string();
            let payload = build_status_details(Code::Unavailable, &m, code, None, None);
            Status::with_details(Code::Unavailable, m, bytes::Bytes::from(payload))
        }
        ClientError::Grpc(s) => *s,
        ClientError::InvalidTimestamp(d) => {
            with_canonical_details(Code::InvalidArgument, d.message, code, Some(&d.details))
        }
        ClientError::Rejected(r) => r.into(),
    }
}

/// Build a `Status` whose `grpc-status-details-bin` trailer carries the
/// canonical `google.rpc.Status`/`ErrorInfo` payload.
fn with_canonical_details(
    grpc_code: Code,
    message: &'static str,
    error_code: &'static str,
    details: Option<&std::collections::BTreeMap<String, String>>,
) -> Status {
    let payload = build_status_details(grpc_code, message, error_code, details, None);
    Status::with_details(grpc_code, message, bytes::Bytes::from(payload))
}

// ---------------------------------------------------------------------------
// Upcaster wrappers — unified-Router factory-based dispatch (R8b).
// ---------------------------------------------------------------------------

/// gRPC upcaster service wrapping an [`crate::router::UpcasterRouter`].
pub struct UpcasterGrpc {
    router: Arc<crate::router::routers::UpcasterRouter>,
}

impl UpcasterGrpc {
    pub fn new(router: crate::router::routers::UpcasterRouter) -> Self {
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
        let response = on_blocking_pool(&self.router, move |r| r.dispatch(req)).await?;
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

    use crate::error::unpack_status_details;

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
        let (code, _meta, _cover) = unpack_status_details(status.details()).expect("decode");
        assert_eq!(code, "BAD_INPUT");
    }

    #[test]
    fn invalid_argument_packs_canonical_details() {
        let err = ClientError::invalid_argument(
            "BAD_INPUT",
            "bad",
            [("field", "amount"), ("expected", "positive")],
        );
        let status = client_error_to_status(err);
        let (_code, metadata, _cover) = unpack_status_details(status.details()).expect("decode");
        assert_eq!(metadata.get("field").map(String::as_str), Some("amount"));
        assert_eq!(
            metadata.get("expected").map(String::as_str),
            Some("positive"),
        );
    }

    #[test]
    fn rejected_status_packs_code_and_details() {
        let rej = CommandRejectedError::invalid_argument(
            "VALUE_NOT_POSITIVE",
            "value must be positive",
            [("field", "amount")],
        );
        let status = client_error_to_status(ClientError::Rejected(rej));
        let (code, metadata, _cover) = unpack_status_details(status.details()).expect("decode");
        assert_eq!(code, "VALUE_NOT_POSITIVE");
        assert_eq!(metadata.get("field").map(String::as_str), Some("amount"));
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
