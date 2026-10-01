//! The gRPC adapters run handler dispatch off the async worker threads.
//!
//! Each handler below blocks until a flag is set by a task spawned on the
//! same single-threaded runtime. If the adapter ran the synchronous
//! dispatch inline on that runtime's only worker, the task could never run
//! and the handler would give up; dispatched on the blocking pool, the task
//! runs and the handler completes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use angzarr_client::proto::command_handler_service_server::CommandHandlerService;
use angzarr_client::proto::process_manager_service_server::ProcessManagerService;
use angzarr_client::proto::projector_service_server::ProjectorService;
use angzarr_client::proto::saga_service_server::SagaService;
use angzarr_client::proto::upcaster_service_server::UpcasterService;
use angzarr_client::proto::{
    command_page, event_page, CommandBook, CommandPage, ContextualCommand, Cover, EventBook,
    EventPage, ProcessManagerHandleRequest, ProcessManagerHandleResponse, SagaHandleRequest,
    SagaResponse, UpcastRequest,
};
use angzarr_client::router::{Built, Router};
#[allow(unused_imports)]
use angzarr_client::{
    command_handler, full_type_url, handles, process_manager, projector, saga, upcaster, upcasts,
    CommandHandlerGrpc, CommandRejectedError, CommandResult, ProcessManagerGrpc, ProjectorGrpc,
    SagaGrpc, UpcasterGrpc,
};
use prost_types::Any;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Ping {
    #[prost(string, tag = "1")]
    id: String,
}

impl ::prost::Name for Ping {
    const NAME: &'static str = "Ping";
    const PACKAGE: &'static str = "blocking";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct Pong {
    #[prost(string, tag = "1")]
    id: String,
}

impl ::prost::Name for Pong {
    const NAME: &'static str = "Pong";
    const PACKAGE: &'static str = "blocking";
}

/// Blocks the calling thread until `flag` is set or two seconds pass.
fn wait_for(flag: &AtomicBool) -> Result<(), CommandRejectedError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !flag.load(Ordering::SeqCst) {
        if Instant::now() > deadline {
            return Err(CommandRejectedError::precondition_failed(
                "BLOCKED_RUNTIME",
                "handler ran on the async worker",
                std::iter::empty::<(String, String)>(),
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

static UPCAST_FLAG: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct S;

struct BlockingAggregate(Arc<AtomicBool>);

#[command_handler(domain = "blocking", state = S)]
impl BlockingAggregate {
    #[handles(Ping)]
    fn on_ping(&self, _cmd: Ping, _state: &S, _seq: u32) -> CommandResult<EventBook> {
        wait_for(&self.0)?;
        Ok(EventBook::default())
    }
}

struct BlockingSaga(Arc<AtomicBool>);

#[saga(name = "blocking-saga", source = "blocking", target = "other")]
impl BlockingSaga {
    #[handles(Ping)]
    fn on_ping(&self, _evt: Ping) -> CommandResult<SagaResponse> {
        wait_for(&self.0)?;
        Ok(SagaResponse::default())
    }
}

struct BlockingPm(Arc<AtomicBool>);

#[process_manager(
    name = "blocking-pm",
    pm_domain = "blocking-pm",
    sources = ["blocking"],
    targets = ["other"],
    state = S
)]
impl BlockingPm {
    #[handles(Ping)]
    fn on_ping(&self, _evt: Ping, _state: &S) -> CommandResult<ProcessManagerHandleResponse> {
        wait_for(&self.0)?;
        Ok(ProcessManagerHandleResponse::default())
    }
}

struct BlockingProjector(Arc<AtomicBool>);

#[projector(name = "blocking-projector", domains = ["blocking"])]
impl BlockingProjector {
    #[handles(Ping)]
    fn on_ping(&self, _evt: Ping) -> CommandResult<()> {
        wait_for(&self.0)
    }
}

struct BlockingUpcaster;

#[upcaster(name = "blocking-upcaster", domain = "blocking")]
impl BlockingUpcaster {
    #[upcasts(from = Ping, to = Pong)]
    fn upgrade(old: Ping) -> Pong {
        let id = match wait_for(&UPCAST_FLAG) {
            Ok(()) => old.id,
            Err(_) => "blocked".to_string(),
        };
        Pong { id }
    }
}

fn ping_any() -> Any {
    Any {
        type_url: full_type_url::<Ping>(),
        value: ::prost::Message::encode_to_vec(&Ping { id: "p".into() }),
    }
}

fn ping_book() -> EventBook {
    EventBook {
        cover: Some(Cover {
            domain: "blocking".into(),
            ..Default::default()
        }),
        pages: vec![EventPage {
            payload: Some(event_page::Payload::Event(ping_any())),
            ..Default::default()
        }],
        next_sequence: 1,
        ..Default::default()
    }
}

/// Spawn the task that releases the handler, then yield once so it is
/// queued behind the request.
fn release_later(flag: Arc<AtomicBool>) {
    tokio::spawn(async move {
        flag.store(true, Ordering::SeqCst);
    });
}

#[tokio::test(flavor = "current_thread")]
async fn command_handler_dispatch_does_not_block_the_runtime() {
    let flag = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&flag);
    let Built::CommandHandler(router) = Router::new("blocking")
        .with_handler(move || BlockingAggregate(Arc::clone(&f)))
        .build()
        .expect("build")
    else {
        panic!("expected a command-handler router");
    };
    let svc = CommandHandlerGrpc::new(router);
    release_later(Arc::clone(&flag));
    let cmd = ContextualCommand {
        command: Some(CommandBook {
            cover: Some(Cover {
                domain: "blocking".into(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(ping_any())),
                ..Default::default()
            }],
        }),
        events: None,
    };
    svc.handle(tonic::Request::new(cmd)).await.expect("handled");
}

#[tokio::test(flavor = "current_thread")]
async fn saga_dispatch_does_not_block_the_runtime() {
    let flag = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&flag);
    let Built::Saga(router) = Router::new("blocking")
        .with_handler(move || BlockingSaga(Arc::clone(&f)))
        .build()
        .expect("build")
    else {
        panic!("expected a saga router");
    };
    let svc = SagaGrpc::new(router);
    release_later(Arc::clone(&flag));
    let req = SagaHandleRequest {
        source: Some(ping_book()),
        ..Default::default()
    };
    svc.handle(tonic::Request::new(req)).await.expect("handled");
}

