//! Fluent builders for commands and queries.

use crate::convert::{parse_timestamp, uuid_to_proto};
use crate::error::{ClientError, Result};
use crate::error_codes::{codes, messages};
use crate::proto::{
    page_header::SequenceType, query::Selection, temporal_query::PointInTime, CommandBook,
    CommandPage, CommandResponse, Cover, Edition, EventBook, EventPage, PageHeader, Query,
    SequenceRange, TemporalQuery,
};
use crate::traits;
use prost::Message;
use uuid::Uuid;

/// Builder for constructing and executing commands.
pub struct CommandBuilder<'a, C: traits::GatewayClient> {
    client: &'a C,
    domain: String,
    root: Option<Uuid>,
    correlation_id: Option<String>,
    sequence: Option<u32>,
    merge_strategy: crate::proto::MergeStrategy,
    sync_mode: Option<crate::proto::SyncMode>,
    type_url: Option<String>,
    payload: Option<Vec<u8>>,
}

impl<'a, C: traits::GatewayClient> CommandBuilder<'a, C> {
    /// Construct a CommandBuilder for an aggregate with the given `root`.
    ///
    /// Audit #67: `root` is required, no path exists to skip it. The
    /// only way to obtain an auto-generated UUID v4 is via
    /// [`CommandBuilderExt::command_new`], which materializes the UUID
    /// and passes it explicitly to this constructor. Aggregate roots
    /// are always client-assigned across all six languages (audit #20
    /// convention). The previously-existing `pub(crate) new_rootless`
    /// path was deleted as dead code on 2026-04-28.
    pub(crate) fn new(client: &'a C, domain: impl Into<String>, root: Uuid) -> Self {
        Self {
            client,
            domain: domain.into(),
            root: Some(root),
            correlation_id: None,
            sequence: None,
            merge_strategy: crate::proto::MergeStrategy::MergeCommutative,
            sync_mode: None,
            type_url: None,
            payload: None,
        }
    }

    /// Set the correlation ID for request tracing.
    /// If not set, a random UUID will be generated.
    pub fn with_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Set the expected sequence number for optimistic locking.
    pub fn with_sequence(mut self, seq: u32) -> Self {
        self.sequence = Some(seq);
        self
    }

    /// Set the merge strategy for conflict resolution.
    /// Defaults to `MergeCommutative`.
    pub fn with_merge_strategy(mut self, strategy: crate::proto::MergeStrategy) -> Self {
        self.merge_strategy = strategy;
        self
    }

    /// Set the sync mode stamped onto the built `CommandPage`'s header.
    ///
    /// When unset, [`build`](Self::build) emits a header with
    /// `sync_mode = None`, and [`execute`](Self::execute) supplies
    /// `SyncMode::Async` (the cross-language default). Calling this
    /// makes the `build()` output round-trip the choice — important
    /// for callers who hand the produced `CommandBook` to a transport
    /// helper that doesn't accept a separate `sync_mode` argument.
    pub fn with_sync_mode(mut self, mode: crate::proto::SyncMode) -> Self {
        self.sync_mode = Some(mode);
        self
    }

    /// Set the command type URL and message.
    pub fn with_command<M: Message>(mut self, type_url: impl Into<String>, message: &M) -> Self {
        self.type_url = Some(type_url.into());
        self.payload = Some(message.encode_to_vec());
        self
    }

