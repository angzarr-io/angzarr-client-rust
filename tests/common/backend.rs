//! In-process test backend for the client-surface feature tier.
//!
//! `features/client/README.md` defines the "test backend" for
//! `aggregate_client`, `domain-client` and `query_client`: an in-process
//! fake of the coordinator gRPC services owned by each client repo. This
//! module is that fake. It serves the real generated tonic services
//! (`CommandHandlerCoordinatorService` and `EventQueryService`) over a real
//! TCP or Unix socket, so every step drives the library's actual client
//! objects over the wire.
//!
//! The backend keeps an in-memory event store and a tiny scripted
//! aggregate:
//!
//! - commands are `GenericCommand` payloads (or the `CreateOrder` fixture);
//!   a command named `XxxYyy` emits `count` events (default 1) named by
//!   [`event_name_for`];
//! - `CreateOrder` requires `customer_id`;
//! - `CancelOrder` is refused once the history holds an `OrderShipped`;
//! - a command whose `Handling.expected_sequence` is set must carry the
//!   aggregate's next sequence (STRICT: a stale one is ABORTED).
//!
//! Coordinator RPC modes (command_handler.proto): `Handle` (DECISION)
//! answers the decision and runs the projectors on a delayed task,
//! `HandleSync` (SIMPLE) runs the configured projectors before replying and
//! reports them in `outcome.projections`, `HandleAsync` (ASYNC) enqueues the
//! command and answers at once.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use angzarr_client::proto::{
    command_handler_coordinator_service_server::{
        CommandHandlerCoordinatorService, CommandHandlerCoordinatorServiceServer,
    },
    event_query_service_server::{EventQueryService, EventQueryServiceServer},
    query::Selection,
    temporal_query::PointInTime,
    CommandBook, CommandPage, CommandResponse, CorrelationQuery, Cover, DeliveryOutcome, EventBook,
    EventMeta, EventPage, Outcome, Projection, Query, Snapshot, Uuid as ProtoUuid,
};
use prost::Message;
use prost_types::{Any, Timestamp};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::oneshot;
use tokio_stream::StreamExt;
use tonic::transport::server::Connected;
use tonic::{Request, Response, Status};

use super::fixtures::CreateOrder;

/// Type URL prefix + package used for every backend command/event.
pub const TYPE_PREFIX: &str = "type.googleapis.com/order.";

/// Payload for every scripted command except `CreateOrder`.
#[derive(Clone, PartialEq, Message)]
pub struct GenericCommand {
    #[prost(string, tag = "1")]
    pub data: String,
    /// Number of events to emit (0 = 1).
    #[prost(uint32, tag = "2")]
    pub count: u32,
}

/// Payload of every backend-emitted event.
#[derive(Clone, PartialEq, Message)]
pub struct GenericEvent {
    #[prost(string, tag = "1")]
    pub data: String,
}

/// Name of the event a command type emits.
pub fn event_name_for(command: &str) -> String {
    match command {
        "CreateOrder" => "OrderCreated".into(),
        "AddItem" => "ItemAdded".into(),
        "CancelOrder" => "OrderCancelled".into(),
        "ShipOrder" => "OrderShipped".into(),
        "ReserveStock" => "StockReserved".into(),
        other => format!("{other}Done"),
    }
}

/// Root bytes for a scenario label: a UUID literal parses as-is, any other
/// label maps through uuid5(NAMESPACE_OID, label).
pub fn root_for(label: &str) -> uuid::Uuid {
    uuid::Uuid::parse_str(label)
        .unwrap_or_else(|_| uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, label.as_bytes()))
}

/// Correlation id for a scenario label (correlation ids are UUIDs on the
/// wire); the same mapping as [`root_for`].
pub fn correlation_for(label: &str) -> uuid::Uuid {
    root_for(label)
}

