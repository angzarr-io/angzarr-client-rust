//! Step definitions for `features/client/query_client.feature`.
//!
//! Drives a real [`QueryClient`] (through [`QueryBuilderExt`]) against the
//! in-process test backend (`tests/common/backend.rs`).

use angzarr_client::proto::{event_page, EventBook, EventPage};
use angzarr_client::{ClientError, QueryBuilderExt, QueryClient};
use cucumber::{given, then, when, World};
use prost::Message;

use crate::common::backend::{
    cover, page_seq, root_for, short_type, single_attempt, ts, unavailable_endpoint, GenericEvent,
    Hidden, TestBackend,
};

#[derive(Debug, Default, World)]
pub struct QueryClientWorld {
    backend: Option<TestBackend>,
    client: Hidden<QueryClient>,
    endpoint: String,
    book: Option<Result<EventBook, ClientError>>,
    books: Option<Result<Vec<EventBook>, ClientError>>,
    cutoff: Option<prost_types::Timestamp>,
    expected_edition_data: Vec<String>,
    correlated_roots: usize,
}

impl QueryClientWorld {
    fn backend(&self) -> &TestBackend {
        self.backend.as_ref().expect("test backend running")
    }

    fn book(&self) -> &EventBook {
        match self.book.as_ref().expect("a query was made") {
            Ok(b) => b,
            Err(e) => panic!("query failed: {e:?}"),
        }
    }

    fn err(&self) -> &ClientError {
        match self.book.as_ref().expect("a query was made") {
            Ok(b) => panic!("query unexpectedly succeeded: {b:?}"),
            Err(e) => e,
        }
    }
}

fn seqs(pages: &[EventPage]) -> Vec<u32> {
    pages
        .iter()
        .map(|p| page_seq(p).expect("sequence"))
        .collect()
}

fn event_of(page: &EventPage) -> &prost_types::Any {
    match &page.payload {
        Some(event_page::Payload::Event(a)) => a,
        other => panic!("expected event payload, got {other:?}"),
    }
}

// --------------------------------------------------------------------------
// Arrangement
// --------------------------------------------------------------------------

#[given("a query surface available")]
async fn given_surface(world: &mut QueryClientWorld) {
    let backend = TestBackend::start_tcp().await;
    world.client.set(
        QueryClient::connect(backend.endpoint())
            .await
            .expect("query client connects to the test backend"),
    );
    world.endpoint = backend.endpoint().to_string();
    world.backend = Some(backend);
}

#[given(expr = "an aggregate {string} with root {string}")]
async fn given_unknown(world: &mut QueryClientWorld, domain: String, root: String) {
    assert!(world
        .backend()
        .stored_pages(&cover(&domain, &root, None, ""))
        .is_empty());
}

#[given(expr = "an aggregate {string} with root {string} has {int} events")]
async fn given_n_events(world: &mut QueryClientWorld, domain: String, root: String, n: u32) {
    world
        .backend()
        .seed(&cover(&domain, &root, None, ""), "ItemAdded", n);
}

#[given(expr = "an aggregate {string} with root {string} has event {string} with data {string}")]
async fn given_event_with_data(
    world: &mut QueryClientWorld,
    domain: String,
    root: String,
    name: String,
    data: String,
) {
    world
        .backend()
        .seed_event(&cover(&domain, &root, None, ""), &name, &data, None);
}

#[given(expr = "an aggregate {string} with root {string} has events at known timestamps")]
async fn given_timestamps(world: &mut QueryClientWorld, domain: String, root: String) {
    let c = cover(&domain, &root, None, "");
    for (i, at) in [
        "2024-01-15T10:00:00Z",
        "2024-01-15T10:15:00Z",
        "2024-01-15T10:30:00Z",
        "2024-01-15T10:45:00Z",
        "2024-01-15T11:00:00Z",
    ]
    .iter()
    .enumerate()
    {
        world
            .backend()
            .seed_event(&c, "ItemAdded", &format!("t{i}"), Some(ts(at)));
    }
}

#[given(expr = "an aggregate {string} with root {string} in edition {string}")]
async fn given_in_edition(
    world: &mut QueryClientWorld,
    domain: String,
    root: String,
    edition: String,
) {
    world.backend().seed_event(
        &cover(&domain, &root, None, ""),
        "ItemAdded",
        "main-0",
        None,
    );
    world.backend().seed_event(
        &cover(&domain, &root, Some(&edition), ""),
        "ItemAdded",
        "edition-0",
        None,
    );
    world.expected_edition_data = vec!["edition-0".into()];
}

#[given(expr = "an aggregate {string} with root {string} has {int} events in main")]
async fn given_main(world: &mut QueryClientWorld, domain: String, root: String, n: u32) {
    world
        .backend()
        .seed(&cover(&domain, &root, None, ""), "ItemAdded", n);
}

