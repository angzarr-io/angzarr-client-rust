//! gRPC service adapters wrapping Tier 5 unified runtime routers.
//!
//! Each wrapper takes the matching `router::runtime::*Router` produced by
//! `Router::build().into_*()?` and exposes it as a `tonic` service.

use std::collections::BTreeMap;
use std::sync::Arc;

use tonic::metadata::{MetadataKey, MetadataMap, MetadataValue};
use tonic::{Request, Response, Status};

use crate::error_codes::{codes, messages};
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

/// Metadata trailer carrying the SCREAMING_SNAKE error code so polyglot
/// siblings can route on the structured identifier without parsing the
/// human-readable Status message.
pub const ERROR_CODE_HEADER: &str = "x-angzarr-error-code";

/// Prefix for metadata trailers carrying structured error details.
/// One header per detail entry: `x-angzarr-detail-<key>: <value>`.
pub const ERROR_DETAIL_HEADER_PREFIX: &str = "x-angzarr-detail-";

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
            return Err(unimplemented_with_code(
                codes::HANDLER_DOES_NOT_SUPPORT_FACT,
                messages::HANDLER_DOES_NOT_SUPPORT_FACT,
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
            return Err(unimplemented_with_code(
                codes::HANDLER_DOES_NOT_SUPPORT_REPLAY,
                messages::HANDLER_DOES_NOT_SUPPORT_REPLAY,
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
    // Structured `code` + `details` ride in trailing metadata so polyglot
    // siblings that read them get full fidelity; siblings that ignore
    // unknown trailers see exactly the previous behavior.
    let code = err.code();
    let (mut status, details): (Status, Option<&BTreeMap<String, String>>) = match err {
        ClientError::InvalidArgument(ref d) => (Status::invalid_argument(d.message), Some(&d.details)),
        ClientError::Connection(ref d) => (Status::unavailable(d.message), Some(&d.details)),
        ClientError::Transport(ref e) => (Status::unavailable(e.to_string()), None),
        ClientError::Grpc(s) => return *s,
        ClientError::InvalidTimestamp(ref d) => {
            (Status::invalid_argument(d.message), Some(&d.details))
        }
        ClientError::Rejected(ref r) => {
            // Reuse the existing CommandRejectedError → Status mapping
            // for status code, then attach structured details from the
            // rejection (including cover, if stamped).
            let mut s: Status = r.clone().into();
            attach_error_metadata(s.metadata_mut(), code, Some(&r.details));
            return s;
        }
    };
    attach_error_metadata(status.metadata_mut(), code, details);
    status
}

/// Build a `Status::unimplemented` whose trailing metadata carries the
/// SCREAMING_SNAKE inventory code.
fn unimplemented_with_code(code: &'static str, message: &'static str) -> Status {
    let mut status = Status::unimplemented(message);
    attach_error_metadata(status.metadata_mut(), code, None);
    status
}

/// Stamp `x-angzarr-error-code` + `x-angzarr-detail-<key>` headers onto a
/// gRPC Status's trailing metadata. Skips silently on any header that
/// isn't ASCII-safe — keeps callers from accidentally hard-failing on
/// pathological detail values.
fn attach_error_metadata(
    md: &mut MetadataMap,
    code: &'static str,
    details: Option<&BTreeMap<String, String>>,
) {
    if let Ok(v) = MetadataValue::try_from(code) {
        md.insert(ERROR_CODE_HEADER, v);
    }
    let Some(details) = details else { return };
    for (k, v) in details {
        let header = format!("{}{}", ERROR_DETAIL_HEADER_PREFIX, k);
        let Ok(key) = header.parse::<MetadataKey<_>>() else {
            continue;
        };
        let Ok(value) = MetadataValue::try_from(v.as_str()) else {
            continue;
        };
        md.insert(key, value);
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
        assert_eq!(
            status.metadata().get(ERROR_CODE_HEADER).map(|v| v.to_str().unwrap()),
            Some("BAD_INPUT"),
        );
    }

    #[test]
    fn invalid_argument_attaches_detail_headers() {
        let err = ClientError::invalid_argument(
            "BAD_INPUT",
            "bad",
            [("field", "amount"), ("expected", "positive")],
        );
        let status = client_error_to_status(err);
        let md = status.metadata();
        assert_eq!(
            md.get("x-angzarr-detail-field").map(|v| v.to_str().unwrap()),
            Some("amount"),
        );
        assert_eq!(
            md.get("x-angzarr-detail-expected").map(|v| v.to_str().unwrap()),
            Some("positive"),
        );
    }

    #[test]
    fn rejected_status_attaches_code_and_details() {
        let rej = CommandRejectedError::invalid_argument(
            "VALUE_NOT_POSITIVE",
            "value must be positive",
            [("field", "amount")],
        );
        let status = client_error_to_status(ClientError::Rejected(rej));
        let md = status.metadata();
        assert_eq!(
            md.get(ERROR_CODE_HEADER).map(|v| v.to_str().unwrap()),
            Some("VALUE_NOT_POSITIVE"),
        );
        assert_eq!(
            md.get("x-angzarr-detail-field").map(|v| v.to_str().unwrap()),
            Some("amount"),
        );
    }

    #[test]
    fn unimplemented_with_code_carries_inventory_code() {
        let status = unimplemented_with_code(
            codes::HANDLER_DOES_NOT_SUPPORT_FACT,
            messages::HANDLER_DOES_NOT_SUPPORT_FACT,
        );
        assert_eq!(status.code(), tonic::Code::Unimplemented);
        assert_eq!(status.message(), messages::HANDLER_DOES_NOT_SUPPORT_FACT);
        assert_eq!(
            status.metadata().get(ERROR_CODE_HEADER).map(|v| v.to_str().unwrap()),
            Some(codes::HANDLER_DOES_NOT_SUPPORT_FACT),
        );
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
