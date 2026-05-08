//! Default client implementations wrapping tonic gRPC clients.

use std::time::Duration;

use crate::error::{ClientError, Result};
use crate::error_codes::{codes, keys, messages};
use crate::proto::{
    command_handler_coordinator_service_client::CommandHandlerCoordinatorServiceClient as TonicCommandHandlerClient,
    event_query_service_client::EventQueryServiceClient as TonicQueryClient,
    process_manager_coordinator_service_client::ProcessManagerCoordinatorServiceClient as TonicPmClient,
    projector_coordinator_service_client::ProjectorCoordinatorServiceClient as TonicProjectorClient,
    saga_coordinator_service_client::SagaCoordinatorServiceClient as TonicSagaClient,
    CascadeErrorMode, CommandBook, CommandRequest, CommandResponse, EventBook,
    ProcessManagerHandleResponse, Projection, Query, SagaResponse, SpeculateCommandHandlerRequest,
    SpeculatePmRequest, SpeculateProjectorRequest, SpeculateSagaRequest, SyncMode,
};
use crate::retry::RetryPolicy;
use crate::traits;
use crate::transport::{resolve_ch_endpoint, TransportMode};
use async_trait::async_trait;
use tonic::transport::{Channel, Endpoint, Uri};
use tracing::warn;

/// Per-RPC deadline applied to every request issued via this client.
///
/// Without this, a stalled server keeps the client awaiting forever
/// (the TCP RST never fires under packet loss). Matches polyglot
/// sibling defaults — siblings expose this as `--rpc-timeout` /
/// `RpcTimeout` config; in Rust we wire it once on the channel.
const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP/2 keepalive ping interval.
///
/// Without keepalive, a half-open TCP connection won't surface as a
/// failure until the OS-level keepalive eventually fires (often >2h).
const DEFAULT_HTTP2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

/// Time to wait for a keepalive PONG before treating the connection
/// as dead.
const DEFAULT_HTTP2_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Cap on the number of `EventBook`s returned from a streaming
/// `get_events` call. Without this, a hostile or buggy server can
/// stream forever and OOM the client.
const DEFAULT_MAX_EVENT_BOOKS: usize = 100_000;

/// Apply the standard set of resilience knobs to a [`tonic::transport::Endpoint`].
///
/// - per-RPC deadline so stalled servers don't hang the client,
/// - HTTP/2 keepalive ping so half-open connections surface fast,
/// - TCP keepalive for the same reason at the socket layer.
fn apply_endpoint_defaults(ep: Endpoint) -> Endpoint {
    ep.timeout(DEFAULT_RPC_TIMEOUT)
        .tcp_keepalive(Some(DEFAULT_HTTP2_KEEPALIVE_INTERVAL))
        .http2_keep_alive_interval(DEFAULT_HTTP2_KEEPALIVE_INTERVAL)
        .keep_alive_timeout(DEFAULT_HTTP2_KEEPALIVE_TIMEOUT)
        .keep_alive_while_idle(true)
}

/// Normalize a TCP endpoint string. Adds a default `http://` scheme
/// when the caller hands us a bare `host:port` (e.g., the form
/// `transport::resolve_ch_endpoint` emits in distributed mode).
///
/// UDS endpoints are out of scope here — they go through their own
/// connector path; only call this for TCP-class strings.
fn normalize_tcp_endpoint(endpoint: &str) -> String {
    if endpoint.contains("://") {
        endpoint.to_string()
    } else {
        format!("http://{}", endpoint)
    }
}

