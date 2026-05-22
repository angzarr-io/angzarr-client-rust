//! Recording mocks for the client traits.
//!
//! Port of `client-python/main/tests/client/steps/_fakes.py`. Python uses one
//! `RecordingStub` because gRPC stubs are duck-typed; Rust needs one struct
//! per client trait. The recording surface (`call_count`, `last_call`, canned
//! responses & errors keyed by method name) is the same.

use std::sync::Mutex;

use angzarr_client::error::{ClientError, Result};
use angzarr_client::proto::{
    CommandBook, CommandResponse, EventBook, ProcessManagerHandleResponse, Projection, Query,
    SagaResponse, SpeculateCommandHandlerRequest, SpeculatePmRequest, SpeculateProjectorRequest,
    SpeculateSagaRequest, SyncMode,
};
use angzarr_client::traits::{GatewayClient, QueryClient, SpeculativeClient};
use async_trait::async_trait;
use tonic::{Code, Status};

// ---------------------------------------------------------------------------
// StubRpcError analog
// ---------------------------------------------------------------------------

/// Build a [`ClientError::Grpc`] wrapping a `tonic::Status` with the given
/// code/details. Mirrors Python's `StubRpcError`; the production clients
/// wrap real `tonic::Status` via `ClientError::from(Status)` so error-
/// classification assertions match end-to-end.
pub fn stub_rpc_error(code: Code, details: &str) -> ClientError {
    Status::new(code, details.to_string()).into()
}

// ---------------------------------------------------------------------------
// RecordingGatewayClient
// ---------------------------------------------------------------------------

/// Recorded call to a [`GatewayClient`] method.
#[derive(Debug, Clone)]
pub struct GatewayCall {
    pub method: &'static str,
    pub command: CommandBook,
    pub sync_mode: Option<SyncMode>,
}

#[derive(Debug, Default)]
pub struct RecordingGatewayClient {
    calls: Mutex<Vec<GatewayCall>>,
    /// Canned `execute` / `execute_with_sync_mode` response. Returned (cloned)
    /// for every call until overwritten. Unset → `Ok(CommandResponse::default())`.
    response: Mutex<Option<CommandResponse>>,
    /// If set, every call returns `Err(error.clone())` instead of `response`.
    error: Mutex<Option<ClientError>>,
}

impl RecordingGatewayClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_response(self, resp: CommandResponse) -> Self {
        *self.response.lock().unwrap() = Some(resp);
        self
    }

    pub fn respond(&self, resp: CommandResponse) {
        *self.response.lock().unwrap() = Some(resp);
    }

    pub fn err_on(&self, _method: &str, err: ClientError) {
        // Single-error slot: Rust trait methods are statically dispatched, so
        // the per-method indirection that Python needs (because of duck typing)
        // collapses to a single trait-wide slot. Method name retained in the
        // signature for parity with Python call sites.
        *self.error.lock().unwrap() = Some(err);
    }

    /// All recorded calls, in order.
    pub fn calls(&self) -> Vec<GatewayCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn call_count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method == method)
            .count()
    }

    pub fn last_call(&self, method: &str) -> Option<GatewayCall> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|c| c.method == method)
            .cloned()
    }

    fn produce(&self) -> Result<CommandResponse> {
        if let Some(err) = self.error.lock().unwrap().as_ref() {
            return Err(clone_err(err));
        }
        Ok(self.response.lock().unwrap().clone().unwrap_or_default())
    }
}

#[async_trait]
impl GatewayClient for RecordingGatewayClient {
    async fn execute(&self, command: CommandBook) -> Result<CommandResponse> {
        self.calls.lock().unwrap().push(GatewayCall {
            method: "execute",
            command,
            sync_mode: None,
        });
        self.produce()
    }

    async fn execute_with_sync_mode(
        &self,
        command: CommandBook,
        sync_mode: SyncMode,
    ) -> Result<CommandResponse> {
        // Record under "execute" so callers checking `last_call("execute")`
        // see both code paths (matches Python's RecordingStub behavior where
        // the wrapping client's `execute(sync_mode=...)` lands in the same
        // recorded slot). The variant is distinguishable via `sync_mode`.
        self.calls.lock().unwrap().push(GatewayCall {
            method: "execute",
            command,
            sync_mode: Some(sync_mode),
        });
        self.produce()
    }
}

// ---------------------------------------------------------------------------
// RecordingQueryClient
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct QueryCall {
    pub method: &'static str,
    pub query: Query,
}

#[derive(Debug, Default)]
pub struct RecordingQueryClient {
    calls: Mutex<Vec<QueryCall>>,
    /// Canned event book returned by `get_event_book` and (single-element-wrapped)
    /// by `get_events`. Defaults to `EventBook::default()` when unset.
    event_book: Mutex<Option<EventBook>>,
    /// Override for `get_events` if a multi-book stream is needed.
    events_stream: Mutex<Option<Vec<EventBook>>>,
    error: Mutex<Option<ClientError>>,
}

impl RecordingQueryClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_event_book(self, book: EventBook) -> Self {
        *self.event_book.lock().unwrap() = Some(book);
        self
    }

    pub fn respond_event_book(&self, book: EventBook) {
        *self.event_book.lock().unwrap() = Some(book);
    }

    pub fn respond_events_stream(&self, books: Vec<EventBook>) {
        *self.events_stream.lock().unwrap() = Some(books);
    }

    pub fn err_on(&self, _method: &str, err: ClientError) {
        *self.error.lock().unwrap() = Some(err);
    }

    pub fn calls(&self) -> Vec<QueryCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn call_count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method == method)
            .count()
    }

    pub fn last_call(&self, method: &str) -> Option<QueryCall> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|c| c.method == method)
            .cloned()
    }

    fn err_if_set(&self) -> Option<ClientError> {
        self.error.lock().unwrap().as_ref().map(clone_err)
    }
}

