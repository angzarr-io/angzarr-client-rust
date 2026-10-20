//! Fluent builders for commands and queries.

use crate::convert::{parse_timestamp, uuid_to_proto};
use crate::error::{ClientError, Result};
use crate::error_codes::{codes, messages};
use crate::proto::{
    command_page, query::Selection, temporal_query::PointInTime, Command, CommandBook, CommandPage,
    CommandResponse, ConflictResolution, Cover, EventBook, EventPage, Handling, Query,
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
    correlation_id: Option<Uuid>,
    expected_sequence: Option<u64>,
    conflict_resolution: ConflictResolution,
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
            expected_sequence: None,
            conflict_resolution: ConflictResolution::Unspecified,
            type_url: None,
            payload: None,
        }
    }

    /// Set the correlation ID for request tracing
    /// (`CommandPage.correlation_id`). If not set, none is sent and the
    /// coordinator assigns one.
    pub fn with_correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self
    }

    /// Set the expected sequence for optimistic concurrency
    /// (`Handling.expected_sequence`). If not set, none is sent and the
    /// coordinator stamps the head it decides against.
    pub fn with_expected_sequence(mut self, seq: u64) -> Self {
        self.expected_sequence = Some(seq);
        self
    }

    /// Set the conflict resolution for a stale expected sequence
    /// (`Handling.conflict_resolution`). Defaults to `Unspecified`: the
    /// configured default applies.
    pub fn with_conflict_resolution(mut self, resolution: ConflictResolution) -> Self {
        self.conflict_resolution = resolution;
        self
    }

    /// Set the command type URL and message.
    pub fn with_command<M: Message>(mut self, type_url: impl Into<String>, message: &M) -> Self {
        self.type_url = Some(type_url.into());
        self.payload = Some(message.encode_to_vec());
        self
    }

    /// Set the command type URL alone; pair with
    /// [`with_payload`](Self::with_payload).
    pub fn with_type_url(mut self, type_url: impl Into<String>) -> Self {
        self.type_url = Some(type_url.into());
        self
    }

    /// Set the already-encoded command payload; pair with
    /// [`with_type_url`](Self::with_type_url).
    pub fn with_payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Build the CommandBook without executing.
    ///
    /// Requires a type URL and payload ([`with_command`](Self::with_command),
    /// or [`with_type_url`](Self::with_type_url) plus
    /// [`with_payload`](Self::with_payload)); without either `build()`
    /// returns `COMMAND_TYPE_URL_MISSING` / `COMMAND_PAYLOAD_MISSING`.
    /// Only what was set is sent: the book carries `Handling` when an
    /// expected sequence or a conflict resolution was set, and the page a
    /// correlation id when one was.
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
        let handling = (self.expected_sequence.is_some()
            || self.conflict_resolution != ConflictResolution::Unspecified)
            .then_some(Handling {
                expected_sequence: self.expected_sequence,
                conflict_resolution: self.conflict_resolution as i32,
            });

        Ok(CommandBook {
            cover: Some(Cover {
                domain: self.domain,
                root: self.root.map(uuid_to_proto),
                ..Default::default()
            }),
            page: Some(CommandPage {
                body: Some(command_page::Body::Command(Command {
                    payload: Some(prost_types::Any {
                        type_url,
                        value: payload,
                    }),
                })),
                correlation_id: self.correlation_id.map(uuid_to_proto),
                ..Default::default()
            }),
            handling,
        })
    }

    /// Build the command and send it ASYNC (`HandleAsync`): accepted for
    /// processing, no decision yet.
    pub async fn execute_async(self) -> Result<()> {
        let client = self.client;
        client.handle_async(self.build()?).await
    }

    /// Build the command and send it for the coordinator's decision
    /// (DECISION, `Handle`).
    pub async fn execute(self) -> Result<CommandResponse> {
        let client = self.client;
        client.handle(self.build()?).await
    }

    /// Build the command and send it SIMPLE (`HandleSync`): the decision
    /// after the synchronous projectors ran.
    pub async fn execute_sync(self) -> Result<CommandResponse> {
        let client = self.client;
        client.handle_sync(self.build()?).await
    }
}

/// Builder for constructing and executing queries.
pub struct QueryBuilder<'a, C: traits::QueryClient> {
    client: &'a C,
    domain: String,
    root: Option<Uuid>,
    correlation_id: Option<Uuid>,
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

    /// Select only events of this correlation (`Query.correlation_id`),
    /// and clear `root` — correlation-ID queries select across roots in
    /// the domain. Mirrors Python's `builder.py:178` (`self._root = None`).
    pub fn by_correlation_id(mut self, id: Uuid) -> Self {
        self.correlation_id = Some(id);
        self.root = None;
        self
    }

    /// Query events from a specific edition (diverged timeline).
    pub fn with_edition(mut self, edition: impl Into<String>) -> Self {
        self.edition = Some(edition.into());
        self
    }

    /// Restrict the query to a range of sequences.
    ///
    /// Accepts any standard Rust `RangeBounds<u64>` form:
    /// `range(N..)` (open upper), `range(N..=M)` (inclusive upper),
    /// `range(..M)` (no lower bound, inclusive upper), `range(..=M)`,
    /// `range(..)`. Both bounds are coerced into the
    /// inclusive-lower / inclusive-upper `SequenceRange` proto shape;
    /// an exclusive upper (`N..M`) decrements `M` by one.
    pub fn range(mut self, range: impl std::ops::RangeBounds<u64>) -> Self {
        use std::ops::Bound;
        let lower = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n.saturating_add(1),
            Bound::Unbounded => 0,
        };
        // The wire range is `[lower, upper]` with an inclusive upper. An
        // exclusive end at or below `lower` selects nothing, which is
        // encoded as `upper < lower` (`..0` needs `lower` raised to 1,
        // since no upper is below 0).
        let (lower, upper) = match range.end_bound() {
            Bound::Included(&n) => (lower, Some(n)),
            Bound::Excluded(&0) => (lower.max(1), Some(0)),
            Bound::Excluded(&n) => (lower, Some(n - 1)),
            Bound::Unbounded => (lower, None),
        };
        self.selection = Some(Selection::Range(SequenceRange { lower, upper }));
        self
    }

    /// Query state as of a specific sequence number.
    pub fn as_of_sequence(mut self, seq: u64) -> Self {
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

    /// Build the Query without executing.
    pub fn build(self) -> Query {
        Query {
            cover: Some(Cover {
                domain: self.domain,
                root: self.root.map(uuid_to_proto),
                edition: self.edition,
            }),
            selection: self.selection,
            correlation_id: self.correlation_id.map(uuid_to_proto),
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
/// suffix. The type URL may carry any prefix; the name after its last
/// `/` must equal `full_type_name` exactly, so a real `OrderCreated` and
/// an unrelated `legacy.OrderCreated` stay distinct.
pub fn decode_event<M: Message + Default>(event: &EventPage, full_type_name: &str) -> Option<M> {
    let any = event.payload.as_ref()?;
    if !crate::convert::type_url_matches_exact(&any.type_url, full_type_name) {
        return None;
    }
    M::decode(any.value.as_slice()).ok()
}

#[cfg(test)]
#[path = "builder.test.rs"]
mod tests;
