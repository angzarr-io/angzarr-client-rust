//! Composition through the client builder: build-time validation and the
//! routing that depends on each component's identity.

use std::sync::{Arc, Mutex};

use angzarr_client::proto::{
    business_response, command_page, event_page, page_header::SequenceType,
    AngzarrDeferredSequence, BusinessResponse, CommandBook, CommandPage, ContextualCommand, Cover,
    EventBook, EventPage, Notification, PageHeader, ProcessManagerHandleRequest,
    ProcessManagerHandleResponse, RejectionNotification,
};
#[allow(unused_imports)]
use angzarr_client::router::{command_handler, handles, process_manager, rejected};
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
msg!(ReserveStock);
msg!(ProcessPayment);
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

fn released(by: &'static str, log: &Log) -> CommandResult<BusinessResponse> {
    log.lock().unwrap().push(by);
    Ok(BusinessResponse {
        result: Some(business_response::Result::Events(EventBook::default())),
    })
}

struct Ambiguous;
#[command_handler(domain = "payment", state = NoState)]
impl Ambiguous {
    #[rejected(command = ReserveStock)]
    fn any(&self, _n: &Notification, _s: &NoState) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse::default())
    }
    #[rejected(domain = "inventory", command = ReserveStock)]
    fn inventory(&self, _n: &Notification, _s: &NoState) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse::default())
    }
}

#[test]
fn ambiguous_compensates_entries_fail_the_build() {
    let err = Router::new("payment")
        .with_handler(|| Ambiguous)
        .build()
        .expect_err("ambiguous compensates");
    assert!(
        matches!(err, BuildError::InvalidComponent(_)),
        "got {err:?}"
    );
    assert_eq!(err.code(), "AMBIGUOUS_COMPENSATION");
}

struct StockComp(Log);
#[command_handler(domain = "payment", state = NoState)]
impl StockComp {
    #[rejected(domain = "inventory", command = ReserveStock)]
    fn on_stock(&self, _n: &Notification, _s: &NoState) -> CommandResult<BusinessResponse> {
        released("stock", &self.0)
    }
}

struct PaymentComp(Log);
#[command_handler(domain = "payment", state = NoState)]
impl PaymentComp {
    #[handles(Charge)]
    fn charge(&self, _c: Charge, _s: &NoState, _seq: u32) -> CommandResult<EventBook> {
        Err(CommandRejectedError::precondition_failed(
            "CARD_DECLINED",
            "card declined",
            std::iter::empty::<(String, String)>(),
        ))
    }

    #[rejected(domain = "payment", command = ProcessPayment)]
    fn on_payment(&self, _n: &Notification, _s: &NoState) -> CommandResult<BusinessResponse> {
        released("payment", &self.0)
    }
}

fn payment_router(log: &Log) -> angzarr_client::router::CommandHandlerRouter {
    let (a, b) = (Arc::clone(log), Arc::clone(log));
    match Router::new("payments")
        .with_handler(move || StockComp(Arc::clone(&a)))
        .with_handler(move || PaymentComp(Arc::clone(&b)))
        .build()
    {
        Ok(Built::CommandHandler(r)) => r,
        other => panic!("expected a command-handler router, got {other:?}"),
    }
}

fn rejection_of<M: ::prost::Message + ::prost::Name>(cmd: &M, sent_to: &str) -> ContextualCommand {
    let notification = Notification {
        payload: Some(pack(&RejectionNotification {
            rejected_command: Some(CommandBook {
                cover: Some(Cover {
                    domain: sent_to.into(),
                    ..Default::default()
                }),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(pack(cmd))),
                    ..Default::default()
                }],
            }),
            rejection_reason: "no".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    ContextualCommand {
        command: Some(CommandBook {
            cover: Some(Cover {
                domain: "payment".into(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(pack(&notification))),
                ..Default::default()
            }],
        }),
        events: None,
    }
}

/// A rejection goes to the aggregate of the domain that declares a
/// compensation for it, not merely the first one registered.
#[test]
fn a_rejection_reaches_the_aggregate_that_compensates_it() {
    let log: Log = Arc::default();
    let router = payment_router(&log);
    router
        .dispatch(rejection_of(&ProcessPayment {}, "payment"))
        .expect("dispatch");
    assert_eq!(*log.lock().unwrap(), vec!["payment"]);
    router
        .dispatch(rejection_of(&ReserveStock {}, "inventory"))
        .expect("dispatch");
    assert_eq!(*log.lock().unwrap(), vec!["payment", "stock"]);
    assert_eq!(router.name(), "payment");
    assert!(router.output_domains().is_empty());
    assert_eq!(router.handler_count(), 2);
}

/// A handler's rejection reaches the caller with the request's cover; a
/// cover the handler set itself is kept.
#[test]
fn rejections_carry_the_request_cover() {
    let log: Log = Arc::default();
    let router = payment_router(&log);
    let cover = Cover {
        domain: "payment".into(),
        correlation_id: "corr-9".into(),
        ..Default::default()
    };
    let err = router
        .dispatch(ContextualCommand {
            command: Some(CommandBook {
                cover: Some(cover.clone()),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(pack(&Charge {}))),
                    ..Default::default()
                }],
            }),
            events: None,
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
    fn charge(&self, _c: Charge, _s: &NoState, _seq: u32) -> CommandResult<EventBook> {
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
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(pack(&Charge {}))),
                    ..Default::default()
                }],
            }),
            events: None,
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

    #[rejected(command = ReserveStock)]
    fn on_reserve_rejected(
        &self,
        _n: &Notification,
        _s: &NoState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        self.0.lock().unwrap().push("pm-b compensates");
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
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::Sequence(0)),
                sync_mode: None,
            }),
            payload: Some(event_page::Payload::Event(any)),
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

