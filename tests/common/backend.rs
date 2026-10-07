//! In-process test backend for the client-surface feature tier.
//!
//! `features/client/README.md` defines the "test backend" for
//! `aggregate_client`, `domain-client`, `query_client` and
//! `speculative_client`: an in-process fake of the coordinator gRPC
//! services owned by each client repo. This module is that fake. It serves
//! the real generated tonic services (`CommandHandlerCoordinatorService`,
//! `EventQueryService` and the projector / saga / process-manager
//! coordinator services) over a real TCP or Unix socket, so every step
//! drives the library's actual client objects over the wire.
//!
//! The backend keeps an in-memory event store and a tiny scripted
//! aggregate:
//!
//! - commands are `GenericCommand` payloads (or the `CreateOrder` fixture);
//!   a command named `XxxYyy` emits `count` events (default 1) named by
//!   [`event_name_for`];
//! - `CreateOrder` requires `customer_id`;
//! - `CancelOrder` is refused once the history holds an `OrderShipped`;
//! - every non-deferred page must carry the aggregate's next sequence.
//!
//! Sync modes: `ASYNC` returns immediately and runs downstream work on a
//! delayed task, `SIMPLE` runs the configured projectors before replying,
//! `CASCADE` also runs the configured sagas (which write to their target
//! aggregate) before replying.

use std::collections::{BTreeMap, HashMap, HashSet};
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
    command_page, event_page,
    event_query_service_server::{EventQueryService, EventQueryServiceServer},
    page_header::SequenceType,
    process_manager_coordinator_service_server::{
        ProcessManagerCoordinatorService, ProcessManagerCoordinatorServiceServer,
    },
    projector_coordinator_service_server::{
        ProjectorCoordinatorService, ProjectorCoordinatorServiceServer,
    },
    query::Selection,
    saga_coordinator_service_server::{SagaCoordinatorService, SagaCoordinatorServiceServer},
    temporal_query::PointInTime,
    AggregateRoot, AngzarrDeferredSequence, BusinessResponse, CommandBook, CommandPage,
    CommandRequest, CommandResponse, Cover, Edition, EventBook, EventPage, EventRequest,
    FactInjectionResponse, PageHeader, ProcessManagerCoordinatorRequest,
    ProcessManagerHandleResponse, Projection, Query, SagaHandleRequest, SagaResponse, Snapshot,
    SpeculateCommandHandlerRequest, SpeculatePmRequest, SpeculateProjectorRequest,
    SpeculateSagaRequest, SyncMode, Uuid as ProtoUuid,
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