/// `Any` for a generic event of type `name` carrying `data`.
pub fn event_any(name: &str, data: &str) -> Any {
    Any {
        type_url: format!("{TYPE_PREFIX}{name}"),
        value: GenericEvent { data: data.into() }.encode_to_vec(),
    }
}

/// `Any` for a generic command of type `name`.
pub fn command_any(name: &str, data: &str, count: u32) -> Any {
    Any {
        type_url: format!("{TYPE_PREFIX}{name}"),
        value: GenericCommand {
            data: data.into(),
            count,
        }
        .encode_to_vec(),
    }
}

/// Short type name (`Foo`) of an `Any`.
pub fn short_type(any: &Any) -> &str {
    any.type_url.rsplit(['/', '.']).next().unwrap_or("")
}

/// Sequence of a page.
pub fn page_seq(page: &EventPage) -> u64 {
    page.sequence
}

/// The time a page was recorded, if stamped.
fn recorded_at(page: &EventPage) -> Option<&Timestamp> {
    page.meta.as_ref()?.recorded_at.as_ref()
}

/// A stored page: `any` at `sequence`, recorded at `at` under
/// `correlation`.
fn stored_page(
    sequence: u64,
    any: Any,
    at: Timestamp,
    correlation: Option<&ProtoUuid>,
) -> EventPage {
    EventPage {
        payload: Some(any),
        meta: Some(EventMeta {
            recorded_at: Some(at),
            correlation_id: correlation.cloned(),
            ..Default::default()
        }),
        sequence,
    }
}