/// Validate a UDS path: reject empty / NUL-bearing paths up front so
/// they surface as a non-retryable `ENDPOINT_INVALID_URI` instead of a
/// retried `CONNECTION_FAILED` storm against `UnixStream::connect`.
fn validate_uds_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(ClientError::connection(
            codes::ENDPOINT_INVALID_URI,
            messages::ENDPOINT_INVALID_URI,
            [(keys::CAUSE, "UDS path is empty".to_string())],
        ));
    }
    if path.contains('\0') {
        return Err(ClientError::connection(
            codes::ENDPOINT_INVALID_URI,
            messages::ENDPOINT_INVALID_URI,
            [(keys::CAUSE, "UDS path contains a NUL byte".to_string())],
        ));
    }
    Ok(())
}

/// Detect a UDS endpoint and return the socket path, or `None` for TCP.
///
/// Audit finding #39: lenient prefix detection matching Python's
/// `client.py::_create_channel`. Recognized forms:
///
///   - `/abs/path`              — absolute path, no scheme
///   - `./rel/path`             — relative path, no scheme
///   - `unix:relative/path`     — gRPC URI, relative
///   - `unix:/abs/path`         — gRPC URI, absolute (single-slash form)
///   - `unix:///abs/path`       — gRPC URI, absolute with empty authority
///
/// Anything else is treated as TCP.
fn detect_uds_path(endpoint: &str) -> Option<String> {
    if let Some(rest) = endpoint.strip_prefix("unix://") {
        Some(rest.to_string())
    } else if let Some(rest) = endpoint.strip_prefix("unix:") {
        Some(rest.to_string())
    } else if endpoint.starts_with('/') || endpoint.starts_with("./") {
        Some(endpoint.to_string())
    } else {
        None
    }
}

/// Retries connection with the provided `RetryPolicy` on failure.
///
/// Audit finding #44: backoff math comes from
/// [`RetryPolicy::compute_delay`] (the cross-language helper exported as
/// the public `RetryPolicy::execute` API) so the connection-retry path
/// uses the same formula and the same `rand`-based jitter (post-#29) as
/// every other retry call site. The previously-used `backon` dep is
/// dropped — single source of truth for retry semantics, no per-call-
/// site divergence between the public retry helper and the channel
/// connector.
async fn create_channel(endpoint: &str, retry: &RetryPolicy) -> Result<Channel> {
    let uds_path = detect_uds_path(endpoint);
    if let Some(ref path) = uds_path {
        validate_uds_path(path)?;
    }

    // Build the TCP-class Endpoint once outside the retry loop — a
    // parse error is operator-typo-class and shouldn't be retried.
    let tcp_endpoint: Option<Endpoint> = if uds_path.is_none() {
        let normalized = normalize_tcp_endpoint(endpoint);
        match Channel::from_shared(normalized.clone()) {
            Ok(ep) => Some(apply_endpoint_defaults(ep)),
            Err(e) => {
                return Err(ClientError::connection(
                    codes::ENDPOINT_INVALID_URI,
                    messages::ENDPOINT_INVALID_URI,
                    [
                        (keys::ENDPOINT, endpoint.to_string()),
                        (keys::CAUSE, e.to_string()),
                    ],
                ));
            }
        }
    } else {
        None
    };

    // Pre-build the dummy Endpoint used for UDS — its URI is ignored
    // by `connect_with_connector`, so any failure here is logic bug
    // territory and should not retry.
    let uds_dummy: Option<Endpoint> = if uds_path.is_some() {
        let ep = Endpoint::try_from("http://[::]:50051").map_err(|e| {
            ClientError::connection(
                codes::ENDPOINT_PARSE_FAILED,
                messages::ENDPOINT_PARSE_FAILED,
                [(keys::CAUSE, e.to_string())],
            )
        })?;
        Some(apply_endpoint_defaults(ep))
    } else {
        None
    };

    let mut last_error: Option<ClientError> = None;

    for attempt in 0..retry.max_attempts {
        if attempt > 0 {
            // RetryPolicy::compute_delay(N) = the delay before the N+1-th
            // attempt. attempt=1 sleeps compute_delay(0) = min_delay,
            // attempt=2 sleeps compute_delay(1) = 2 * min_delay, capped
            // at max_delay, optionally jittered.
            let delay = retry.compute_delay(attempt - 1);
            warn!(
                endpoint = %endpoint,
                attempt = attempt,
                backoff_ms = %delay.as_millis(),
                "gRPC connection failed, retrying after backoff"
            );
            tokio::time::sleep(delay).await;
        }

        let result = if let Some(ref path) = uds_path {
            let path = path.clone();
            uds_dummy
                .as_ref()
                .expect("uds_dummy is Some when uds_path is Some")
                .clone()
                .connect_with_connector(tower::service_fn(move |_: Uri| {
                    let path = path.clone();
                    async move {
                        tokio::net::UnixStream::connect(path)
                            .await
                            .map(hyper_util::rt::TokioIo::new)
                    }
                }))
                .await
        } else {
            tcp_endpoint
                .as_ref()
                .expect("tcp_endpoint is Some when uds_path is None")
                .connect()
                .await
        };

        match result {
            Ok(channel) => return Ok(channel),
            Err(e) => {
                last_error = Some(ClientError::connection(
                    codes::CONNECTION_FAILED,
                    messages::CONNECTION_FAILED,
                    [
                        (keys::ENDPOINT, endpoint.to_string()),
                        (keys::CAUSE, e.to_string()),
                    ],
                ));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        ClientError::connection(
            codes::CONNECTION_FAILED_MAX_RETRIES,
            messages::CONNECTION_FAILED_MAX_RETRIES,
            [(keys::ENDPOINT, endpoint.to_string())],
        )
    }))
}