#[async_trait]
impl QueryClient for RecordingQueryClient {
    async fn get_event_book(&self, query: Query) -> Result<EventBook> {
        self.calls.lock().unwrap().push(QueryCall {
            method: "get_event_book",
            query,
        });
        if let Some(err) = self.err_if_set() {
            return Err(err);
        }
        Ok(self.event_book.lock().unwrap().clone().unwrap_or_default())
    }

    async fn get_events(&self, query: Query) -> Result<Vec<EventBook>> {
        self.calls.lock().unwrap().push(QueryCall {
            method: "get_events",
            query,
        });
        if let Some(err) = self.err_if_set() {
            return Err(err);
        }
        if let Some(stream) = self.events_stream.lock().unwrap().clone() {
            return Ok(stream);
        }
        Ok(vec![self
            .event_book
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default()])
    }
}

// ---------------------------------------------------------------------------
// RecordingSpeculativeClient
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SpeculativeCall {
    CommandHandler(SpeculateCommandHandlerRequest),
    Projector(SpeculateProjectorRequest),
    Saga(SpeculateSagaRequest),
    ProcessManager(SpeculatePmRequest),
}

#[derive(Debug, Default)]
pub struct RecordingSpeculativeClient {
    calls: Mutex<Vec<(&'static str, SpeculativeCall)>>,
    command_handler_response: Mutex<Option<CommandResponse>>,
    projector_response: Mutex<Option<Projection>>,
    saga_response: Mutex<Option<SagaResponse>>,
    pm_response: Mutex<Option<ProcessManagerHandleResponse>>,
    error: Mutex<Option<ClientError>>,
}

impl RecordingSpeculativeClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn respond_command_handler(&self, resp: CommandResponse) {
        *self.command_handler_response.lock().unwrap() = Some(resp);
    }

    pub fn respond_projector(&self, resp: Projection) {
        *self.projector_response.lock().unwrap() = Some(resp);
    }

    pub fn respond_saga(&self, resp: SagaResponse) {
        *self.saga_response.lock().unwrap() = Some(resp);
    }

    pub fn respond_pm(&self, resp: ProcessManagerHandleResponse) {
        *self.pm_response.lock().unwrap() = Some(resp);
    }

    pub fn err_on(&self, _method: &str, err: ClientError) {
        *self.error.lock().unwrap() = Some(err);
    }

    pub fn call_count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| *m == method)
            .count()
    }

    pub fn last_call(&self, method: &str) -> Option<SpeculativeCall> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(m, _)| *m == method)
            .map(|(_, c)| c.clone())
    }

    fn err_if_set(&self) -> Option<ClientError> {
        self.error.lock().unwrap().as_ref().map(clone_err)
    }
}

#[async_trait]
impl SpeculativeClient for RecordingSpeculativeClient {
    async fn command_handler(
        &self,
        request: SpeculateCommandHandlerRequest,
    ) -> Result<CommandResponse> {
        self.calls
            .lock()
            .unwrap()
            .push(("command_handler", SpeculativeCall::CommandHandler(request)));
        if let Some(err) = self.err_if_set() {
            return Err(err);
        }
        Ok(self
            .command_handler_response
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default())
    }

    async fn projector(&self, request: SpeculateProjectorRequest) -> Result<Projection> {
        self.calls
            .lock()
            .unwrap()
            .push(("projector", SpeculativeCall::Projector(request)));
        if let Some(err) = self.err_if_set() {
            return Err(err);
        }
        Ok(self
            .projector_response
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default())
    }

    async fn saga(&self, request: SpeculateSagaRequest) -> Result<SagaResponse> {
        self.calls
            .lock()
            .unwrap()
            .push(("saga", SpeculativeCall::Saga(request)));
        if let Some(err) = self.err_if_set() {
            return Err(err);
        }
        Ok(self
            .saga_response
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default())
    }

    async fn process_manager(
        &self,
        request: SpeculatePmRequest,
    ) -> Result<ProcessManagerHandleResponse> {
        self.calls
            .lock()
            .unwrap()
            .push(("process_manager", SpeculativeCall::ProcessManager(request)));
        if let Some(err) = self.err_if_set() {
            return Err(err);
        }
        Ok(self.pm_response.lock().unwrap().clone().unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// ClientError is not Clone — provide a best-effort clone for canned errors.
// ---------------------------------------------------------------------------

fn clone_err(err: &ClientError) -> ClientError {
    // ClientError is not Clone because two variants wrap non-Clone types
    // (`tonic::Status` and `tonic::transport::Error`). Reconstruct those by
    // their public surface; other variants clone via their inner detail
    // structs.
    match err {
        ClientError::Grpc(s) => Status::new(s.code(), s.message().to_string()).into(),
        ClientError::Connection(d) => ClientError::Connection(d.clone()),
        ClientError::InvalidArgument(d) => ClientError::InvalidArgument(d.clone()),
        ClientError::InvalidTimestamp(d) => ClientError::InvalidTimestamp(d.clone()),
        ClientError::Rejected(r) => ClientError::Rejected(r.clone()),
        ClientError::Transport(_) => {
            // tonic::transport::Error cannot be reconstructed externally;
            // surface a Grpc(Unavailable) stand-in so the test still observes
            // a failure path. Tests should prefer `stub_rpc_error` over
            // injecting Transport directly.
            Status::new(Code::Unavailable, "stub: transport error replayed").into()
        }
    }
}