/// Process-wide lock serialising scenarios that mutate environment
/// variables; held for the lifetime of the scenario's world.
pub fn env_lock() -> Arc<tokio::sync::Mutex<()>> {
    static LOCK: std::sync::OnceLock<Arc<tokio::sync::Mutex<()>>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

fn edition_key(cover: &Cover) -> String {
    match cover.edition.as_deref() {
        None | Some("") | Some("angzarr") => String::new(),
        Some(name) => name.to_string(),
    }
}

type Key = (String, String, Vec<u8>);

fn key_of(cover: &Cover) -> Key {
    (
        edition_key(cover),
        cover.domain.clone(),
        cover
            .root
            .as_ref()
            .map(|r| r.value.to_vec())
            .unwrap_or_default(),
    )
}

#[derive(Debug, Default, Clone)]
struct Aggregate {
    cover: Cover,
    pages: Vec<EventPage>,
    snapshot: Option<Snapshot>,
}

impl Aggregate {
    fn next_sequence(&self) -> u64 {
        self.pages
            .last()
            .map(|p| p.sequence + 1)
            .or_else(|| self.snapshot.as_ref().map(|s| s.sequence + 1))
            .unwrap_or(0)
    }

    /// Pages recorded under `correlation`.
    fn correlated(&self, correlation: &ProtoUuid) -> Vec<EventPage> {
        self.pages
            .iter()
            .filter(|p| {
                p.meta
                    .as_ref()
                    .and_then(|m| m.correlation_id.as_ref())
                    .is_some_and(|c| c == correlation)
            })
            .cloned()
            .collect()
    }
}

#[derive(Debug, Default)]
struct Config {
    known_domains: Option<HashSet<String>>,
    response_delay: Option<Duration>,
    projector_domains: HashSet<String>,
}

/// One downstream projector run: `(projector, domain, sequence)`.
pub type ProjectorRun = (String, String, u64);

#[derive(Debug, Default)]
struct Inner {
    store: Mutex<BTreeMap<Key, Aggregate>>,
    config: Mutex<Config>,
    projector_runs: Mutex<Vec<ProjectorRun>>,
    rpcs: Mutex<Vec<&'static str>>,
    accepted: AtomicUsize,
    active: Arc<AtomicUsize>,
}

/// Handle to a running test backend.
pub struct TestBackend {
    inner: Arc<Inner>,
    endpoint: String,
    uds_path: Option<PathBuf>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for TestBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestBackend")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl Drop for TestBackend {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(path) = &self.uds_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Connection wrapper that counts accepted and live transport connections.
struct Counted<T> {
    io: T,
    active: Arc<AtomicUsize>,
}

impl<T> Drop for Counted<T> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<T> Connected for Counted<T> {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl<T: AsyncRead + Unpin> AsyncRead for Counted<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Counted<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }
}

#[derive(Clone)]
struct Svc(Arc<Inner>);

impl TestBackend {
    /// Start a backend on an ephemeral TCP port of 127.0.0.1.
    pub async fn start_tcp() -> Self {
        Self::start_tcp_on(0).await
    }

    /// Start a backend on 127.0.0.1:`port` (0 = ephemeral).
    pub async fn start_tcp_on(port: u16) -> Self {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("bind test backend");
        let addr: SocketAddr = listener.local_addr().expect("local addr");
        let inner = Arc::new(Inner::default());
        let incoming = Self::counted(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            &inner,
        );
        let (shutdown, task) = Self::serve(inner.clone(), incoming);
        Self {
            inner,
            endpoint: format!("http://{addr}"),
            uds_path: None,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    /// Start a backend listening on the Unix socket `path`.
    pub async fn start_uds(path: &Path) -> Self {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create socket dir");
        }
        let _ = std::fs::remove_file(path);
        let listener = tokio::net::UnixListener::bind(path).expect("bind unix socket");
        let inner = Arc::new(Inner::default());
        let incoming = Self::counted(
            tokio_stream::wrappers::UnixListenerStream::new(listener),
            &inner,
        );
        let (shutdown, task) = Self::serve(inner.clone(), incoming);
        Self {
            inner,
            endpoint: path.display().to_string(),
            uds_path: Some(path.to_path_buf()),
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    fn counted<S, T>(
        stream: S,
        inner: &Arc<Inner>,
    ) -> impl tokio_stream::Stream<Item = io::Result<Counted<T>>> + Send + 'static
    where
        S: tokio_stream::Stream<Item = io::Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let counts = inner.clone();
        stream.map(move |res| {
            res.map(|io| {
                counts.accepted.fetch_add(1, Ordering::SeqCst);
                counts.active.fetch_add(1, Ordering::SeqCst);
                Counted {
                    io,
                    active: counts.active.clone(),
                }
            })
        })
    }

    fn serve<S, T>(
        inner: Arc<Inner>,
        incoming: S,
    ) -> (oneshot::Sender<()>, tokio::task::JoinHandle<()>)
    where
        S: tokio_stream::Stream<Item = io::Result<Counted<T>>> + Send + 'static,
        T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (tx, rx) = oneshot::channel::<()>();
        let svc = Svc(inner);
        let task = tokio::spawn(async move {
            let _ = tonic::transport::Server::builder()
                .add_service(CommandHandlerCoordinatorServiceServer::new(svc.clone()))
                .add_service(EventQueryServiceServer::new(svc))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = rx.await;
                })
                .await;
        });
        (tx, task)
    }

    /// Endpoint string the client objects accept (`http://ip:port` or a
    /// socket path).
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// TCP port, for TCP backends.
    pub fn port(&self) -> Option<u16> {
        self.endpoint
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
    }

    /// Stop serving and wait until the listener is closed.
    pub async fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        }
    }

    /// Restrict the domains the backend recognises.
    pub fn set_known_domains(&self, domains: &[&str]) {
        self.inner.config.lock().unwrap().known_domains =
            Some(domains.iter().map(|d| d.to_string()).collect());
    }

    /// Delay every command response by `delay`.
    pub fn set_response_delay(&self, delay: Duration) {
        self.inner.config.lock().unwrap().response_delay = Some(delay);
    }

    /// Configure a projector for `domain`.
    pub fn add_projector(&self, domain: &str) {
        self.inner
            .config
            .lock()
            .unwrap()
            .projector_domains
            .insert(domain.into());
    }

    /// Projector runs recorded so far.
    pub fn projector_runs(&self) -> Vec<ProjectorRun> {
        self.inner.projector_runs.lock().unwrap().clone()
    }

    /// RPC method names received, in arrival order.
    pub fn rpcs(&self) -> Vec<&'static str> {
        self.inner.rpcs.lock().unwrap().clone()
    }

    /// Transport connections accepted since start.
    pub fn accepted_connections(&self) -> usize {
        self.inner.accepted.load(Ordering::SeqCst)
    }

    /// Transport connections currently open.
    pub fn active_connections(&self) -> usize {
        self.inner.active.load(Ordering::SeqCst)
    }

    /// Seed `count` events onto an aggregate, continuing its history.
    /// Each event is `<name>` with data `"<name>-<seq>"`.
    pub fn seed(&self, cover: &Cover, name: &str, count: u64) {
        self.seed_correlated(cover, name, count, None);
    }

    /// [`Self::seed`], recording the events under `correlation`.
    pub fn seed_correlated(
        &self,
        cover: &Cover,
        name: &str,
        count: u64,
        correlation: Option<&ProtoUuid>,
    ) {
        let mut store = self.inner.store.lock().unwrap();
        let agg = store.entry(key_of(cover)).or_insert_with(|| Aggregate {
            cover: cover.clone(),
            ..Default::default()
        });
        for _ in 0..count {
            let seq = agg.next_sequence();
            agg.pages.push(stored_page(
                seq,
                event_any(name, &format!("{name}-{seq}")),
                now(),
                correlation,
            ));
        }
    }

    /// Seed one event with explicit payload data and timestamp.
    pub fn seed_event(&self, cover: &Cover, name: &str, data: &str, at: Option<Timestamp>) {
        let mut store = self.inner.store.lock().unwrap();
        let agg = store.entry(key_of(cover)).or_insert_with(|| Aggregate {
            cover: cover.clone(),
            ..Default::default()
        });
        let seq = agg.next_sequence();
        agg.pages.push(stored_page(
            seq,
            event_any(name, data),
            at.unwrap_or_else(now),
            None,
        ));
    }

    /// Record a snapshot at `sequence` and continue history after it.
    pub fn seed_snapshot(&self, cover: &Cover, sequence: u64) {
        let mut store = self.inner.store.lock().unwrap();
        let agg = store.entry(key_of(cover)).or_insert_with(|| Aggregate {
            cover: cover.clone(),
            ..Default::default()
        });
        agg.snapshot = Some(Snapshot {
            sequence,
            state: Some(event_any("OrderState", &format!("as-of-{sequence}"))),
            ..Default::default()
        });
    }

    /// Stored pages of an aggregate (empty when it does not exist).
    pub fn stored_pages(&self, cover: &Cover) -> Vec<EventPage> {
        self.inner
            .store
            .lock()
            .unwrap()
            .get(&key_of(cover))
            .map(|a| a.pages.clone())
            .unwrap_or_default()
    }

    /// Total number of stored events across every aggregate.
    pub fn total_events(&self) -> usize {
        self.inner
            .store
            .lock()
            .unwrap()
            .values()
            .map(|a| a.pages.len())
            .sum()
    }
}