/// Default event query client using tonic gRPC.
#[derive(Clone)]
pub struct QueryClient {
    inner: TonicQueryClient<Channel>,
}

impl QueryClient {
    /// Connect to an event query service at the given endpoint.
    ///
    /// Supports both TCP (host:port) and Unix Domain Sockets (file paths).
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Self::connect_with_retry(endpoint, &RetryPolicy::default()).await
    }

    /// Connect with a custom retry policy.
    pub async fn connect_with_retry(endpoint: &str, retry: &RetryPolicy) -> Result<Self> {
        let channel = create_channel(endpoint, retry).await?;
        Ok(Self::from_channel(channel))
    }

    /// Connect using an endpoint from environment variable with fallback.
    pub async fn from_env(env_var: &str, default: &str) -> Result<Self> {
        let endpoint = std::env::var(env_var).unwrap_or_else(|_| default.to_string());
        Self::connect(&endpoint).await
    }

    /// Create a client from an existing channel.
    pub fn from_channel(channel: Channel) -> Self {
        Self {
            inner: TonicQueryClient::new(channel),
        }
    }

    /// Explicitly release this client's channel handle.
    ///
    /// Equivalent to letting the client drop; provided for symmetry with the
    /// synchronous sibling clients (Java/C#/Python/C++/Go) where explicit
    /// close is idiomatic. `tonic::transport::Channel` is reference-counted
    /// internally — `close` on one clone is a no-op for connection
    /// lifecycle; the underlying channel only tears down when the last
    /// clone drops.
    pub fn close(self) {
        drop(self);
    }

    /// Query events for an aggregate (unary RPC, returns single EventBook).
    ///
    /// # Cancellation
    /// This is an `async fn`; cancel by dropping the returned future (e.g. via
    /// `tokio::select!`, `tokio::time::timeout`, or letting a higher-level
    /// `CancellationToken` abort the task). There is no explicit cancellation
    /// parameter — dropping the future releases the RPC slot. This matches the
    /// per-call cancellation APIs exposed by the other clients:
    ///
    /// - Go: `context.Context` argument
    /// - Java: `Duration timeout` overload via `stub.withDeadlineAfter(...)`
    /// - C#: `CancellationToken` parameter
    /// - C++: `std::chrono::milliseconds deadline` parameter
    /// - Python: `timeout: float | None` kwarg
    pub async fn get_event_book(&self, query: Query) -> Result<EventBook> {
        // Audit #69 stage (b): attach `x-correlation-id` from the
        // canonical Cover.correlation_id field on every outbound RPC,
        // so OTel exporters / mesh sidecars can filter on it without
        // decoding the body. Send-only — the server side does not read
        // the metadata header (data structures are canonical, body
        // wins for in-framework dispatch).
        let corr_id = query
            .cover
            .as_ref()
            .map(|c| c.correlation_id.clone())
            .unwrap_or_default();
        let req = crate::proto_ext::correlated_request(query, &corr_id);
        let response = self.inner.clone().get_event_book(req).await?;
        Ok(response.into_inner())
    }

    /// Query events for an aggregate (streaming RPC, returns all matching EventBooks).
    ///
    /// Uses the streaming `GetEvents` RPC to fetch multiple EventBooks.
    /// For a single EventBook, use `get_event_book()` instead.
    ///
    /// Capped at [`DEFAULT_MAX_EVENT_BOOKS`] books — a hostile or
    /// buggy server can otherwise stream forever and OOM the client.
    /// Use [`Self::get_events_with_limit`] for a custom cap.
    pub async fn get_events(&self, query: Query) -> Result<Vec<EventBook>> {
        self.get_events_with_limit(query, DEFAULT_MAX_EVENT_BOOKS).await
    }

    /// Same as [`Self::get_events`] with a caller-supplied cap on the
    /// number of EventBooks read from the stream.
    pub async fn get_events_with_limit(
        &self,
        query: Query,
        max_books: usize,
    ) -> Result<Vec<EventBook>> {
        let corr_id = query
            .cover
            .as_ref()
            .map(|c| c.correlation_id.clone())
            .unwrap_or_default();
        let req = crate::proto_ext::correlated_request(query, &corr_id);
        let mut stream = self.inner.clone().get_events(req).await?.into_inner();
        let mut results = Vec::with_capacity(max_books.min(1024));
        while let Some(book) = stream.message().await? {
            if results.len() >= max_books {
                return Err(ClientError::connection(
                    codes::STREAM_LIMIT_EXCEEDED,
                    messages::STREAM_LIMIT_EXCEEDED,
                    [
                        (keys::EXPECTED, max_books.to_string()),
                        (keys::ACTUAL, format!("{}+", max_books)),
                    ],
                ));
            }
            results.push(book);
        }
        Ok(results)
    }
}

