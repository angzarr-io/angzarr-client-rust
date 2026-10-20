//! Composition through the client builder: build-time validation and the
//! routing that depends on each component's identity.

use std::sync::{Arc, Mutex};

use angzarr_client::proto::{
    command_page, Command, CommandBook, CommandPage, ContextualCommand, Cover, EventBook,
    EventPage, ProcessManagerHandleRequest, ProcessManagerHandleResponse,
};
#[allow(unused_imports)]
use angzarr_client::router::{command_handler, handles, process_manager};
use angzarr_client::router::{BuildError, Built, Router};
#[allow(unused_imports)]
use angzarr_client::{full_type_url, ClientError, CommandRejectedError, CommandResult};
use prost_types::Any;

macro_rules! msg {
    ($name:ident) => {
        #[derive(Clone, PartialEq, ::prost::Message)]
        struct $name {}
        impl ::prost::Name for $name {
            const NAME: &'static str = stringify!($name);
            const PACKAGE: &'static str = "composition";
        }
    };
}
msg!(OrderCreated);
msg!(Charge);

fn pack<M: ::prost::Message + ::prost::Name>(m: &M) -> Any {
    Any {
        type_url: full_type_url::<M>(),
        value: ::prost::Message::encode_to_vec(m),
    }
}

#[derive(Default)]
struct NoState;

type Log = Arc<Mutex<Vec<&'static str>>>;

struct PaymentComp;
#[command_handler(domain = "payment", state = NoState)]
impl PaymentComp {
    #[handles(Charge)]
    fn charge(&self, _c: Charge, _s: &NoState, _seq: u64) -> CommandResult<EventBook> {
        Err(CommandRejectedError::precondition_failed(
            "CARD_DECLINED",
            "card declined",
            std::iter::empty::<(String, String)>(),
        ))
    }
}

fn payment_router() -> angzarr_client::router::CommandHandlerRouter {
    match Router::new("payments").with_handler(|| PaymentComp).build() {
        Ok(Built::CommandHandler(r)) => r,
        other => panic!("expected a command-handler router, got {other:?}"),
    }
}