/// Build a cover for `(domain, root label)` with an optional edition.
pub fn cover(domain: &str, root_label: &str, edition: Option<&str>) -> Cover {
    Cover {
        domain: domain.into(),
        root: Some(ProtoUuid::from(root_for(root_label))),
        edition: edition.map(Into::into),
    }
}

fn now() -> Timestamp {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: d.as_secs() as i64,
        nanos: d.subsec_nanos() as i32,
    }
}

fn ts_le(a: &Timestamp, b: &Timestamp) -> bool {
    (a.seconds, a.nanos) <= (b.seconds, b.nanos)
}

/// Outcome of running the scripted aggregate.
struct Decided {
    events: Vec<(String, String)>,
}

/// Run the scripted aggregate for one command page against `history`.
fn decide(page: &CommandPage, history: &[EventPage]) -> Result<Decided, Status> {
    let any = angzarr_client::router::binding::command_payload(page)
        .ok_or_else(|| Status::invalid_argument("missing command payload"))?;
    let name = short_type(any).to_string();
    if name.is_empty() {
        return Err(Status::invalid_argument("missing command type"));
    }
    if name == "CreateOrder" {
        let cmd = CreateOrder::decode(any.value.as_slice())
            .map_err(|e| Status::invalid_argument(format!("malformed payload: {e}")))?;
        if cmd.customer_id.is_empty() {
            return Err(Status::invalid_argument(
                "missing required field: customer_id",
            ));
        }
        return Ok(Decided {
            events: vec![("OrderCreated".into(), cmd.customer_id)],
        });
    }
    let cmd = GenericCommand::decode(any.value.as_slice())
        .map_err(|e| Status::invalid_argument(format!("malformed payload: {e}")))?;
    if name == "CancelOrder"
        && history.iter().any(|p| {
            p.payload
                .as_ref()
                .is_some_and(|e| short_type(e) == "OrderShipped")
        })
    {
        return Err(Status::failed_precondition("cannot cancel shipped order"));
    }
    let n = cmd.count.max(1);
    Ok(Decided {
        events: (0..n)
            .map(|_| (event_name_for(&name), cmd.data.clone()))
            .collect(),
    })
}