#[async_trait]
impl traits::QueryClient for QueryClient {
    async fn get_event_book(&self, query: Query) -> Result<EventBook> {
        self.get_event_book(query).await
    }

    async fn get_events(&self, query: Query) -> Result<Vec<EventBook>> {
        self.get_events(query).await
    }
}

/// Default command handler coordinator client using tonic gRPC.
#[derive(Clone)]
pub struct CommandHandlerClient {
    inner: TonicCommandHandlerClient<Channel>,
}

impl CommandHandlerClient {
    /// Connect to a command handler coordinator at the given endpoint.
    ///
    /// Supports both TCP (host:port) and Unix Domain Sockets (file paths).
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Self::connect_with_retry(endpoint, &RetryPolicy::default()).await
    }

    /// Connect with a custom retry policy.
    pub async fn connect_with_retry(endpoint: &str, retry: &RetryPolicy) -> Result<Self> {
        let channel = create_channel(endpoint, retry).await?;
        Ok(Self::from_channel(channel))
    }

    /// Connect using an endpoint from environment variable with fallback.
    pub async fn from_env(env_var: &str, default: &str) -> Result<Self> {
        let endpoint = std::env::var(env_var).unwrap_or_else(|_| default.to_string());
        Self::connect(&endpoint).await
    }

    /// Create a client from an existing channel.
    pub fn from_channel(channel: Channel) -> Self {
        Self {
            inner: TonicCommandHandlerClient::new(channel),
        }
    }

    /// Explicitly release this client's channel handle.
    ///
    /// Equivalent to letting the client drop; provided for symmetry with the
    /// synchronous client libraries. If other clones hold the same channel,
    /// the connection stays open until the last reference drops.
    pub fn close(self) {
        drop(self);
    }

    /// Execute a command with specified sync mode.
    ///
    /// Use `SyncMode::Async` for fire-and-forget (default).
    /// Use `SyncMode::Simple` to wait for sync projectors.
    /// Use `SyncMode::Cascade` for full sync including saga cascade.
    pub async fn handle_command(&self, command: CommandRequest) -> Result<CommandResponse> {
        // Audit #69 stage (b): canonical correlation_id at
        // `command.command.cover.correlation_id` (CommandRequest →
        // CommandBook → Cover). Empty / missing skips the header.
        let corr_id = command
            .command
            .as_ref()
            .and_then(|cb| cb.cover.as_ref())
            .map(|c| c.correlation_id.clone())
            .unwrap_or_default();
        let req = crate::proto_ext::correlated_request(command, &corr_id);
        let response = self.inner.clone().handle_command(req).await?;
        Ok(response.into_inner())
    }

    /// Execute a command asynchronously (fire-and-forget).
    ///
    /// Convenience method that wraps CommandBook in CommandRequest with async sync mode.
    pub async fn handle(&self, command: CommandBook) -> Result<CommandResponse> {
        self.handle_command(CommandRequest {
            command: Some(command),
            sync_mode: SyncMode::Async as i32,
            cascade_error_mode: CascadeErrorMode::CascadeErrorFailFast as i32,
            cascade_id: None,
        })
        .await
    }

    /// Speculative execution against temporal state.
    pub async fn handle_sync_speculative(
        &self,
        request: SpeculateCommandHandlerRequest,
    ) -> Result<CommandResponse> {
        // Audit #69 stage (b): canonical correlation_id at
        // `request.command.cover.correlation_id`.
        let corr_id = request
            .command
            .as_ref()
            .and_then(|cb| cb.cover.as_ref())
            .map(|c| c.correlation_id.clone())
            .unwrap_or_default();
        let req = crate::proto_ext::correlated_request(request, &corr_id);
        let response = self.inner.clone().handle_sync_speculative(req).await?;
        Ok(response.into_inner())
    }
}