    /// Build the CommandBook without executing.
    ///
    /// Required setters: [`with_command`](Self::with_command) (type
    /// URL + payload) and [`with_sequence`](Self::with_sequence)
    /// (optimistic-lock sequence). Without either, `build()` returns
    /// `COMMAND_*_MISSING`. `correlation_id` defaults to a fresh
    /// random UUID v4 when unset; `sync_mode` rides into the page
    /// header iff [`with_sync_mode`](Self::with_sync_mode) was called.
    /// Matches Python's `CommandBuilder.build` contract.
    pub fn build(self) -> Result<CommandBook> {
        let type_url = self.type_url.ok_or_else(|| {
            ClientError::invalid_argument(
                codes::COMMAND_TYPE_URL_MISSING,
                messages::COMMAND_TYPE_URL_MISSING,
                std::iter::empty::<(String, String)>(),
            )
        })?;
        let payload = self.payload.ok_or_else(|| {
            ClientError::invalid_argument(
                codes::COMMAND_PAYLOAD_MISSING,
                messages::COMMAND_PAYLOAD_MISSING,
                std::iter::empty::<(String, String)>(),
            )
        })?;
        let sequence = self.sequence.ok_or_else(|| {
            ClientError::invalid_argument(
                codes::COMMAND_SEQUENCE_MISSING,
                messages::COMMAND_SEQUENCE_MISSING,
                std::iter::empty::<(String, String)>(),
            )
        })?;
        let correlation_id = self
            .correlation_id
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        Ok(CommandBook {
            cover: Some(Cover {
                domain: self.domain,
                root: self.root.map(uuid_to_proto),
                correlation_id,
                edition: None,
            }),
            pages: vec![CommandPage {
                header: Some(PageHeader {
                    sequence_type: Some(SequenceType::Sequence(sequence)),
                    sync_mode: self.sync_mode.map(|m| m as i32),
                }),
                merge_strategy: self.merge_strategy as i32,
                payload: Some(crate::proto::command_page::Payload::Command(
                    prost_types::Any {
                        type_url,
                        value: payload,
                    },
                )),
            }],
        })
    }

    /// Execute the command, defaulting to `SyncMode::Async`
    /// (fire-and-forget) — the cross-language default mirroring
    /// Python's `CommandBuilder.execute(sync_mode=ASYNC)` kwarg
    /// default. Use [`Self::execute_with_mode`] or
    /// [`Self::with_sync_mode`] + `execute()` to override.
    pub async fn execute(self) -> Result<CommandResponse> {
        let mode = self
            .sync_mode
            .unwrap_or(crate::proto::SyncMode::Async);
        self.execute_with_mode(mode).await
    }

    /// Execute the command with an explicit sync mode.
    ///
    /// `SyncMode::Async` for fire-and-forget, `SyncMode::Simple` to
    /// wait for sync projectors, `SyncMode::Cascade` for full sync
    /// including saga cascade.
    pub async fn execute_with_mode(
        mut self,
        sync_mode: crate::proto::SyncMode,
    ) -> Result<CommandResponse> {
        let client = self.client;
        // Stamp on the builder so `build()` round-trips the mode into
        // the page header, then delegate to the gateway.
        self.sync_mode = Some(sync_mode);
        let command = self.build()?;
        client.execute_with_sync_mode(command, sync_mode).await
    }
}

/// Builder for constructing and executing queries.
pub struct QueryBuilder<'a, C: traits::QueryClient> {
    client: &'a C,
    domain: String,
    root: Option<Uuid>,
    correlation_id: Option<String>,
    selection: Option<Selection>,
    edition: Option<String>,
}

impl<'a, C: traits::QueryClient> QueryBuilder<'a, C> {
    pub(crate) fn new(client: &'a C, domain: impl Into<String>, root: Option<Uuid>) -> Self {
        Self {
            client,
            domain: domain.into(),
            root,
            correlation_id: None,
            selection: None,
            edition: None,
        }
    }

    /// Set the correlation ID stamped on the query's cover.
    ///
    /// Does not modify `root` — earlier versions silently nulled it,
    /// which made `client.query(d, root).by_correlation_id(c)` lose
    /// the root with no signal. Use [`QueryBuilderExt::query_domain`]
    /// for a builder that is rootless from construction.
    pub fn by_correlation_id(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Query events from a specific edition (diverged timeline).
    pub fn with_edition(mut self, edition: impl Into<String>) -> Self {
        self.edition = Some(edition.into());
        self
    }

    /// Restrict the query to a range of sequences.
    ///
    /// Accepts any standard Rust `RangeBounds<u32>` form:
    /// `range(N..)` (open upper), `range(N..=M)` (inclusive upper),
    /// `range(..M)` (no lower bound, inclusive upper), `range(..=M)`,
    /// `range(..)`. Both bounds are coerced into the
    /// inclusive-lower / inclusive-upper `SequenceRange` proto shape;
    /// an exclusive upper (`N..M`) decrements `M` by one.
    pub fn range(mut self, range: impl std::ops::RangeBounds<u32>) -> Self {
        use std::ops::Bound;
        let lower = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n.saturating_add(1),
            Bound::Unbounded => 0,
        };
        let upper = match range.end_bound() {
            Bound::Included(&n) => Some(n),
            Bound::Excluded(&n) => Some(n.saturating_sub(1)),
            Bound::Unbounded => None,
        };
        self.selection = Some(Selection::Range(SequenceRange { lower, upper }));
        self
    }