fn build_pages(
    decided: &Decided,
    first_seq: u64,
    correlation: Option<&ProtoUuid>,
) -> Vec<EventPage> {
    decided
        .events
        .iter()
        .enumerate()
        .map(|(i, (name, data))| {
            stored_page(
                first_seq + i as u64,
                event_any(name, data),
                now(),
                correlation,
            )
        })
        .collect()
}

fn command_parts(book: &CommandBook) -> Result<(&Cover, &CommandPage), Status> {
    let cover = book
        .cover
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("missing cover"))?;
    if cover.domain.is_empty() {
        return Err(Status::invalid_argument("domain is required"));
    }
    let page = book
        .page
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("missing command page"))?;
    Ok((cover, page))
}

impl Svc {
    fn rpc(&self, name: &'static str) {
        self.0.rpcs.lock().unwrap().push(name);
    }

    fn check_domain(&self, domain: &str) -> Result<(), Status> {
        let cfg = self.0.config.lock().unwrap();
        if let Some(known) = &cfg.known_domains {
            if !known.contains(domain) {
                return Err(Status::not_found(format!("unknown domain: {domain}")));
            }
        }
        Ok(())
    }

    /// Validate, decide and persist one command; returns the stored book.
    /// A stale `Handling.expected_sequence` is ABORTED (STRICT).
    fn apply(&self, book: &CommandBook) -> Result<EventBook, Status> {
        let (cover, page) = command_parts(book)?;
        self.check_domain(&cover.domain)?;
        let mut store = self.0.store.lock().unwrap();
        let agg = store.entry(key_of(cover)).or_insert_with(|| Aggregate {
            cover: cover.clone(),
            ..Default::default()
        });
        let next = agg.next_sequence();
        if let Some(n) = book.handling.as_ref().and_then(|h| h.expected_sequence) {
            if n != next {
                return Err(Status::aborted(format!(
                    "sequence mismatch: expected {next}, got {n}"
                )));
            }
        }
        let decided = decide(page, &agg.pages)?;
        let pages = build_pages(&decided, next, page.correlation_id.as_ref());
        agg.pages.extend(pages.iter().cloned());
        Ok(EventBook {
            cover: Some(cover.clone()),
            next_sequence: agg.next_sequence(),
            pages,
            ..Default::default()
        })
    }

    /// Wait out the configured response delay.
    async fn delay(&self) {
        let delay = self.0.config.lock().unwrap().response_delay;
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
    }