#[async_trait]
impl traits::GatewayClient for CommandHandlerClient {
    async fn execute(&self, command: CommandBook) -> Result<CommandResponse> {
        self.handle(command).await
    }

    async fn execute_with_sync_mode(
        &self,
        command: CommandBook,
        sync_mode: SyncMode,
    ) -> Result<CommandResponse> {
        self.handle_command(CommandRequest {
            command: Some(command),
            sync_mode: sync_mode as i32,
            cascade_error_mode: CascadeErrorMode::CascadeErrorFailFast as i32,
            cascade_id: None,
        })
        .await
    }
}

/// Per-domain client combining command execution, event querying, and speculative operations.
///
/// Connects to a single domain's endpoint and provides:
/// - Command execution via `command_handler`
/// - Event querying via `query`
/// - Speculative (what-if) execution via `speculative`
///
/// Matches the distributed architecture where each domain has its own coordinator service.
#[derive(Clone)]
pub struct DomainClient {
    /// Command handler client for command execution.
    pub command_handler: CommandHandlerClient,
    /// Query client for event retrieval.
    pub query: QueryClient,
    /// Speculative client for dry-run and what-if scenarios.
    pub speculative: SpeculativeClient,
}

impl DomainClient {
    /// Connect to a domain's coordinator at the given endpoint.
    ///
    /// Supports both TCP (host:port) and Unix Domain Sockets (file paths).
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Self::connect_with_retry(endpoint, &RetryPolicy::default()).await
    }

    /// Connect with a custom retry policy.
    pub async fn connect_with_retry(endpoint: &str, retry: &RetryPolicy) -> Result<Self> {
        let channel = create_channel(endpoint, retry).await?;
        Ok(Self::from_channel(channel))
    }

    /// Connect using an endpoint from environment variable with fallback.
    pub async fn from_env(env_var: &str, default: &str) -> Result<Self> {
        let endpoint = std::env::var(env_var).unwrap_or_else(|_| default.to_string());
        Self::connect(&endpoint).await
    }

    /// Connect to a domain's coordinator by name.
    ///
    /// Resolves `domain` to an endpoint via `resolve_ch_endpoint(domain, mode)`
    /// and connects. Pass `None` for `mode` to auto-detect from the
    /// `ANGZARR_MODE` env var.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Auto-detect from ANGZARR_MODE
    /// let player = DomainClient::for_domain("player", None).await?;
    ///
    /// // Explicit mode
    /// let player = DomainClient::for_domain("player", Some(TransportMode::Standalone)).await?;
    /// ```
    pub async fn for_domain(domain: &str, mode: Option<TransportMode>) -> Result<Self> {
        let endpoint = resolve_ch_endpoint(domain, mode, None, None, None)?;
        Self::connect(&endpoint).await
    }

    /// Create a client from an existing channel.
    pub fn from_channel(channel: Channel) -> Self {
        Self {
            command_handler: CommandHandlerClient::from_channel(channel.clone()),
            query: QueryClient::from_channel(channel.clone()),
            speculative: SpeculativeClient::from_channel(channel),
        }
    }

    /// Explicitly release this client's channel handle.
    ///
    /// Equivalent to letting the client drop; provided for symmetry with the
    /// synchronous client libraries.
    pub fn close(self) {
        drop(self);
    }

    /// Execute a command asynchronously (fire-and-forget).
    ///
    /// Use `execute_with_mode()` to specify a different sync mode.
    pub async fn execute(&self, command: CommandBook) -> Result<CommandResponse> {
        self.command_handler.handle(command).await
    }

    /// Execute a command with the specified sync mode.
    ///
    /// Use `SyncMode::Async` for fire-and-forget (default).
    /// Use `SyncMode::Simple` to wait for sync projectors.
    /// Use `SyncMode::Cascade` for full sync including saga cascade.
    pub async fn execute_with_mode(
        &self,
        command: CommandBook,
        sync_mode: SyncMode,
    ) -> Result<CommandResponse> {
        self.command_handler
            .handle_command(CommandRequest {
                command: Some(command),
                sync_mode: sync_mode as i32,
                cascade_error_mode: CascadeErrorMode::CascadeErrorFailFast as i32,
                cascade_id: None,
            })
            .await
    }

    /// Query events — unary RPC returning single EventBook (delegates to query client).
    pub async fn get_event_book(&self, query: Query) -> Result<EventBook> {
        self.query.get_event_book(query).await
    }

    /// Query events — streaming RPC returning all matching EventBooks (delegates to query client).
    pub async fn get_events(&self, query: Query) -> Result<Vec<EventBook>> {
        self.query.get_events(query).await
    }
}