/// A handler's rejection reaches the caller with the request's cover; a
/// cover the handler set itself is kept.
#[test]
fn rejections_carry_the_request_cover() {
    let router = payment_router();
    assert_eq!(router.name(), "payment");
    assert!(router.output_domains().is_empty());
    assert_eq!(router.handler_count(), 1);
    let cover = Cover {
        domain: "payment".into(),
        edition: Some("trial".into()),
        ..Default::default()
    };
    let err = router
        .dispatch(ContextualCommand {
            command: Some(CommandBook {
                cover: Some(cover.clone()),
                page: Some(CommandPage {
                    body: Some(command_page::Body::Command(Command {
                        payload: Some(pack(&Charge {})),
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            events: None,
            ..Default::default()
        })
        .expect_err("card declined");
    let ClientError::Rejected(rej) = err else {
        panic!("expected the handler's rejection, got {err:?}");
    };
    assert_eq!(rej.code, "CARD_DECLINED");
    assert_eq!(rej.status_code, "FAILED_PRECONDITION");
    assert_eq!(rej.cover.as_deref(), Some(&cover));
}

struct SelfAddressed;
#[command_handler(domain = "payment", state = NoState)]
impl SelfAddressed {
    #[handles(Charge)]
    fn charge(&self, _c: Charge, _s: &NoState, _seq: u64) -> CommandResult<EventBook> {
        Err(CommandRejectedError::precondition_failed(
            "CARD_DECLINED",
            "card declined",
            std::iter::empty::<(String, String)>(),
        )
        .with_cover(Cover {
            domain: "handler-set".into(),
            ..Default::default()
        }))
    }
}

#[test]
fn a_handler_set_rejection_cover_is_kept() {
    let Ok(Built::CommandHandler(router)) = Router::new("payment")
        .with_handler(|| SelfAddressed)
        .build()
    else {
        panic!("command-handler router");
    };
    let err = router
        .dispatch(ContextualCommand {
            command: Some(CommandBook {
                cover: Some(Cover {
                    domain: "payment".into(),
                    ..Default::default()
                }),
                page: Some(CommandPage {
                    body: Some(command_page::Body::Command(Command {
                        payload: Some(pack(&Charge {})),
                    })),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            events: None,
            ..Default::default()
        })
        .expect_err("declined");
    let ClientError::Rejected(rej) = err else {
        panic!("expected a rejection, got {err:?}");
    };
    assert_eq!(rej.cover.map(|c| c.domain), Some("handler-set".to_string()));
}

struct PmA(Log);
#[process_manager(name = "pm-a", pm_domain = "flow-a", state = NoState, sources = ["order"], targets = ["inventory"])]
impl PmA {
    #[handles(OrderCreated)]
    fn on_created(
        &self,
        _e: OrderCreated,
        _s: &NoState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        self.0.lock().unwrap().push("pm-a");
        Ok(ProcessManagerHandleResponse::default())
    }
}

struct PmB(Log);
#[process_manager(name = "pm-b", pm_domain = "flow-b", state = NoState, sources = ["order"], targets = ["inventory"])]
impl PmB {
    #[handles(OrderCreated)]
    fn on_created(
        &self,
        _e: OrderCreated,
        _s: &NoState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        self.0.lock().unwrap().push("pm-b");
        Ok(ProcessManagerHandleResponse::default())
    }
}

fn pm_router(log: &Log) -> angzarr_client::router::ProcessManagerRouter {
    let (a, b) = (Arc::clone(log), Arc::clone(log));
    match Router::new("pms")
        .with_handler(move || PmA(Arc::clone(&a)))
        .with_handler(move || PmB(Arc::clone(&b)))
        .build()
    {
        Ok(Built::ProcessManager(r)) => r,
        other => panic!("expected a PM router, got {other:?}"),
    }
}

fn order_trigger(any: Any) -> EventBook {
    EventBook {
        cover: Some(Cover {
            domain: "order".into(),
            ..Default::default()
        }),
        pages: vec![EventPage {
            sequence: 0,
            payload: Some(any),
            ..Default::default()
        }],
        next_sequence: 1,
        ..Default::default()
    }
}

/// A process-state book names its PM; a new workflow reaches every PM
/// consuming the trigger domain.
#[test]
fn process_state_routes_to_its_own_process_manager() {
    let log: Log = Arc::default();
    let router = pm_router(&log);
    router
        .dispatch(ProcessManagerHandleRequest {
            trigger: Some(order_trigger(pack(&OrderCreated {}))),
            process_state: Some(EventBook {
                cover: Some(Cover {
                    domain: "flow-b".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        })
        .expect("dispatch");
    assert_eq!(*log.lock().unwrap(), vec!["pm-b"]);
    router
        .dispatch(ProcessManagerHandleRequest {
            trigger: Some(order_trigger(pack(&OrderCreated {}))),
            process_state: None,
        })
        .expect("dispatch");
    assert_eq!(*log.lock().unwrap(), vec!["pm-b", "pm-a", "pm-b"]);
}

/// Command handlers are checked for duplicate (domain, command) claims and
/// instantiated once at build; other kinds are not instantiated.
#[test]
fn command_handlers_are_checked_and_probed_at_build() {
    let probes = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let (p1, p2) = (Arc::clone(&probes), Arc::clone(&probes));
    let err = Router::new("dup")
        .with_handler(move || {
            p1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            PaymentComp
        })
        .with_handler(move || {
            p2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            PaymentComp
        })
        .build()
        .expect_err("duplicate claim");
    assert!(
        matches!(err, BuildError::DuplicateCommandHandler(_)),
        "got {err:?}"
    );
    assert_eq!(
        err.details().get("type_url"),
        Some(&full_type_url::<Charge>())
    );

    // The first handler was probed before the duplicate was found.
    assert_eq!(probes.load(std::sync::atomic::Ordering::SeqCst), 1);

    let probed = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let p = Arc::clone(&probed);
    let built = Router::new("one")
        .with_handler(move || {
            p.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            PaymentComp
        })
        .build();
    assert!(matches!(built, Ok(Built::CommandHandler(_))));
    assert_eq!(probed.load(std::sync::atomic::Ordering::SeqCst), 1);
}

struct BillingPm(Log);
#[process_manager(name = "billing-pm", pm_domain = "billing-flow", state = NoState, sources = ["billing"], targets = ["inventory"])]
impl BillingPm {
    #[handles(OrderCreated)]
    fn on_created(
        &self,
        _e: OrderCreated,
        _s: &NoState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        self.0.lock().unwrap().push("billing-pm");
        Ok(ProcessManagerHandleResponse::default())
    }
}

#[test]
fn a_new_workflow_reaches_only_process_managers_consuming_its_domain() {
    let log: Log = Arc::default();
    let (a, b) = (Arc::clone(&log), Arc::clone(&log));
    let Ok(Built::ProcessManager(router)) = Router::new("pms")
        .with_handler(move || BillingPm(Arc::clone(&a)))
        .with_handler(move || PmA(Arc::clone(&b)))
        .build()
    else {
        panic!("PM router");
    };
    router
        .dispatch(ProcessManagerHandleRequest {
            trigger: Some(order_trigger(pack(&OrderCreated {}))),
            process_state: None,
        })
        .expect("dispatch");
    assert_eq!(*log.lock().unwrap(), vec!["pm-a"]);
}