    /// Query state as of a specific sequence number.
    pub fn as_of_sequence(mut self, seq: u32) -> Self {
        self.selection = Some(Selection::Temporal(TemporalQuery {
            point_in_time: Some(PointInTime::AsOfSequence(seq)),
        }));
        self
    }

    /// Query state as of a specific timestamp (RFC3339 format).
    pub fn as_of_time(mut self, rfc3339: &str) -> Result<Self> {
        let timestamp = parse_timestamp(rfc3339)?;
        self.selection = Some(Selection::Temporal(TemporalQuery {
            point_in_time: Some(PointInTime::AsOfTime(timestamp)),
        }));
        Ok(self)
    }

    /// Build the Query without executing. Auto-generates a fresh
    /// correlation ID if one wasn't supplied — matches `CommandBuilder`
    /// so traces are joinable on the query side too.
    pub fn build(self) -> Query {
        Query {
            cover: Some(Cover {
                domain: self.domain,
                root: self.root.map(uuid_to_proto),
                correlation_id: self
                    .correlation_id
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                edition: self.edition.map(Edition::from),
            }),
            selection: self.selection,
        }
    }

    /// Execute the query and return a single EventBook (unary RPC).
    pub async fn get_event_book(self) -> Result<EventBook> {
        let client = self.client;
        client.get_event_book(self.build()).await
    }

    /// Execute the query and return all matching EventBooks (streaming RPC).
    ///
    /// Mirrors Python's `QueryBuilder.get_events` (`builder.py:235`).
    pub async fn get_events(self) -> Result<Vec<EventBook>> {
        let client = self.client;
        client.get_events(self.build()).await
    }

    /// Execute the query and return just the event pages.
    pub async fn get_pages(self) -> Result<Vec<EventPage>> {
        let event_book = self.get_event_book().await?;
        Ok(event_book.pages)
    }
}

/// Extension trait for creating command builders.
pub trait CommandBuilderExt: traits::GatewayClient + Sized {
    /// Start building a command for an existing aggregate.
    fn command(&self, domain: impl Into<String>, root: Uuid) -> CommandBuilder<'_, Self> {
        CommandBuilder::new(self, domain, root)
    }

    /// Start building a command for a new aggregate.
    ///
    /// Auto-generates a fresh UUID v4 for the aggregate root — the
    /// caller cannot reference a new aggregate without one. Mirrors
    /// Python's `command_new(client, domain)` (`builder.py:223`).
    /// Cross-language convention locked in audit P2.4a / finding #20:
    /// roots are always client-assigned.
    ///
    /// Use [`command`](Self::command) when the root is already known
    /// (existing aggregate or test fixtures).
    fn command_new(&self, domain: impl Into<String>) -> CommandBuilder<'_, Self> {
        CommandBuilder::new(self, domain, Uuid::new_v4())
    }
}

impl<T: traits::GatewayClient> CommandBuilderExt for T {}

/// Extension trait for creating query builders.
pub trait QueryBuilderExt: traits::QueryClient + Sized {
    /// Start building a query for the given domain and root.
    fn query(&self, domain: impl Into<String>, root: Uuid) -> QueryBuilder<'_, Self> {
        QueryBuilder::new(self, domain, Some(root))
    }

    /// Start building a query by domain only (use with by_correlation_id).
    fn query_domain(&self, domain: impl Into<String>) -> QueryBuilder<'_, Self> {
        QueryBuilder::new(self, domain, None)
    }
}

impl<T: traits::QueryClient> QueryBuilderExt for T {}

/// Helper to extract events from a CommandResponse.
pub fn events_from_response(response: &CommandResponse) -> &[EventPage] {
    response
        .events
        .as_ref()
        .map(|e| e.pages.as_slice())
        .unwrap_or(&[])
}

