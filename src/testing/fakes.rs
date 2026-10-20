//! Recording fakes of the client traits.
//!
//! One struct per client trait ([`GatewayClient`], [`QueryClient`])
//! standing in for the coordinator: each records its
//! calls (`call_count`, `last_call`) and answers with canned responses or
//! errors. Mirrors Python's `RecordingStub`.

use std::sync::Mutex;

use crate::error::{ClientError, Result};
use crate::proto::{CommandBook, CommandResponse, EventBook, Query};
use crate::traits::{GatewayClient, QueryClient};
use async_trait::async_trait;
use tonic::{Code, Status};

// ---------------------------------------------------------------------------
// StubRpcError analog
// ---------------------------------------------------------------------------

/// A [`ClientError::Grpc`] wrapping a `tonic::Status` with the given code
/// and details — the error a real client returns for that status, so error
/// classification (`is_not_found`, …) behaves end to end. Mirrors Python's
/// `StubRpcError`.
pub fn stub_rpc_error(code: Code, details: &str) -> ClientError {
    Status::new(code, details.to_string()).into()
}

// ---------------------------------------------------------------------------
// RecordingGatewayClient
// ---------------------------------------------------------------------------

/// Recorded call to a [`GatewayClient`] method.
#[derive(Debug, Clone)]
pub struct GatewayCall {
    /// Trait method name.
    pub method: &'static str,
    /// The command sent.
    pub command: CommandBook,
}

/// [`GatewayClient`] fake: records each command and answers with a canned
/// [`CommandResponse`] (default when unset) or the configured error.
#[derive(Debug, Default)]
pub struct RecordingGatewayClient {
    calls: Mutex<Vec<GatewayCall>>,
    /// Canned `execute` response. Returned (cloned)
    /// for every call until overwritten. Unset → `Ok(CommandResponse::default())`.
    response: Mutex<Option<CommandResponse>>,
    /// If set, every call returns `Err(error.clone())` instead of `response`.
    error: Mutex<Option<ClientError>>,
}

impl RecordingGatewayClient {
    /// A fake with no calls, answering `CommandResponse::default()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer every call with `resp`.
    pub fn with_response(self, resp: CommandResponse) -> Self {
        *self.response.lock().unwrap() = Some(resp);
        self
    }

    /// Answer every subsequent call with `resp`.
    pub fn respond(&self, resp: CommandResponse) {
        *self.response.lock().unwrap() = Some(resp);
    }

    /// Fail every subsequent call with `err`.
    pub fn fail_with(&self, err: ClientError) {
        *self.error.lock().unwrap() = Some(err);
    }

    /// All recorded calls, in order.
    pub fn calls(&self) -> Vec<GatewayCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Number of recorded calls to `method`.
    pub fn call_count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method == method)
            .count()
    }

    /// Most recent call to `method`.
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
        });
        self.produce()
    }
}

// ---------------------------------------------------------------------------
// RecordingQueryClient
// ---------------------------------------------------------------------------

/// Recorded call to a [`QueryClient`] method.
#[derive(Debug, Clone)]
pub struct QueryCall {
    /// Trait method name (`get_event_book` or `get_events`).
    pub method: &'static str,
    /// The query sent.
    pub query: Query,
}