    /// Run the projectors on a delayed task, as downstream work does.
    fn project_later(&self, events: EventBook) {
        let svc = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            svc.run_projectors(&events);
        });
    }

    fn run_projectors(&self, events: &EventBook) -> Vec<Projection> {
        let domain = events
            .cover
            .as_ref()
            .map(|c| c.domain.clone())
            .unwrap_or_default();
        if !self
            .0
            .config
            .lock()
            .unwrap()
            .projector_domains
            .contains(&domain)
        {
            return Vec::new();
        }
        let mut out = Vec::new();
        for page in &events.pages {
            let seq = page_seq(page);
            self.0.projector_runs.lock().unwrap().push((
                format!("{domain}-projector"),
                domain.clone(),
                seq,
            ));
            out.push(Projection {
                cover: events.cover.clone(),
                projector: format!("{domain}-projector"),
                next_sequence: seq + 1,
                projection: None,
            });
        }
        out
    }
}

#[tonic::async_trait]
impl CommandHandlerCoordinatorService for Svc {
    /// DECISION: the decision now, projectors later.
    async fn handle(
        &self,
        request: Request<CommandBook>,
    ) -> Result<Response<CommandResponse>, Status> {
        self.rpc("Handle");
        self.delay().await;
        let events = self.apply(request.get_ref())?;
        self.project_later(events.clone());
        Ok(Response::new(CommandResponse {
            events: Some(events),
            outcome: Some(applied(Vec::new())),
        }))
    }

    /// ASYNC: validate and enqueue; the decision and projectors run later.
    async fn handle_async(&self, request: Request<CommandBook>) -> Result<Response<()>, Status> {
        self.rpc("HandleAsync");
        let book = request.into_inner();
        command_parts(&book)?;
        let svc = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if let Ok(events) = svc.apply(&book) {
                svc.run_projectors(&events);
            }
        });
        Ok(Response::new(()))
    }

    /// SIMPLE: the decision after the synchronous projectors ran.
    async fn handle_sync(
        &self,
        request: Request<CommandBook>,
    ) -> Result<Response<CommandResponse>, Status> {
        self.rpc("HandleSync");
        self.delay().await;
        let events = self.apply(request.get_ref())?;
        let projections = self.run_projectors(&events);
        Ok(Response::new(CommandResponse {
            events: Some(events),
            outcome: Some(applied(projections)),
        }))
    }
}

/// An APPLIED outcome carrying `projections`.
fn applied(projections: Vec<Projection>) -> Outcome {
    Outcome {
        delivery: DeliveryOutcome::Applied as i32,
        projections,
        ..Default::default()
    }
}

type BookStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<EventBook, Status>> + Send>>;
type RootStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<Cover, Status>> + Send>>;

impl Svc {
    fn select(&self, query: &Query) -> Result<Vec<EventBook>, Status> {
        let cover = query
            .cover
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing cover"))?;
        let store = self.0.store.lock().unwrap();
        if let (None, Some(correlation)) = (cover.root.as_ref(), query.correlation_id.as_ref()) {
            return Ok(store
                .values()
                .filter(|a| a.cover.domain == cover.domain)
                .filter_map(|a| {
                    let pages = a.correlated(correlation);
                    (!pages.is_empty()).then(|| EventBook {
                        cover: Some(a.cover.clone()),
                        pages,
                        snapshot: None,
                        next_sequence: a.next_sequence(),
                    })
                })
                .collect());
        }
        if cover.domain.is_empty() {
            return Err(Status::invalid_argument("domain is required"));
        }
        let agg = store.get(&key_of(cover)).cloned().unwrap_or_default();
        let pages: Vec<EventPage> = agg
            .pages
            .iter()
            .filter(|p| {
                let seq = page_seq(p);
                match &query.selection {
                    None => true,
                    Some(Selection::Range(r)) => seq >= r.lower && r.upper.is_none_or(|u| seq <= u),
                    Some(Selection::Sequences(s)) => s.values.contains(&seq),
                    Some(Selection::Temporal(t)) => match &t.point_in_time {
                        Some(PointInTime::AsOfSequence(s)) => seq <= *s,
                        Some(PointInTime::AsOfTime(at)) => {
                            recorded_at(p).is_some_and(|c| ts_le(c, at))
                        }
                        None => true,
                    },
                }
            })
            .cloned()
            .collect();
        Ok(vec![EventBook {
            cover: Some(cover.clone()),
            pages,
            snapshot: agg.snapshot.clone(),
            next_sequence: agg.next_sequence(),
        }])
    }
}

