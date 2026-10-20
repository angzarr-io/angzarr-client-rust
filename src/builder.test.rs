use super::*;
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

    async fn get_correlated_events(
        &self,
        _correlation_id: Uuid,
        _edition: Option<String>,
    ) -> Result<Vec<EventBook>> {
        Ok(vec![self.event_book.clone()])
    }
}

// Mock client for testing CommandBuilder: records the last command sent.
struct MockGatewayClient {
    response: CommandResponse,
    last_command: std::sync::Mutex<Option<CommandBook>>,
}

impl MockGatewayClient {
    fn new(response: CommandResponse) -> Self {
        Self {
            response,
            last_command: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait]
impl traits::GatewayClient for MockGatewayClient {
    async fn handle_async(&self, command: CommandBook) -> Result<()> {
        *self.last_command.lock().unwrap() = Some(command);
        Ok(())
    }

    async fn handle(&self, command: CommandBook) -> Result<CommandResponse> {
        *self.last_command.lock().unwrap() = Some(command);
        Ok(self.response.clone())
    }

    async fn handle_sync(&self, command: CommandBook) -> Result<CommandResponse> {
        *self.last_command.lock().unwrap() = Some(command);
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
    let corr = Uuid::new_v4();
    let builder = CommandBuilder::new(&client, "orders", root).with_correlation_id(corr);

    assert_eq!(builder.correlation_id, Some(corr));
}

#[test]
fn test_command_builder_with_expected_sequence() {
    let client = MockGatewayClient::new(CommandResponse::default());
    let root = Uuid::new_v4();
    let builder = CommandBuilder::new(&client, "orders", root).with_expected_sequence(42);

    assert_eq!(builder.expected_sequence, Some(42));
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
    let corr = Uuid::new_v4();
    let msg = prost_types::Duration {
        seconds: 42,
        nanos: 0,
    };
    let cmd = CommandBuilder::new(&client, "orders", root)
        .with_correlation_id(corr)
        .with_expected_sequence(5)
        .with_command("type.googleapis.com/test.Command", &msg)
        .build()
        .unwrap();

    let cover = cmd.cover.unwrap();
    assert_eq!(cover.domain, "orders");
    assert_eq!(cover.root, Some(uuid_to_proto(root)));
    let page = cmd.page.expect("one command page");
    assert_eq!(page.correlation_id, Some(uuid_to_proto(corr)));
    assert_eq!(
        angzarr_router::command_payload(&page).map(|a| a.type_url.as_str()),
        Some("type.googleapis.com/test.Command")
    );
    let handling = cmd.handling.expect("handling");
    assert_eq!(handling.expected_sequence, Some(5));
    assert_eq!(
        handling.conflict_resolution(),
        ConflictResolution::Unspecified
    );
}

#[test]
fn test_command_builder_build_sends_only_what_was_set() {
    // No correlation id and no Handling unless set: the coordinator
    // assigns the one and stamps the head for the other.
    let client = MockGatewayClient::new(CommandResponse::default());
    let msg = prost_types::Duration {
        seconds: 42,
        nanos: 0,
    };
    let cmd = CommandBuilder::new(&client, "orders", Uuid::new_v4())
        .with_command("type.googleapis.com/test.Command", &msg)
        .build()
        .unwrap();

    assert_eq!(cmd.page.expect("page").correlation_id, None);
    assert_eq!(cmd.handling, None);
}

#[test]
fn test_command_builder_build_missing_type_url() {
    let client = MockGatewayClient::new(CommandResponse::default());
    let root = Uuid::new_v4();
    let result = CommandBuilder::new(&client, "orders", root)
        .with_expected_sequence(0)
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
    builder.expected_sequence = Some(0);
    let result = builder.build();

    assert!(result.is_err());
}

#[test]
fn test_command_builder_conflict_resolution_rides_in_handling() {
    let client = MockGatewayClient::new(CommandResponse::default());
    let msg = prost_types::Duration {
        seconds: 42,
        nanos: 0,
    };
    let book = CommandBuilder::new(&client, "orders", Uuid::new_v4())
        .with_conflict_resolution(ConflictResolution::AggregateHandles)
        .with_command("type.googleapis.com/test.Command", &msg)
        .build()
        .unwrap();
    let handling = book.handling.expect("a resolution sets handling");
    assert_eq!(handling.expected_sequence, None);
    assert_eq!(
        handling.conflict_resolution(),
        ConflictResolution::AggregateHandles
    );
}

#[test]
fn test_command_builder_type_url_without_payload_is_payload_missing() {
    let client = MockGatewayClient::new(CommandResponse::default());
    let err = CommandBuilder::new(&client, "orders", Uuid::new_v4())
        .with_type_url("type.googleapis.com/test.Command")
        .build()
        .expect_err("payload is required");
    assert_eq!(err.code(), codes::COMMAND_PAYLOAD_MISSING);
}

#[test]
fn test_command_builder_type_url_and_payload_set_separately() {
    let client = MockGatewayClient::new(CommandResponse::default());
    let book = CommandBuilder::new(&client, "orders", Uuid::new_v4())
        .with_payload(vec![1, 2, 3])
        .with_type_url("type.googleapis.com/test.Command")
        .build()
        .expect("type URL and payload set");
    let page = book.page.expect("one command page");
    let any = angzarr_router::command_payload(&page).expect("a command payload");
    assert_eq!(any.type_url, "type.googleapis.com/test.Command");
    assert_eq!(any.value, vec![1, 2, 3]);
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
        .with_expected_sequence(0)
        .with_command("type.googleapis.com/test.Cmd", &msg)
        .build()
        .expect("build should succeed — command_new auto-fills root");
    let cover = book.cover.expect("cover must be set");
    let root = cover.root.expect("root must be auto-generated");
    assert_eq!(root.value.len(), 16, "UUID v4 must be 16 bytes");
    let bytes: [u8; 16] = root.value.as_ref().try_into().unwrap();
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
        .with_expected_sequence(0)
        .with_command("type.googleapis.com/test.Cmd", &msg)
        .build()
        .unwrap();
    let b = client
        .command_new("orders")
        .with_expected_sequence(0)
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
async fn test_command_builder_execute_sends_the_built_command() {
    let client = MockGatewayClient::new(CommandResponse::default());
    let msg = prost_types::Duration {
        seconds: 1,
        nanos: 0,
    };
    let _ = client
        .command_new("orders")
        .with_expected_sequence(3)
        .with_command("type.googleapis.com/test.Cmd", &msg)
        .execute()
        .await
        .expect("execute should succeed");
    let sent = client.last_command.lock().unwrap().clone().expect("sent");
    assert_eq!(sent.handling.and_then(|h| h.expected_sequence), Some(3));
}

// QueryBuilder tests
#[test]
fn test_query_builder_by_correlation_id() {
    let client = MockQueryClient {
        event_book: EventBook::default(),
    };
    let root = Uuid::new_v4();
    let corr = Uuid::new_v4();
    let builder = QueryBuilder::new(&client, "orders", Some(root)).by_correlation_id(corr);

    // by_correlation_id clears root: correlation-ID queries select
    // across roots in the domain. Mirrors Python `builder.py:178`.
    assert_eq!(builder.correlation_id, Some(corr));
    assert_eq!(builder.root, None);
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

/// The coordinator reads a range as `[lower, upper + 1)`; an empty
/// Rust range must select nothing, never sequence 0.
#[test]
fn test_query_builder_empty_ranges_select_nothing() {
    let client = MockQueryClient {
        event_book: EventBook::default(),
    };
    let selects = |r: Option<Selection>, seq: u64| match r {
        Some(Selection::Range(r)) => seq >= r.lower && r.upper.is_none_or(|u| seq <= u),
        other => panic!("expected Range selection, got {other:?}"),
    };
    #[allow(clippy::reversed_empty_ranges)]
    for builder in [
        QueryBuilder::new(&client, "orders", None).range(..0),
        QueryBuilder::new(&client, "orders", None).range(0..0),
        QueryBuilder::new(&client, "orders", None).range(5..5),
        QueryBuilder::new(&client, "orders", None).range(7..3),
    ] {
        let sel = builder.selection;
        for seq in 0..10 {
            assert!(!selects(sel.clone(), seq), "{sel:?} selects {seq}");
        }
    }
    let sel = QueryBuilder::new(&client, "orders", None)
        .range(..1)
        .selection;
    assert!(selects(sel.clone(), 0));
    assert!(!selects(sel, 1));
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
    assert_eq!(cover.edition.as_deref(), Some("test-edition"));
    assert!(query.selection.is_some());
    // No correlation selector unless one is asked for: it filters.
    assert!(query.correlation_id.is_none());
}

#[test]
fn test_query_builder_build_with_correlation_id() {
    let client = MockQueryClient {
        event_book: EventBook::default(),
    };
    let corr = Uuid::new_v4();
    let query = QueryBuilder::new(&client, "orders", None)
        .by_correlation_id(corr)
        .build();

    assert_eq!(query.correlation_id, Some(uuid_to_proto(corr)));
    assert!(query.cover.unwrap().root.is_none());
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
    // Use prost_types::Duration which implements Message + Default
    let msg = prost_types::Duration {
        seconds: 42,
        nanos: 0,
    };
    let event = EventPage {
        sequence: 1,
        payload: Some(prost_types::Any {
            type_url: "type.googleapis.com/google.protobuf.Duration".to_string(),
            value: msg.encode_to_vec(),
        }),
        ..Default::default()
    };

    let decoded: Option<prost_types::Duration> = decode_event(&event, "google.protobuf.Duration");
    assert!(decoded.is_some());
    assert_eq!(decoded.unwrap().seconds, 42);
}

#[test]
fn test_decode_event_type_mismatch() {
    let msg = prost_types::Duration {
        seconds: 42,
        nanos: 0,
    };
    let event = EventPage {
        sequence: 1,
        payload: Some(prost_types::Any {
            type_url: "type.googleapis.com/google.protobuf.Duration".to_string(),
            value: msg.encode_to_vec(),
        }),
        ..Default::default()
    };

    let decoded: Option<prost_types::Duration> = decode_event(&event, "google.protobuf.Timestamp");
    assert!(decoded.is_none());
}

#[test]
fn test_decode_event_nil_event() {
    let event = EventPage {
        sequence: 1,
        ..Default::default()
    };

    let decoded: Option<prost_types::Duration> = decode_event(&event, "google.protobuf.Duration");
    assert!(
        decoded.is_none(),
        "a page without a payload decodes nothing"
    );
}

#[test]
fn test_decode_event_invalid_payload() {
    let event = EventPage {
        sequence: 1,
        payload: Some(prost_types::Any {
            type_url: "type.googleapis.com/google.protobuf.Duration".to_string(),
            value: vec![0xFF, 0xFF, 0xFF], // garbage
        }),
        ..Default::default()
    };

    let decoded: Option<prost_types::Duration> = decode_event(&event, "google.protobuf.Duration");
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
fn test_command_builder_with_conflict_resolution() {
    let client = MockGatewayClient::new(CommandResponse::default());
    let root = Uuid::new_v4();
    let builder = CommandBuilder::new(&client, "orders", root)
        .with_conflict_resolution(ConflictResolution::Strict);

    assert_eq!(builder.conflict_resolution, ConflictResolution::Strict);
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