/// Helper to decode an event payload if the type URL matches.
///
/// `full_type_name` is the fully-qualified protobuf type name (e.g.
/// `"examples.OrderCreated"`, `"google.protobuf.Duration"`) — NOT a
/// suffix. The check is exact equality against
/// `TYPE_URL_PREFIX + full_type_name`, mirroring Python's
/// `helpers.decode_event` (`helpers.py:439`). Suffix matching is
/// rejected as a contract: a real `OrderCreated` and an unrelated
/// `legacy.OrderCreated` would be indistinguishable. See
/// PARITY_AUDIT.md finding #25.
pub fn decode_event<M: Message + Default>(event: &EventPage, full_type_name: &str) -> Option<M> {
    let any = match &event.payload {
        Some(crate::proto::event_page::Payload::Event(e)) => e,
        _ => return None,
    };
    if !crate::convert::type_url_matches_exact(&any.type_url, full_type_name) {
        return None;
    }
    M::decode(any.value.as_slice()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{Cover, Uuid as ProtoUuid};
    use async_trait::async_trait;

    // Mock client for testing QueryBuilder
    struct MockQueryClient {
        event_book: EventBook,
    }

    #[async_trait]
    impl traits::QueryClient for MockQueryClient {
        async fn get_event_book(&self, _query: Query) -> Result<EventBook> {
            Ok(self.event_book.clone())
        }

        async fn get_events(&self, _query: Query) -> Result<Vec<EventBook>> {
            Ok(vec![self.event_book.clone(), self.event_book.clone()])
        }
    }

    // Mock client for testing CommandBuilder
    struct MockGatewayClient {
        response: CommandResponse,
        last_sync_mode: std::sync::Mutex<Option<crate::proto::SyncMode>>,
    }

    impl MockGatewayClient {
        fn new(response: CommandResponse) -> Self {
            Self {
                response,
                last_sync_mode: std::sync::Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl traits::GatewayClient for MockGatewayClient {
        async fn execute(&self, _command: CommandBook) -> Result<CommandResponse> {
            Ok(self.response.clone())
        }

        async fn execute_with_sync_mode(
            &self,
            _command: CommandBook,
            sync_mode: crate::proto::SyncMode,
        ) -> Result<CommandResponse> {
            *self.last_sync_mode.lock().unwrap() = Some(sync_mode);
            Ok(self.response.clone())
        }
    }

    // (test fixture `make_cover` previously here was unused after
    // earlier refactors; deleted on Theme 4 cleanup.)

    // CommandBuilder tests
    #[test]
    fn test_command_builder_with_correlation_id() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let builder = CommandBuilder::new(&client, "orders", root).with_correlation_id("corr-123");

        assert_eq!(builder.correlation_id, Some("corr-123".to_string()));
    }

    #[test]
    fn test_command_builder_with_sequence() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let builder = CommandBuilder::new(&client, "orders", root).with_sequence(42);

        assert_eq!(builder.sequence, Some(42));
    }

    #[test]
    fn test_command_builder_with_command() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let msg = prost_types::Duration {
            seconds: 42,
            nanos: 0,
        };
        let builder = CommandBuilder::new(&client, "orders", root)
            .with_command("type.googleapis.com/test.Command", &msg);

        assert_eq!(
            builder.type_url,
            Some("type.googleapis.com/test.Command".to_string())
        );
        assert!(builder.payload.is_some());
    }

    #[test]
    fn test_command_builder_build_success() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let msg = prost_types::Duration {
            seconds: 42,
            nanos: 0,
        };
        let cmd = CommandBuilder::new(&client, "orders", root)
            .with_correlation_id("corr-123")
            .with_sequence(5)
            .with_command("type.googleapis.com/test.Command", &msg)
            .build()
            .unwrap();

        let cover = cmd.cover.unwrap();
        assert_eq!(cover.domain, "orders");
        assert_eq!(cover.correlation_id, "corr-123");
        assert!(cover.root.is_some());
        assert_eq!(cmd.pages.len(), 1);
        // Check sequence via header
        let header = cmd.pages[0].header.as_ref().unwrap();
        match &header.sequence_type {
            Some(SequenceType::Sequence(seq)) => assert_eq!(*seq, 5),
            _ => panic!("expected explicit sequence"),
        }
    }

    #[test]
    fn test_command_builder_build_generates_correlation_id() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let msg = prost_types::Duration {
            seconds: 42,
            nanos: 0,
        };
        let cmd = CommandBuilder::new(&client, "orders", root)
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Command", &msg)
            .build()
            .unwrap();

        let cover = cmd.cover.unwrap();
        assert!(!cover.correlation_id.is_empty());
    }

    #[test]
    fn test_command_builder_build_missing_type_url() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let result = CommandBuilder::new(&client, "orders", root)
            .with_sequence(0)
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.is_invalid_argument());
    }

    #[test]
    fn test_command_builder_build_missing_payload() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let mut builder = CommandBuilder::new(&client, "orders", root);
        builder.type_url = Some("type.googleapis.com/test".to_string());
        builder.sequence = Some(0);
        let result = builder.build();

        assert!(result.is_err());
    }

    #[test]
    fn test_command_builder_build_missing_sequence_is_invalid_argument() {
        // Sequence is required, matching Python builder.py:83-84 which
        // raises InvalidArgumentError("sequence not set (call with_sequence)").
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let msg = prost_types::Duration {
            seconds: 42,
            nanos: 0,
        };
        let result = CommandBuilder::new(&client, "orders", root)
            .with_command("type.googleapis.com/test.Command", &msg)
            .build();

        let err = result.expect_err("build should fail when sequence is unset");
        assert!(
            err.is_invalid_argument(),
            "expected InvalidArgument, got {:?}",
            err
        );
        assert!(err.to_string().contains("sequence not set"));
    }

    #[tokio::test]
    async fn test_command_builder_execute_propagates_async() {
        // P2.4b / audit finding #13: per-call sync mode reaches the
        // gateway untouched. Now uses the explicit-mode entry point
        // since `execute()` defaults to ASYNC silently.
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let msg = prost_types::Duration {
            seconds: 1,
            nanos: 0,
        };
        let _ = CommandBuilder::new(&client, "orders", root)
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Command", &msg)
            .execute_with_mode(crate::proto::SyncMode::Async)
            .await;
        assert_eq!(
            *client.last_sync_mode.lock().unwrap(),
            Some(crate::proto::SyncMode::Async)
        );
    }

    #[tokio::test]
    async fn test_command_builder_execute_propagates_cascade() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let msg = prost_types::Duration {
            seconds: 1,
            nanos: 0,
        };
        let _ = CommandBuilder::new(&client, "orders", root)
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Command", &msg)
            .execute_with_mode(crate::proto::SyncMode::Cascade)
            .await;
        assert_eq!(
            *client.last_sync_mode.lock().unwrap(),
            Some(crate::proto::SyncMode::Cascade)
        );
    }

    #[tokio::test]
    async fn test_command_new_auto_generates_uuid_v4_root() {
        // P2.4a / finding #20 closed: command_new auto-generates a
        // UUID v4 client-side. Aggregate roots are always
        // client-assigned across all six languages.
        let client = MockGatewayClient::new(CommandResponse::default());
        let msg = prost_types::Duration {
            seconds: 1,
            nanos: 0,
        };
        let book = client
            .command_new("orders")
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Cmd", &msg)
            .build()
            .expect("build should succeed — command_new auto-fills root");
        let cover = book.cover.expect("cover must be set");
        let root = cover.root.expect("root must be auto-generated");
        assert_eq!(root.value.len(), 16, "UUID v4 must be 16 bytes");
        let bytes: [u8; 16] = root.value.as_slice().try_into().unwrap();
        let parsed = Uuid::from_bytes(bytes);
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[tokio::test]
    async fn test_command_new_each_call_yields_independent_root() {
        // command_new called twice must produce two distinct UUIDs —
        // mirrors Python's `uuid4()` behavior where each call gets a
        // fresh random UUID.
        let client = MockGatewayClient::new(CommandResponse::default());
        let msg = prost_types::Duration {
            seconds: 1,
            nanos: 0,
        };
        let a = client
            .command_new("orders")
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Cmd", &msg)
            .build()
            .unwrap();
        let b = client
            .command_new("orders")
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Cmd", &msg)
            .build()
            .unwrap();
        assert_ne!(
            a.cover.unwrap().root.unwrap().value,
            b.cover.unwrap().root.unwrap().value,
            "each command_new call must yield a fresh UUID"
        );
    }

    #[tokio::test]
    async fn test_command_builder_execute_defaults_to_async() {
        // Cross-language parity: when execute() is called without a
        // mode, the gateway sees SyncMode::Async (Python's
        // CommandBuilder.execute() kwarg default).
        let client = MockGatewayClient::new(CommandResponse::default());
        let msg = prost_types::Duration {
            seconds: 1,
            nanos: 0,
        };
        let _ = client
            .command_new("orders")
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Cmd", &msg)
            .execute()
            .await
            .expect("execute should succeed");
        assert_eq!(
            *client.last_sync_mode.lock().unwrap(),
            Some(crate::proto::SyncMode::Async),
        );
    }

    #[tokio::test]
    async fn test_command_builder_execute_with_mode_overrides_default() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let msg = prost_types::Duration {
            seconds: 1,
            nanos: 0,
        };
        let _ = client
            .command_new("orders")
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Cmd", &msg)
            .execute_with_mode(crate::proto::SyncMode::Cascade)
            .await
            .expect("execute_with_mode should succeed");
        assert_eq!(
            *client.last_sync_mode.lock().unwrap(),
            Some(crate::proto::SyncMode::Cascade),
        );
    }

    #[test]
    fn test_command_builder_with_sync_mode_stamps_into_page_header() {
        // Build path must round-trip the mode into PageHeader.sync_mode
        // — the previous behavior dropped it on the floor for callers
        // that hand the built CommandBook to a transport helper that
        // doesn't accept a separate sync_mode argument.
        let client = MockGatewayClient::new(CommandResponse::default());
        let msg = prost_types::Duration {
            seconds: 1,
            nanos: 0,
        };
        let book = CommandBuilder::new(&client, "orders", Uuid::new_v4())
            .with_sequence(0)
            .with_sync_mode(crate::proto::SyncMode::Cascade)
            .with_command("type.googleapis.com/test.Cmd", &msg)
            .build()
            .expect("build should succeed");
        let header = book.pages[0].header.as_ref().expect("page must have header");
        assert_eq!(header.sync_mode, Some(crate::proto::SyncMode::Cascade as i32));
    }

    #[test]
    fn test_command_builder_build_omits_sync_mode_when_unset() {
        // Without with_sync_mode, the page header stays sync_mode=None
        // — matches the previous default and lets execute() supply the
        // ASYNC default at dispatch time.
        let client = MockGatewayClient::new(CommandResponse::default());
        let msg = prost_types::Duration {
            seconds: 1,
            nanos: 0,
        };
        let book = CommandBuilder::new(&client, "orders", Uuid::new_v4())
            .with_sequence(0)
            .with_command("type.googleapis.com/test.Cmd", &msg)
            .build()
            .expect("build should succeed");
        assert!(book.pages[0].header.as_ref().unwrap().sync_mode.is_none());
    }

    // QueryBuilder tests
    #[test]
    fn test_query_builder_by_correlation_id() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let root = Uuid::new_v4();
        let builder =
            QueryBuilder::new(&client, "orders", Some(root)).by_correlation_id("corr-123");

        // by_correlation_id no longer silently nulls root — it just
        // sets the correlation field. Callers wanting a rootless
        // builder use QueryBuilderExt::query_domain.
        assert_eq!(builder.correlation_id, Some("corr-123".to_string()));
        assert_eq!(builder.root, Some(root));
    }

    #[test]
    fn test_query_builder_edition() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let builder = QueryBuilder::new(&client, "orders", None).with_edition("test-edition");

        assert_eq!(builder.edition, Some("test-edition".to_string()));
    }

    #[test]
    fn test_query_builder_range_open_upper() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let builder = QueryBuilder::new(&client, "orders", None).range(10..);

        match builder.selection {
            Some(Selection::Range(r)) => {
                assert_eq!(r.lower, 10);
                assert!(r.upper.is_none());
            }
            _ => panic!("expected Range selection"),
        }
    }

    #[test]
    fn test_query_builder_range_inclusive_upper() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let builder = QueryBuilder::new(&client, "orders", None).range(5..=15);

        match builder.selection {
            Some(Selection::Range(r)) => {
                assert_eq!(r.lower, 5);
                assert_eq!(r.upper, Some(15));
            }
            _ => panic!("expected Range selection"),
        }
    }

    #[test]
    fn test_query_builder_range_exclusive_upper_decrements() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        // 5..15 is exclusive of 15 — coerce to inclusive 14.
        let builder = QueryBuilder::new(&client, "orders", None).range(5..15);

        match builder.selection {
            Some(Selection::Range(r)) => {
                assert_eq!(r.lower, 5);
                assert_eq!(r.upper, Some(14));
            }
            _ => panic!("expected Range selection"),
        }
    }

    #[test]
    fn test_query_builder_range_unbounded_lower() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let builder = QueryBuilder::new(&client, "orders", None).range(..=20);

        match builder.selection {
            Some(Selection::Range(r)) => {
                assert_eq!(r.lower, 0);
                assert_eq!(r.upper, Some(20));
            }
            _ => panic!("expected Range selection"),
        }
    }

    #[test]
    fn test_query_builder_as_of_sequence() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let builder = QueryBuilder::new(&client, "orders", None).as_of_sequence(42);

        match builder.selection {
            Some(Selection::Temporal(t)) => match t.point_in_time {
                Some(PointInTime::AsOfSequence(s)) => assert_eq!(s, 42),
                _ => panic!("expected AsOfSequence"),
            },
            _ => panic!("expected Temporal selection"),
        }
    }

    #[test]
    fn test_query_builder_as_of_time_valid() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let builder = QueryBuilder::new(&client, "orders", None)
            .as_of_time("2024-01-15T10:30:00Z")
            .unwrap();

        match builder.selection {
            Some(Selection::Temporal(t)) => match t.point_in_time {
                Some(PointInTime::AsOfTime(ts)) => assert_eq!(ts.seconds, 1705314600),
                _ => panic!("expected AsOfTime"),
            },
            _ => panic!("expected Temporal selection"),
        }
    }

    #[test]
    fn test_query_builder_as_of_time_invalid() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let result = QueryBuilder::new(&client, "orders", None).as_of_time("not a timestamp");

        assert!(result.is_err());
    }

    #[test]
    fn test_query_builder_build() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let root = Uuid::new_v4();
        let query = QueryBuilder::new(&client, "orders", Some(root))
            .with_edition("test-edition")
            .range(10..)
            .build();

        let cover = query.cover.unwrap();
        assert_eq!(cover.domain, "orders");
        assert!(cover.root.is_some());
        assert!(cover.edition.is_some());
        assert!(query.selection.is_some());
    }

    #[test]
    fn test_query_builder_build_with_correlation_id() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let query = QueryBuilder::new(&client, "orders", None)
            .by_correlation_id("corr-123")
            .build();

        let cover = query.cover.unwrap();
        assert_eq!(cover.correlation_id, "corr-123");
        assert!(cover.root.is_none());
    }

    #[test]
    fn test_query_builder_build_auto_generates_correlation_id() {
        // When no correlation_id is supplied, build() generates a
        // fresh UUID v4 — matches CommandBuilder behavior so query
        // traces remain joinable to their command counterparts.
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let query = QueryBuilder::new(&client, "orders", Some(Uuid::new_v4())).build();
        let cover = query.cover.unwrap();
        // RFC 4122 v4 format: 36 chars, with dashes, version nibble = 4.
        assert_eq!(cover.correlation_id.len(), 36);
        assert_eq!(cover.correlation_id.chars().nth(14), Some('4'));
    }

    // Helper function tests
    #[test]
    fn test_events_from_response_with_events() {
        let events = EventBook {
            cover: None,
            pages: vec![EventPage::default(), EventPage::default()],
            snapshot: None,
            next_sequence: 0,
        };
        let response = CommandResponse {
            events: Some(events),
            ..Default::default()
        };

        let pages = events_from_response(&response);
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn test_events_from_response_no_events() {
        let response = CommandResponse {
            events: None,
            ..Default::default()
        };

        let pages = events_from_response(&response);
        assert!(pages.is_empty());
    }

    #[test]
    fn test_decode_event_success() {
        use crate::proto::event_page::Payload;

        // Use prost_types::Duration which implements Message + Default
        let msg = prost_types::Duration {
            seconds: 42,
            nanos: 0,
        };
        let event = EventPage {
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::Sequence(1)),
                sync_mode: None,
            }),
            created_at: None,
            payload: Some(Payload::Event(prost_types::Any {
                type_url: "type.googleapis.com/google.protobuf.Duration".to_string(),
                value: msg.encode_to_vec(),
            })),
            cascade_id: None,
            no_commit: false,
        };

        let decoded: Option<prost_types::Duration> =
            decode_event(&event, "google.protobuf.Duration");
        assert!(decoded.is_some());
        assert_eq!(decoded.unwrap().seconds, 42);
    }

    #[test]
    fn test_decode_event_type_mismatch() {
        use crate::proto::event_page::Payload;

        let msg = prost_types::Duration {
            seconds: 42,
            nanos: 0,
        };
        let event = EventPage {
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::Sequence(1)),
                sync_mode: None,
            }),
            created_at: None,
            payload: Some(Payload::Event(prost_types::Any {
                type_url: "type.googleapis.com/google.protobuf.Duration".to_string(),
                value: msg.encode_to_vec(),
            })),
            cascade_id: None,
            no_commit: false,
        };

        let decoded: Option<prost_types::Duration> =
            decode_event(&event, "google.protobuf.Timestamp");
        assert!(decoded.is_none());
    }

    #[test]
    fn test_decode_event_nil_event() {
        let event = EventPage {
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::Sequence(1)),
                sync_mode: None,
            }),
            created_at: None,
            payload: None,
            cascade_id: None,
            no_commit: false,
        };

        let decoded: Option<prost_types::Duration> =
            decode_event(&event, "google.protobuf.Duration");
        assert!(decoded.is_none(), "garbage bytes must not decode");
    }

    #[test]
    fn test_decode_event_invalid_payload() {
        use crate::proto::event_page::Payload;

        let event = EventPage {
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::Sequence(1)),
                sync_mode: None,
            }),
            created_at: None,
            payload: Some(Payload::Event(prost_types::Any {
                type_url: "type.googleapis.com/google.protobuf.Duration".to_string(),
                value: vec![0xFF, 0xFF, 0xFF], // garbage
            })),
            cascade_id: None,
            no_commit: false,
        };

        let decoded: Option<prost_types::Duration> =
            decode_event(&event, "google.protobuf.Duration");
        assert!(decoded.is_none());
    }

    // Extension trait tests
    #[test]
    fn test_command_builder_ext_command() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let builder = client.command("orders", root);

        assert_eq!(builder.domain, "orders");
        assert_eq!(builder.root, Some(root));
    }

    #[test]
    fn test_command_builder_with_merge_strategy() {
        let client = MockGatewayClient::new(CommandResponse::default());
        let root = Uuid::new_v4();
        let builder = CommandBuilder::new(&client, "orders", root)
            .with_merge_strategy(crate::proto::MergeStrategy::MergeStrict);

        assert_eq!(
            builder.merge_strategy,
            crate::proto::MergeStrategy::MergeStrict
        );
    }

    #[tokio::test]
    async fn test_query_builder_get_events_returns_streaming_results() {
        // Mirrors Python's `QueryBuilder.get_events` which calls the
        // streaming `GetEvents` RPC and returns `list[EventBook]`.
        let client = MockQueryClient {
            event_book: EventBook {
                next_sequence: 7,
                ..Default::default()
            },
        };
        let books = QueryBuilder::new(&client, "orders", None)
            .range(0..)
            .get_events()
            .await
            .unwrap();
        assert_eq!(books.len(), 2);
        assert_eq!(books[0].next_sequence, 7);
    }

    #[test]
    fn test_query_builder_ext_query() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let root = Uuid::new_v4();
        let builder = client.query("orders", root);

        assert_eq!(builder.domain, "orders");
        assert_eq!(builder.root, Some(root));
    }

    #[test]
    fn test_query_builder_ext_query_domain() {
        let client = MockQueryClient {
            event_book: EventBook::default(),
        };
        let builder = client.query_domain("orders");

        assert_eq!(builder.domain, "orders");
        assert!(builder.root.is_none());
    }
}