#[tonic::async_trait]
impl EventQueryService for Svc {
    async fn get_event_book(&self, request: Request<Query>) -> Result<Response<EventBook>, Status> {
        self.rpc("GetEventBook");
        let books = self.select(request.get_ref())?;
        Ok(Response::new(books.into_iter().next().unwrap_or_default()))
    }

    type GetEventsStream = BookStream;

    async fn get_events(
        &self,
        request: Request<Query>,
    ) -> Result<Response<Self::GetEventsStream>, Status> {
        self.rpc("GetEvents");
        let books = self.select(request.get_ref())?;
        Ok(Response::new(Box::pin(tokio_stream::iter(
            books.into_iter().map(Ok),
        ))))
    }

    type GetCorrelatedEventsStream = BookStream;

    /// Every stream of the edition holding pages of the correlation, one
    /// book per stream, across domains.
    async fn get_correlated_events(
        &self,
        request: Request<CorrelationQuery>,
    ) -> Result<Response<Self::GetCorrelatedEventsStream>, Status> {
        self.rpc("GetCorrelatedEvents");
        let query = request.into_inner();
        let correlation = query
            .correlation_id
            .ok_or_else(|| Status::invalid_argument("correlation_id is required"))?;
        let edition = query.edition.unwrap_or_default();
        let books: Vec<EventBook> = self
            .0
            .store
            .lock()
            .unwrap()
            .values()
            .filter(|a| edition_key(&a.cover) == edition)
            .filter_map(|a| {
                let pages = a.correlated(&correlation);
                (!pages.is_empty()).then(|| EventBook {
                    cover: Some(a.cover.clone()),
                    pages,
                    snapshot: None,
                    next_sequence: a.next_sequence(),
                })
            })
            .collect();
        Ok(Response::new(Box::pin(tokio_stream::iter(
            books.into_iter().map(Ok),
        ))))
    }

    type GetAggregateRootsStream = RootStream;

    async fn get_aggregate_roots(
        &self,
        _request: Request<()>,
    ) -> Result<Response<Self::GetAggregateRootsStream>, Status> {
        Err(Status::unimplemented("test backend does not list roots"))
    }
}

/// An endpoint with nothing listening (a just-released ephemeral port).
pub async fn unavailable_endpoint() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind probe");
    let addr = listener.local_addr().expect("addr");
    drop(listener);
    format!("http://{addr}")
}

/// A retry policy that tries once — connection failures surface
/// immediately instead of after the default backoff schedule.
pub fn single_attempt() -> angzarr_client::RetryPolicy {
    angzarr_client::RetryPolicy::default().with_max_attempts(1)
}

/// Seconds of `s` as a `Timestamp`, parsed from RFC 3339.
pub fn ts(rfc3339: &str) -> Timestamp {
    angzarr_client::convert::parse_timestamp(rfc3339).expect("valid timestamp")
}

/// Holder for values without a `Debug` impl (the client objects), so step
/// worlds can derive `Debug`.
#[derive(Clone)]
pub struct Hidden<T>(pub Option<T>);

impl<T> Default for Hidden<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> std::fmt::Debug for Hidden<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "Some(..)" } else { "None" })
    }
}

impl<T> Hidden<T> {
    pub fn get(&self) -> &T {
        self.0.as_ref().expect("value set")
    }
    pub fn set(&mut self, v: T) {
        self.0 = Some(v);
    }
    pub fn take(&mut self) -> Option<T> {
        self.0.take()
    }
    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }
}