/// Explicit sequence of a page header, if any.
pub fn page_seq(page: &EventPage) -> Option<u32> {
    match page.header.as_ref()?.sequence_type.as_ref()? {
        SequenceType::Sequence(n) => Some(*n),
        _ => None,
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
    match cover.edition.as_ref().map(|e| e.name.as_str()) {
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
            .map(|r| r.value.clone())
            .unwrap_or_default(),
    )
}

#[derive(Debug, Default, Clone)]
struct Aggregate {
    cover: Cover,
    pages: Vec<EventPage>,
    snapshot: Option<Snapshot>,
    correlations: HashSet<String>,
}

impl Aggregate {
    fn next_sequence(&self) -> u32 {
        self.pages
            .last()
            .and_then(page_seq)
            .map(|s| s + 1)
            .or_else(|| self.snapshot.as_ref().map(|s| s.sequence + 1))
            .unwrap_or(0)
    }
}

#[derive(Debug, Default)]
struct Config {
    known_domains: Option<HashSet<String>>,
    response_delay: Option<Duration>,
    projector_domains: HashSet<String>,
    /// source domain → target domain.
    sagas: HashMap<String, String>,
}

/// One downstream projector run: `(projector, domain, sequence)`.
pub type ProjectorRun = (String, String, u32);

#[derive(Debug, Default)]
struct Inner {
    store: Mutex<BTreeMap<Key, Aggregate>>,
    config: Mutex<Config>,
    projector_runs: Mutex<Vec<ProjectorRun>>,
    sync_modes: Mutex<Vec<i32>>,
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
                .add_service(EventQueryServiceServer::new(svc.clone()))
                .add_service(ProjectorCoordinatorServiceServer::new(svc.clone()))
                .add_service(SagaCoordinatorServiceServer::new(svc.clone()))
                .add_service(ProcessManagerCoordinatorServiceServer::new(svc))
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

    /// Configure a saga from `source` to `target`.
    pub fn add_saga(&self, source: &str, target: &str) {
        self.inner
            .config
            .lock()
            .unwrap()
            .sagas
            .insert(source.into(), target.into());
    }

    /// Projector runs recorded so far.
    pub fn projector_runs(&self) -> Vec<ProjectorRun> {
        self.inner.projector_runs.lock().unwrap().clone()
    }

    /// Sync modes received on `HandleCommand`, in arrival order.
    pub fn sync_modes(&self) -> Vec<i32> {
        self.inner.sync_modes.lock().unwrap().clone()
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
    pub fn seed(&self, cover: &Cover, name: &str, count: u32) {
        let mut store = self.inner.store.lock().unwrap();
        let agg = store.entry(key_of(cover)).or_insert_with(|| Aggregate {
            cover: cover.clone(),
            ..Default::default()
        });
        for _ in 0..count {
            let seq = agg.next_sequence();
            agg.pages.push(EventPage {
                header: Some(PageHeader {
                    sequence_type: Some(SequenceType::Sequence(seq)),
                    sync_mode: None,
                }),
                created_at: Some(now()),
                payload: Some(event_page::Payload::Event(event_any(
                    name,
                    &format!("{name}-{seq}"),
                ))),
            });
        }
        if !cover.correlation_id.is_empty() {
            agg.correlations.insert(cover.correlation_id.clone());
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
        agg.pages.push(EventPage {
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::Sequence(seq)),
                sync_mode: None,
            }),
            created_at: Some(at.unwrap_or_else(now)),
            payload: Some(event_page::Payload::Event(event_any(name, data))),
        });
        if !cover.correlation_id.is_empty() {
            agg.correlations.insert(cover.correlation_id.clone());
        }
    }

    /// Record a snapshot at `sequence` and continue history after it.
    pub fn seed_snapshot(&self, cover: &Cover, sequence: u32) {
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

/// Build a cover for `(domain, root label)` with optional edition and
/// correlation id.
pub fn cover(domain: &str, root_label: &str, edition: Option<&str>, correlation: &str) -> Cover {
    Cover {
        domain: domain.into(),
        root: Some(ProtoUuid {
            value: root_for(root_label).as_bytes().to_vec(),
        }),
        correlation_id: correlation.into(),
        edition: edition.map(|e| Edition {
            name: e.into(),
            ..Default::default()
        }),
        ..Default::default()
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
    let any = match &page.payload {
        Some(command_page::Payload::Command(a)) => a,
        _ => return Err(Status::invalid_argument("missing command payload")),
    };
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
            matches!(&p.payload, Some(event_page::Payload::Event(e)) if short_type(e) == "OrderShipped")
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

fn build_pages(decided: &Decided, first_seq: u32) -> Vec<EventPage> {
    decided
        .events
        .iter()
        .enumerate()
        .map(|(i, (name, data))| EventPage {
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::Sequence(first_seq + i as u32)),
                sync_mode: None,
            }),
            created_at: Some(now()),
            payload: Some(event_page::Payload::Event(event_any(name, data))),
        })
        .collect()
}

fn command_parts(book: Option<&CommandBook>) -> Result<(&Cover, &CommandPage), Status> {
    let book = book.ok_or_else(|| Status::invalid_argument("missing command book"))?;
    let cover = book
        .cover
        .as_ref()
        .ok_or_else(|| Status::invalid_argument("missing cover"))?;
    if cover.domain.is_empty() {
        return Err(Status::invalid_argument("domain is required"));
    }
    let page = book
        .pages
        .first()
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
    fn apply(&self, book: Option<&CommandBook>) -> Result<EventBook, Status> {
        let (cover, page) = command_parts(book)?;
        self.check_domain(&cover.domain)?;
        let mut store = self.0.store.lock().unwrap();
        let agg = store.entry(key_of(cover)).or_insert_with(|| Aggregate {
            cover: cover.clone(),
            ..Default::default()
        });
        let next = agg.next_sequence();
        if let Some(SequenceType::Sequence(n)) =
            page.header.as_ref().and_then(|h| h.sequence_type.as_ref())
        {
            if *n != next {
                return Err(Status::failed_precondition(format!(
                    "sequence mismatch: expected {next}, got {n}"
                )));
            }
        }
        let decided = decide(page, &agg.pages)?;
        let pages = build_pages(&decided, next);
        agg.pages.extend(pages.iter().cloned());
        if !cover.correlation_id.is_empty() {
            agg.correlations.insert(cover.correlation_id.clone());
        }
        Ok(EventBook {
            cover: Some(cover.clone()),
            next_sequence: agg.next_sequence(),
            pages,
            ..Default::default()
        })
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
            let seq = page_seq(page).unwrap_or(0);
            self.0.projector_runs.lock().unwrap().push((
                format!("{domain}-projector"),
                domain.clone(),
                seq,
            ));
            out.push(Projection {
                cover: events.cover.clone(),
                projector: format!("{domain}-projector"),
                sequence: seq,
                projection: None,
            });
        }
        out
    }

    fn run_sagas(&self, events: &EventBook) -> Result<(), Status> {
        let Some(source) = events.cover.as_ref() else {
            return Ok(());
        };
        let target = self
            .0
            .config
            .lock()
            .unwrap()
            .sagas
            .get(&source.domain)
            .cloned();
        let Some(target) = target else {
            return Ok(());
        };
        for page in &events.pages {
            let cmd = saga_command(source, page, &target);
            self.apply(Some(&cmd))?;
        }
        Ok(())
    }
}