#[given(expr = "an aggregate {string} with root {string} has {int} events in edition {string}")]
async fn given_edition_n(
    world: &mut QueryClientWorld,
    domain: String,
    root: String,
    n: u32,
    edition: String,
) {
    world
        .backend()
        .seed(&cover(&domain, &root, Some(&edition), ""), "ItemAdded", n);
}

#[given(expr = "events with correlation ID {string} exist in multiple aggregates")]
async fn given_correlated(world: &mut QueryClientWorld, id: String) {
    let b = world.backend();
    b.seed(&cover("orders", "corr-order", None, &id), "OrderCreated", 1);
    b.seed(
        &cover("inventory", "corr-stock", None, &id),
        "StockReserved",
        1,
    );
    b.seed(
        &cover("shipping", "corr-ship", None, &id),
        "ShipmentCreated",
        1,
    );
    b.seed(
        &cover("orders", "unrelated", None, "other-flow"),
        "OrderCreated",
        1,
    );
    world.correlated_roots = 3;
}

#[given(
    expr = "an aggregate {string} with root {string} has a snapshot at sequence {int} and {int} events"
)]
async fn given_snapshot(
    world: &mut QueryClientWorld,
    domain: String,
    root: String,
    snap: u32,
    n: u32,
) {
    let c = cover(&domain, &root, None, "");
    world.backend().seed(&c, "ItemAdded", n);
    world.backend().seed_snapshot(&c, snap);
}

#[given("the query service is unavailable")]
async fn given_unavailable(world: &mut QueryClientWorld) {
    world.endpoint = unavailable_endpoint().await;
}

// --------------------------------------------------------------------------
// Actions
// --------------------------------------------------------------------------

#[when(expr = "I query events for {string} root {string}")]
async fn when_query(world: &mut QueryClientWorld, domain: String, root: String) {
    world.book = Some(
        world
            .client
            .get()
            .query(domain, root_for(&root))
            .get_event_book()
            .await,
    );
}

#[when(expr = "I query events for {string} root {string} from sequence {int} to {int}")]
async fn when_query_range(
    world: &mut QueryClientWorld,
    domain: String,
    root: String,
    lo: u32,
    hi: u32,
) {
    world.book = Some(
        world
            .client
            .get()
            .query(domain, root_for(&root))
            .range(lo..=hi)
            .get_event_book()
            .await,
    );
}

#[when(expr = "I query events for {string} root {string} from sequence {int}")]
async fn when_query_from(world: &mut QueryClientWorld, domain: String, root: String, lo: u32) {
    world.book = Some(
        world
            .client
            .get()
            .query(domain, root_for(&root))
            .range(lo..)
            .get_event_book()
            .await,
    );
}

#[when(expr = "I query events for {string} root {string} as of sequence {int}")]
async fn when_query_as_of_seq(
    world: &mut QueryClientWorld,
    domain: String,
    root: String,
    seq: u32,
) {
    world.book = Some(
        world
            .client
            .get()
            .query(domain, root_for(&root))
            .as_of_sequence(seq)
            .get_event_book()
            .await,
    );
}

#[when(expr = "I query events for {string} root {string} as of time {string}")]
async fn when_query_as_of_time(
    world: &mut QueryClientWorld,
    domain: String,
    root: String,
    at: String,
) {
    world.cutoff = Some(ts(&at));
    let builder = world
        .client
        .get()
        .query(domain, root_for(&root))
        .as_of_time(&at)
        .expect("valid RFC 3339 timestamp");
    world.book = Some(builder.get_event_book().await);
}

#[when(expr = "I query events for {string} root {string} in edition {string}")]
async fn when_query_edition(
    world: &mut QueryClientWorld,
    domain: String,
    root: String,
    edition: String,
) {
    world.book = Some(
        world
            .client
            .get()
            .query(domain, root_for(&root))
            .with_edition(edition)
            .get_event_book()
            .await,
    );
}

#[when(expr = "I query events by correlation ID {string}")]
async fn when_query_correlation(world: &mut QueryClientWorld, id: String) {
    world.books = Some(
        world
            .client
            .get()
            .query_domain("orders")
            .by_correlation_id(id)
            .get_events()
            .await,
    );
}

#[when("I query events with empty domain")]
async fn when_query_empty_domain(world: &mut QueryClientWorld) {
    world.book = Some(
        world
            .client
            .get()
            .query("", root_for("any"))
            .get_event_book()
            .await,
    );
}