#[async_trait]
impl traits::GatewayClient for DomainClient {
    async fn execute(&self, command: CommandBook) -> Result<CommandResponse> {
        self.execute(command).await
    }

    async fn execute_with_sync_mode(
        &self,
        command: CommandBook,
        sync_mode: SyncMode,
    ) -> Result<CommandResponse> {
        self.command_handler
            .execute_with_sync_mode(command, sync_mode)
            .await
    }
}

#[async_trait]
impl traits::QueryClient for DomainClient {
    async fn get_event_book(&self, query: Query) -> Result<EventBook> {
        self.get_event_book(query).await
    }

    async fn get_events(&self, query: Query) -> Result<Vec<EventBook>> {
        self.get_events(query).await
    }
}

/// Speculative client for what-if scenarios.
///
/// Provides speculative execution across different coordinator types.
/// Each method targets a specific coordinator's speculative RPC.
#[derive(Clone)]
pub struct SpeculativeClient {
    command_handler: TonicCommandHandlerClient<Channel>,
    projector: TonicProjectorClient<Channel>,
    saga: TonicSagaClient<Channel>,
    pm: TonicPmClient<Channel>,
}

impl SpeculativeClient {
    /// Connect to services at the given endpoint.
    ///
    /// Supports both TCP (host:port) and Unix Domain Sockets (file paths).
    pub async fn connect(endpoint: &str) -> Result<Self> {
        Self::connect_with_retry(endpoint, &RetryPolicy::default()).await
    }