/// The command a backend saga emits for one source event.
fn saga_command(source: &Cover, page: &EventPage, target: &str) -> CommandBook {
    CommandBook {
        cover: Some(Cover {
            domain: target.into(),
            root: source.root.clone(),
            correlation_id: source.correlation_id.clone(),
            ..Default::default()
        }),
        pages: vec![CommandPage {
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::AngzarrDeferred(AngzarrDeferredSequence {
                    source: Some(source.clone()),
                    source_seq: page_seq(page).unwrap_or(0),
                    ..Default::default()
                })),
                sync_mode: None,
            }),
            payload: Some(command_page::Payload::Command(command_any(
                "ReserveStock",
                &source.domain,
                1,
            ))),
            ..Default::default()
        }],
    }
}

#[tonic::async_trait]
impl CommandHandlerCoordinatorService for Svc {
    async fn handle_command(
        &self,
        request: Request<CommandRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        self.rpc("HandleCommand");
        let req = request.into_inner();
        self.0.sync_modes.lock().unwrap().push(req.sync_mode);
        let delay = self.0.config.lock().unwrap().response_delay;
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        let events = self.apply(req.command.as_ref())?;
        let mode = SyncMode::try_from(req.sync_mode).unwrap_or(SyncMode::Async);
        let projections = match mode {
            SyncMode::Simple => self.run_projectors(&events),
            SyncMode::Cascade => {
                let p = self.run_projectors(&events);
                self.run_sagas(&events)?;
                p
            }
            _ => {
                let svc = self.clone();
                let book = events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    svc.run_projectors(&book);
                    let _ = svc.run_sagas(&book);
                });
                Vec::new()
            }
        };
        Ok(Response::new(CommandResponse {
            events: Some(events),
            projections,
            ..Default::default()
        }))
    }

    async fn handle_event(
        &self,
        _request: Request<EventRequest>,
    ) -> Result<Response<FactInjectionResponse>, Status> {
        Err(Status::unimplemented("test backend does not inject facts"))
    }

    async fn handle_sync_speculative(
        &self,
        request: Request<SpeculateCommandHandlerRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        self.rpc("HandleSyncSpeculative");
        let req = request.into_inner();
        let (cover, page) = command_parts(req.command.as_ref())?;
        self.check_domain(&cover.domain)?;
        let history: Vec<EventPage> = {
            let store = self.0.store.lock().unwrap();
            store
                .get(&key_of(cover))
                .map(|a| a.pages.clone())
                .unwrap_or_default()
        };
        let visible: Vec<EventPage> = match req
            .point_in_time
            .as_ref()
            .and_then(|t| t.point_in_time.as_ref())
        {
            Some(PointInTime::AsOfSequence(s)) => history
                .into_iter()
                .filter(|p| page_seq(p).is_some_and(|q| q <= *s))
                .collect(),
            Some(PointInTime::AsOfTime(t)) => history
                .into_iter()
                .filter(|p| p.created_at.as_ref().is_some_and(|c| ts_le(c, t)))
                .collect(),
            None => history,
        };
        let next = visible.last().and_then(page_seq).map_or(0, |s| s + 1);
        let decided = decide(page, &visible)?;
        let pages = build_pages(&decided, next);
        Ok(Response::new(CommandResponse {
            events: Some(EventBook {
                cover: Some(cover.clone()),
                next_sequence: next + pages.len() as u32,
                pages,
                ..Default::default()
            }),
            projections: Vec::new(),
            ..Default::default()
        }))
    }

    async fn handle_compensation(
        &self,
        _request: Request<CommandRequest>,
    ) -> Result<Response<BusinessResponse>, Status> {
        Err(Status::unimplemented("test backend does not compensate"))
    }
}

type BookStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<EventBook, Status>> + Send>>;
type RootStream = Pin<Box<dyn tokio_stream::Stream<Item = Result<AggregateRoot, Status>> + Send>>;

impl Svc {
    fn select(&self, query: &Query) -> Result<Vec<EventBook>, Status> {
        let cover = query
            .cover
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing cover"))?;
        let store = self.0.store.lock().unwrap();
        if cover.root.is_none() && !cover.correlation_id.is_empty() {
            return Ok(store
                .values()
                .filter(|a| a.correlations.contains(&cover.correlation_id))
                .map(|a| EventBook {
                    cover: Some(a.cover.clone()),
                    pages: a.pages.clone(),
                    snapshot: a.snapshot.clone(),
                    next_sequence: a.next_sequence(),
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
                let seq = page_seq(p).unwrap_or(0);
                match &query.selection {
                    None => true,
                    Some(Selection::Range(r)) => seq >= r.lower && r.upper.is_none_or(|u| seq <= u),
                    Some(Selection::Sequences(s)) => s.values.contains(&seq),
                    Some(Selection::Temporal(t)) => match &t.point_in_time {
                        Some(PointInTime::AsOfSequence(s)) => seq <= *s,
                        Some(PointInTime::AsOfTime(at)) => {
                            p.created_at.as_ref().is_some_and(|c| ts_le(c, at))
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

    type SynchronizeStream = BookStream;

    async fn synchronize(
        &self,
        _request: Request<tonic::Streaming<Query>>,
    ) -> Result<Response<Self::SynchronizeStream>, Status> {
        Err(Status::unimplemented("test backend does not synchronize"))
    }

    type GetAggregateRootsStream = RootStream;

    async fn get_aggregate_roots(
        &self,
        _request: Request<()>,
    ) -> Result<Response<Self::GetAggregateRootsStream>, Status> {
        Err(Status::unimplemented("test backend does not list roots"))
    }
}

#[tonic::async_trait]
impl ProjectorCoordinatorService for Svc {
    async fn handle_sync(
        &self,
        _request: Request<EventRequest>,
    ) -> Result<Response<Projection>, Status> {
        Err(Status::unimplemented("test backend: HandleSync"))
    }

    async fn handle(&self, _request: Request<EventBook>) -> Result<Response<()>, Status> {
        Err(Status::unimplemented("test backend: Handle"))
    }

    /// The `order-summary` projector: projects the sequences it saw, in
    /// order, without recording a projector run.
    async fn handle_speculative(
        &self,
        request: Request<SpeculateProjectorRequest>,
    ) -> Result<Response<Projection>, Status> {
        self.rpc("ProjectorHandleSpeculative");
        let events = request
            .into_inner()
            .events
            .ok_or_else(|| Status::invalid_argument("missing events"))?;
        let seqs: Vec<String> = events
            .pages
            .iter()
            .filter_map(page_seq)
            .map(|s| s.to_string())
            .collect();
        Ok(Response::new(Projection {
            cover: events.cover.clone(),
            projector: "order-summary".into(),
            sequence: events.pages.last().and_then(page_seq).unwrap_or(0),
            projection: Some(event_any("OrderSummary", &seqs.join(","))),
        }))
    }
}

#[tonic::async_trait]
impl SagaCoordinatorService for Svc {
    async fn execute(
        &self,
        _request: Request<SagaHandleRequest>,
    ) -> Result<Response<SagaResponse>, Status> {
        Err(Status::unimplemented("test backend: Execute"))
    }

    /// Sagas translate every source event into a `ReserveStock` for the
    /// configured target (default: `inventory` for `orders`, `orders` for
    /// anything else) without delivering it.
    async fn execute_speculative(
        &self,
        request: Request<SpeculateSagaRequest>,
    ) -> Result<Response<SagaResponse>, Status> {
        self.rpc("SagaExecuteSpeculative");
        let source = request
            .into_inner()
            .request
            .and_then(|r| r.source)
            .ok_or_else(|| Status::invalid_argument("missing saga source"))?;
        let cover = source
            .cover
            .clone()
            .ok_or_else(|| Status::invalid_argument("missing source cover"))?;
        let target = if cover.domain == "orders" {
            "inventory"
        } else {
            "orders"
        };
        let commands = source
            .pages
            .iter()
            .map(|p| saga_command(&cover, p, target))
            .collect();
        Ok(Response::new(SagaResponse {
            commands,
            events: Vec::new(),
        }))
    }
}

#[tonic::async_trait]
impl ProcessManagerCoordinatorService for Svc {
    async fn handle(
        &self,
        _request: Request<ProcessManagerCoordinatorRequest>,
    ) -> Result<Response<ProcessManagerHandleResponse>, Status> {
        Err(Status::unimplemented("test backend: Handle"))
    }

    /// The `order-workflow` PM: requires a correlation id and emits one
    /// command per trigger event.
    async fn handle_speculative(
        &self,
        request: Request<SpeculatePmRequest>,
    ) -> Result<Response<ProcessManagerHandleResponse>, Status> {
        self.rpc("PmHandleSpeculative");
        let trigger = request
            .into_inner()
            .request
            .and_then(|r| r.trigger)
            .ok_or_else(|| Status::invalid_argument("missing trigger"))?;
        let cover = trigger
            .cover
            .clone()
            .ok_or_else(|| Status::invalid_argument("missing trigger cover"))?;
        if cover.correlation_id.is_empty() {
            return Err(Status::invalid_argument("correlation_id is required"));
        }
        let commands = trigger
            .pages
            .iter()
            .map(|p| saga_command(&cover, p, "shipping"))
            .collect();
        Ok(Response::new(ProcessManagerHandleResponse {
            commands,
            ..Default::default()
        }))
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