/// [`QueryClient`] fake: records each query and answers with a canned
/// [`EventBook`] (default when unset), a canned stream, or the configured
/// error.
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
    /// A fake with no calls, answering `EventBook::default()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer `get_event_book` (and `get_events`, as one book) with `book`.
    pub fn with_event_book(self, book: EventBook) -> Self {
        *self.event_book.lock().unwrap() = Some(book);
        self
    }

    /// Answer subsequent `get_event_book` (and `get_events`, as one book)
    /// calls with `book`.
    pub fn respond_event_book(&self, book: EventBook) {
        *self.event_book.lock().unwrap() = Some(book);
    }

    /// Answer subsequent `get_events` calls with `books`.
    pub fn respond_events_stream(&self, books: Vec<EventBook>) {
        *self.events_stream.lock().unwrap() = Some(books);
    }

    /// Fail every subsequent call with `err`.
    pub fn fail_with(&self, err: ClientError) {
        *self.error.lock().unwrap() = Some(err);
    }

    /// All recorded calls, in order.
    pub fn calls(&self) -> Vec<QueryCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Number of recorded calls to `method`.
    /// Number of recorded calls to `method`.
    pub fn call_count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.method == method)
            .count()
    }

    /// Most recent call to `method`.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn book(domain: &str) -> EventBook {
        EventBook {
            cover: Some(crate::proto::Cover {
                domain: domain.into(),
                ..Default::default()
            }),
            next_sequence: 7,
            ..Default::default()
        }
    }

    fn command(domain: &str) -> CommandBook {
        CommandBook {
            cover: book(domain).cover,
            ..Default::default()
        }
    }

    fn query(domain: &str) -> Query {
        Query {
            cover: book(domain).cover,
            ..Default::default()
        }
    }

    #[test]
    fn stub_rpc_error_is_a_grpc_status_with_code_and_details() {
        let err = stub_rpc_error(Code::NotFound, "gone");
        assert!(err.is_not_found());
        match err {
            ClientError::Grpc(s) => {
                assert_eq!(s.code(), Code::NotFound);
                assert_eq!(s.message(), "gone");
            }
            other => panic!("expected Grpc, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn gateway_fake_records_calls_and_answers_canned_response() {
        let canned = CommandResponse {
            events: Some(book("orders")),
            ..Default::default()
        };
        let fake = RecordingGatewayClient::new().with_response(canned.clone());
        assert_eq!(fake.execute(command("a")).await.unwrap(), canned);
        fake.execute(command("b")).await.unwrap();
        assert_eq!(fake.call_count("execute"), 2);
        assert_eq!(fake.call_count("other"), 0);
        assert_eq!(fake.calls().len(), 2);
        assert_eq!(fake.calls()[0].command, command("a"));
        let last = fake.last_call("execute").unwrap();
        assert_eq!(last.command, command("b"));
        assert!(fake.last_call("other").is_none());

        let replaced = CommandResponse::default();
        fake.respond(replaced.clone());
        assert_eq!(fake.execute(command("c")).await.unwrap(), replaced);
    }

    #[tokio::test]
    async fn gateway_fake_fails_every_call_once_told_to() {
        let fake = RecordingGatewayClient::new();
        assert_eq!(
            fake.execute(command("a")).await.unwrap(),
            CommandResponse::default()
        );
        fake.fail_with(stub_rpc_error(Code::FailedPrecondition, "stale"));
        for _ in 0..2 {
            let err = fake.execute(command("a")).await.unwrap_err();
            assert!(err.is_precondition_failed());
        }
        assert_eq!(fake.call_count("execute"), 3);
    }

    #[tokio::test]
    async fn query_fake_records_calls_and_answers_canned_books() {
        let fake = RecordingQueryClient::new().with_event_book(book("orders"));
        assert_eq!(
            fake.get_event_book(query("q1")).await.unwrap(),
            book("orders")
        );
        assert_eq!(
            fake.get_events(query("q2")).await.unwrap(),
            vec![book("orders")]
        );
        fake.respond_events_stream(vec![book("a"), book("b")]);
        assert_eq!(
            fake.get_events(query("q3")).await.unwrap(),
            vec![book("a"), book("b")]
        );
        fake.respond_event_book(book("carts"));
        assert_eq!(
            fake.get_event_book(query("q4")).await.unwrap(),
            book("carts")
        );

        fake.get_events(query("q5")).await.unwrap();
        assert_eq!(fake.call_count("get_event_book"), 2);
        assert_eq!(fake.call_count("get_events"), 3);
        assert_eq!(fake.calls().len(), 5);
        assert_eq!(fake.last_call("get_events").unwrap().query, query("q5"));
        assert_eq!(fake.last_call("get_event_book").unwrap().query, query("q4"));
        assert!(fake.last_call("other").is_none());
    }

    #[tokio::test]
    async fn query_fake_defaults_and_fails() {
        let fake = RecordingQueryClient::new();
        assert_eq!(
            fake.get_events(query("q")).await.unwrap(),
            vec![EventBook::default()]
        );
        fake.fail_with(stub_rpc_error(Code::NotFound, "no"));
        assert!(fake
            .get_event_book(query("q"))
            .await
            .unwrap_err()
            .is_not_found());
        assert!(fake
            .get_events(query("q"))
            .await
            .unwrap_err()
            .is_not_found());
    }

    #[test]
    fn replayed_errors_keep_their_classification() {
        let rejected = crate::CommandRejectedError::precondition_failed(
            "NOPE",
            "nope",
            std::iter::empty::<(String, String)>(),
        );
        for err in [
            stub_rpc_error(Code::Unavailable, "down"),
            ClientError::Rejected(rejected),
        ] {
            let copy = clone_err(&err);
            assert_eq!(copy.to_string(), err.to_string());
            assert_eq!(copy.is_connection_error(), err.is_connection_error());
        }
    }
}