#[when("I attempt to query events")]
async fn when_attempt(world: &mut QueryClientWorld) {
    let result = match QueryClient::connect_with_retry(&world.endpoint, &single_attempt()).await {
        Ok(client) => {
            client
                .query("orders", root_for("any"))
                .get_event_book()
                .await
        }
        Err(e) => Err(e),
    };
    world.book = Some(result);
}

// --------------------------------------------------------------------------
// Outcomes
// --------------------------------------------------------------------------

#[then(expr = "the history is empty and the next sequence is {int}")]
async fn then_empty(world: &mut QueryClientWorld, next: u32) {
    let book = world.book();
    assert!(book.pages.is_empty());
    assert_eq!(book.next_sequence, next);
}

#[then(expr = "I receive {int} events")]
async fn then_n_events(world: &mut QueryClientWorld, n: usize) {
    assert_eq!(world.book().pages.len(), n);
}

#[then(expr = "the events are in sequence order {int} to {int}")]
async fn then_order(world: &mut QueryClientWorld, lo: u32, hi: u32) {
    assert_eq!(seqs(&world.book().pages), (lo..=hi).collect::<Vec<_>>());
}

#[then(expr = "the first event has type {string}")]
async fn then_first_type(world: &mut QueryClientWorld, name: String) {
    let page = world.book().pages.first().expect("an event");
    assert_eq!(short_type(event_of(page)), name);
}

#[then(expr = "the first event has payload {string}")]
async fn then_first_payload(world: &mut QueryClientWorld, data: String) {
    let page = world.book().pages.first().expect("an event");
    let evt = GenericEvent::decode(event_of(page).value.as_slice()).expect("decodes");
    assert_eq!(evt.data, data);
}

#[then(expr = "the first event has sequence {int}")]
async fn then_first_seq(world: &mut QueryClientWorld, seq: u32) {
    assert_eq!(seqs(&world.book().pages).first().copied(), Some(seq));
}

#[then(expr = "the last event has sequence {int}")]
async fn then_last_seq(world: &mut QueryClientWorld, seq: u32) {
    assert_eq!(seqs(&world.book().pages).last().copied(), Some(seq));
}

#[then("I receive no events")]
async fn then_none(world: &mut QueryClientWorld) {
    if let Some(books) = &world.books {
        let books = books.as_ref().expect("query succeeds");
        assert_eq!(books.iter().map(|b| b.pages.len()).sum::<usize>(), 0);
    } else {
        assert!(world.book().pages.is_empty());
    }
}

#[then("I receive events up to that timestamp")]
async fn then_up_to_time(world: &mut QueryClientWorld) {
    let cutoff = world.cutoff.expect("cutoff recorded");
    let pages = &world.book().pages;
    assert_eq!(pages.len(), 3, "events at 10:00, 10:15 and 10:30 qualify");
    for p in pages {
        let at = p.created_at.expect("timestamp");
        assert!((at.seconds, at.nanos) <= (cutoff.seconds, cutoff.nanos));
    }
}

#[then("I receive events from that edition only")]
async fn then_edition_only(world: &mut QueryClientWorld) {
    let data: Vec<String> = world
        .book()
        .pages
        .iter()
        .map(|p| {
            GenericEvent::decode(event_of(p).value.as_slice())
                .expect("decodes")
                .data
        })
        .collect();
    assert_eq!(data, world.expected_edition_data);
}

#[then("I receive events from all correlated aggregates")]
async fn then_correlated(world: &mut QueryClientWorld) {
    let books = world
        .books
        .as_ref()
        .expect("a correlation query was made")
        .as_ref()
        .expect("query succeeds");
    assert_eq!(books.len(), world.correlated_roots);
    let mut domains: Vec<String> = books
        .iter()
        .map(|b| b.cover.as_ref().expect("cover").domain.clone())
        .collect();
    domains.sort();
    assert_eq!(domains, vec!["inventory", "orders", "shipping"]);
}

#[then(expr = "the result carries a snapshot taken at sequence {int}")]
async fn then_snapshot(world: &mut QueryClientWorld, seq: u32) {
    let snap = world.book().snapshot.as_ref().expect("snapshot present");
    assert_eq!(snap.sequence, seq);
}

#[then("the query is refused because a domain is required")]
async fn then_domain_required(world: &mut QueryClientWorld) {
    let err = world.err();
    assert!(
        err.is_invalid_argument(),
        "expected INVALID_ARGUMENT, got {err:?}"
    );
    let msg = err
        .status()
        .map(|s| s.message().to_string())
        .unwrap_or_default();
    assert!(msg.contains("domain"), "message: {msg}");
}

#[then("the query fails because the backend is unreachable")]
async fn then_unreachable(world: &mut QueryClientWorld) {
    let err = world.err();
    assert!(
        err.is_connection_error(),
        "expected connection error, got {err:?}"
    );
}