    /// Connect with a custom retry policy.
    pub async fn connect_with_retry(endpoint: &str, retry: &RetryPolicy) -> Result<Self> {
        let channel = create_channel(endpoint, retry).await?;
        Ok(Self::from_channel(channel))
    }

    /// Connect using an endpoint from environment variable with fallback.
    pub async fn from_env(env_var: &str, default: &str) -> Result<Self> {
        let endpoint = std::env::var(env_var).unwrap_or_else(|_| default.to_string());
        Self::connect(&endpoint).await
    }

    /// Create a client from an existing channel.
    pub fn from_channel(channel: Channel) -> Self {
        Self {
            command_handler: TonicCommandHandlerClient::new(channel.clone()),
            projector: TonicProjectorClient::new(channel.clone()),
            saga: TonicSagaClient::new(channel.clone()),
            pm: TonicPmClient::new(channel),
        }
    }

    /// Explicitly release this client's channel handle.
    ///
    /// Equivalent to letting the client drop; provided for symmetry with the
    /// synchronous client libraries.
    pub fn close(self) {
        drop(self);
    }
}

#[async_trait]
impl traits::SpeculativeClient for SpeculativeClient {
    // Audit #69 stage (b): each speculative method attaches
    // `x-correlation-id` from the canonical Cover.correlation_id field
    // in its request type. Different request types nest the Cover at
    // different paths; canonical paths inlined per method.

    async fn command_handler(
        &self,
        request: SpeculateCommandHandlerRequest,
    ) -> Result<CommandResponse> {
        // SpeculateCommandHandlerRequest.command: CommandBook → Cover.
        let corr_id = request
            .command
            .as_ref()
            .and_then(|cb| cb.cover.as_ref())
            .map(|c| c.correlation_id.clone())
            .unwrap_or_default();
        let req = crate::proto_ext::correlated_request(request, &corr_id);
        let response = self
            .command_handler
            .clone()
            .handle_sync_speculative(req)
            .await?;
        Ok(response.into_inner())
    }

    async fn projector(&self, request: SpeculateProjectorRequest) -> Result<Projection> {
        // SpeculateProjectorRequest.events: EventBook → Cover.
        let corr_id = request
            .events
            .as_ref()
            .and_then(|eb| eb.cover.as_ref())
            .map(|c| c.correlation_id.clone())
            .unwrap_or_default();
        let req = crate::proto_ext::correlated_request(request, &corr_id);
        let response = self.projector.clone().handle_speculative(req).await?;
        Ok(response.into_inner())
    }

    async fn saga(&self, request: SpeculateSagaRequest) -> Result<SagaResponse> {
        // SpeculateSagaRequest.request: SagaHandleRequest, .source:
        // EventBook → Cover.
        let corr_id = request
            .request
            .as_ref()
            .and_then(|sr| sr.source.as_ref())
            .and_then(|eb| eb.cover.as_ref())
            .map(|c| c.correlation_id.clone())
            .unwrap_or_default();
        let req = crate::proto_ext::correlated_request(request, &corr_id);
        let response = self.saga.clone().execute_speculative(req).await?;
        Ok(response.into_inner())
    }