/// A rejection goes to the PM named as the rejected command's issuer.
#[test]
fn a_rejection_routes_to_its_issuing_process_manager() {
    let log: Log = Arc::default();
    let router = pm_router(&log);
    let notification = Notification {
        payload: Some(pack(&RejectionNotification {
            rejected_command: Some(CommandBook {
                cover: Some(Cover {
                    domain: "inventory".into(),
                    ..Default::default()
                }),
                pages: vec![CommandPage {
                    header: Some(PageHeader {
                        sequence_type: Some(SequenceType::AngzarrDeferred(
                            AngzarrDeferredSequence {
                                source_component: "pm-b".into(),
                                ..Default::default()
                            },
                        )),
                        sync_mode: None,
                    }),
                    payload: Some(command_page::Payload::Command(pack(&ReserveStock {}))),
                    ..Default::default()
                }],
            }),
            rejection_reason: "no stock".into(),
            ..Default::default()
        })),
        ..Default::default()
    };
    router
        .dispatch(ProcessManagerHandleRequest {
            trigger: Some(order_trigger(pack(&notification))),
            process_state: None,
        })
        .expect("dispatch");
    assert_eq!(*log.lock().unwrap(), vec!["pm-b compensates"]);
}

struct AmbiguousPm;
#[process_manager(name = "amb", pm_domain = "amb", state = NoState, sources = ["order"], targets = ["inventory"])]
impl AmbiguousPm {
    #[handles(OrderCreated)]
    fn on_created(
        &self,
        _e: OrderCreated,
        _s: &NoState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
    #[rejected(command = ReserveStock)]
    fn any(&self, _n: &Notification, _s: &NoState) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
    #[rejected(domain = "inventory", command = ReserveStock)]
    fn inventory(
        &self,
        _n: &Notification,
        _s: &NoState,
    ) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
}

#[test]
fn ambiguous_process_manager_compensation_fails_the_build() {
    let err = Router::new("pms")
        .with_handler(|| AmbiguousPm)
        .build()
        .expect_err("ambiguous compensates");
    assert_eq!(err.code(), "AMBIGUOUS_COMPENSATION");
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
            PaymentComp(Arc::default())
        })
        .with_handler(move || {
            p2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            PaymentComp(Arc::default())
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
            PaymentComp(Arc::default())
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

/// What a `#[rejected]` handler read from the rejection it received.
type Seen = Arc<Mutex<Option<(String, String)>>>;

struct CodeReader(Seen);
#[command_handler(domain = "payment", state = NoState)]
impl CodeReader {
    #[rejected(domain = "payment", command = ProcessPayment)]
    fn on_payment(&self, n: &Notification, _s: &NoState) -> CommandResult<BusinessResponse> {
        let rejection: RejectionNotification =
            angzarr_client::unpack(n.payload.as_ref().expect("notification payload"))
                .expect("rejection notification");
        *self.0.lock().unwrap() = Some((rejection.code, rejection.rejection_reason));
        Ok(BusinessResponse::default())
    }
}

/// The rejecting handler's machine code reaches the compensator next to,
/// and separate from, the human message.
#[test]
fn a_rejected_handler_reads_the_code_apart_from_the_message() {
    let seen: Seen = Arc::default();
    let s = Arc::clone(&seen);
    let Built::CommandHandler(router) = Router::new("payments")
        .with_handler(move || CodeReader(Arc::clone(&s)))
        .build()
        .expect("build")
    else {
        panic!("expected a command-handler router");
    };
    let notification = Notification {
        payload: Some(pack(&RejectionNotification {
            rejected_command: Some(CommandBook {
                cover: Some(Cover {
                    domain: "payment".into(),
                    ..Default::default()
                }),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(pack(&ProcessPayment {}))),
                    ..Default::default()
                }],
            }),
            rejection_reason: "card was declined by the issuer".into(),
            code: "CARD_DECLINED".into(),
        })),
        ..Default::default()
    };
    router
        .dispatch(ContextualCommand {
            command: Some(CommandBook {
                cover: Some(Cover {
                    domain: "payment".into(),
                    ..Default::default()
                }),
                pages: vec![CommandPage {
                    payload: Some(command_page::Payload::Command(pack(&notification))),
                    ..Default::default()
                }],
            }),
            events: None,
        })
        .expect("dispatch");
    let (code, message) = seen.lock().unwrap().clone().expect("handler ran");
    assert_eq!(code, "CARD_DECLINED");
    assert_eq!(message, "card was declined by the issuer");
}