#[tokio::test(flavor = "current_thread")]
async fn process_manager_dispatch_does_not_block_the_runtime() {
    let flag = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&flag);
    let Built::ProcessManager(router) = Router::new("blocking")
        .with_handler(move || BlockingPm(Arc::clone(&f)))
        .build()
        .expect("build")
    else {
        panic!("expected a PM router");
    };
    let svc = ProcessManagerGrpc::new(router);
    release_later(Arc::clone(&flag));
    let req = ProcessManagerHandleRequest {
        trigger: Some(ping_book()),
        ..Default::default()
    };
    svc.handle(tonic::Request::new(req)).await.expect("handled");
}

#[tokio::test(flavor = "current_thread")]
async fn projector_dispatch_does_not_block_the_runtime() {
    let flag = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&flag);
    let Built::Projector(router) = Router::new("blocking")
        .with_handler(move || BlockingProjector(Arc::clone(&f)))
        .build()
        .expect("build")
    else {
        panic!("expected a projector router");
    };
    let svc = ProjectorGrpc::new(router);
    release_later(Arc::clone(&flag));
    svc.handle(tonic::Request::new(ping_book()))
        .await
        .expect("handled");
    flag.store(false, Ordering::SeqCst);
    release_later(Arc::clone(&flag));
    svc.handle_speculative(tonic::Request::new(ping_book()))
        .await
        .expect("handled speculatively");
}

#[tokio::test(flavor = "current_thread")]
async fn upcaster_dispatch_does_not_block_the_runtime() {
    let Built::Upcaster(router) = Router::new("blocking")
        .with_handler(|| BlockingUpcaster)
        .build()
        .expect("build")
    else {
        panic!("expected an upcaster router");
    };
    let svc = UpcasterGrpc::new(router);
    tokio::spawn(async {
        UPCAST_FLAG.store(true, Ordering::SeqCst);
    });
    let resp = svc
        .upcast(tonic::Request::new(UpcastRequest {
            domain: "blocking".into(),
            events: ping_book().pages,
        }))
        .await
        .expect("upcast")
        .into_inner();
    let Some(event_page::Payload::Event(any)) = &resp.events[0].payload else {
        panic!("expected an event");
    };
    let pong: Pong = ::prost::Message::decode(any.value.as_slice()).expect("decode");
    assert_eq!(pong.id, "p");
}

struct PanickingProjector;

#[projector(name = "panicking-projector", domains = ["blocking"])]
impl PanickingProjector {
    #[handles(Ping)]
    fn on_ping(&self, _evt: Ping) -> CommandResult<()> {
        panic!("projector bug");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn panicking_handler_maps_to_internal() {
    let Built::Projector(router) = Router::new("blocking")
        .with_handler(|| PanickingProjector)
        .build()
        .expect("build")
    else {
        panic!("expected a projector router");
    };
    let svc = ProjectorGrpc::new(router);
    let status = svc
        .handle(tonic::Request::new(ping_book()))
        .await
        .expect_err("panic surfaces as a status");
    assert_eq!(status.code(), tonic::Code::Internal);
    assert_eq!(
        status.message(),
        angzarr_client::error_codes::messages::HANDLER_PANICKED
    );
    assert!(!status.details().is_empty(), "canonical details trailer");
}