    async fn process_manager(
        &self,
        request: SpeculatePmRequest,
    ) -> Result<ProcessManagerHandleResponse> {
        // SpeculatePmRequest.request: ProcessManagerHandleRequest,
        // .trigger: EventBook → Cover.
        let corr_id = request
            .request
            .as_ref()
            .and_then(|pr| pr.trigger.as_ref())
            .and_then(|eb| eb.cover.as_ref())
            .map(|c| c.correlation_id.clone())
            .unwrap_or_default();
        let req = crate::proto_ext::correlated_request(request, &corr_id);
        let response = self.pm.clone().handle_speculative(req).await?;
        Ok(response.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_uds_path, normalize_tcp_endpoint, validate_uds_path};
    use crate::error_codes::codes;

    // Audit #39: lenient UDS prefix detection matching Python's
    // client.py::_create_channel. Each test pins one of the recognized
    // forms.

    #[test]
    fn detect_uds_absolute_path() {
        assert_eq!(
            detect_uds_path("/var/run/foo.sock").as_deref(),
            Some("/var/run/foo.sock")
        );
    }

    #[test]
    fn detect_uds_relative_path() {
        assert_eq!(
            detect_uds_path("./local.sock").as_deref(),
            Some("./local.sock")
        );
    }

    #[test]
    fn detect_uds_unix_scheme_absolute_single_slash() {
        // unix:/abs — single-slash gRPC URI form
        assert_eq!(
            detect_uds_path("unix:/var/run/foo.sock").as_deref(),
            Some("/var/run/foo.sock")
        );
    }

    #[test]
    fn detect_uds_unix_scheme_relative() {
        // unix:rel — relative gRPC URI form
        assert_eq!(
            detect_uds_path("unix:relative/foo.sock").as_deref(),
            Some("relative/foo.sock")
        );
    }

    #[test]
    fn detect_uds_unix_scheme_absolute_empty_authority() {
        // unix:///abs — triple-slash form with empty authority
        assert_eq!(
            detect_uds_path("unix:///var/run/foo.sock").as_deref(),
            Some("/var/run/foo.sock")
        );
    }

    #[test]
    fn detect_uds_tcp_endpoint_returns_none() {
        assert_eq!(detect_uds_path("localhost:50051"), None);
        assert_eq!(detect_uds_path("http://localhost:50051"), None);
        assert_eq!(detect_uds_path("[::1]:50051"), None);
    }

    #[test]
    fn normalize_tcp_endpoint_prepends_scheme_when_missing() {
        // resolve_ch_endpoint emits bare host:port in distributed
        // mode; without normalization Channel::from_shared rejects it
        // as INVALID_URI and the caller hits a non-retryable error
        // for what should be a perfectly valid endpoint.
        assert_eq!(
            normalize_tcp_endpoint("ch-player.angzarr.svc:1310"),
            "http://ch-player.angzarr.svc:1310",
        );
    }

    #[test]
    fn normalize_tcp_endpoint_preserves_explicit_scheme() {
        assert_eq!(
            normalize_tcp_endpoint("https://example.com:443"),
            "https://example.com:443",
        );
        assert_eq!(
            normalize_tcp_endpoint("http://localhost:8080"),
            "http://localhost:8080",
        );
    }

    #[test]
    fn validate_uds_path_rejects_empty_with_invalid_uri_code() {
        let err = validate_uds_path("").unwrap_err();
        assert_eq!(err.code(), codes::ENDPOINT_INVALID_URI);
    }

    #[test]
    fn validate_uds_path_rejects_nul_byte_with_invalid_uri_code() {
        let err = validate_uds_path("/var/run/has\0nul.sock").unwrap_err();
        assert_eq!(err.code(), codes::ENDPOINT_INVALID_URI);
    }

    #[test]
    fn validate_uds_path_accepts_normal_path() {
        assert!(validate_uds_path("/var/run/foo.sock").is_ok());
    }
}
