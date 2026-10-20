//! Composition through the client builder: rejections keep their cover,
//! and a command handler is probed once at build.

use std::sync::Arc;

use angzarr_client::proto::{
    command_page, Command, CommandBook, CommandPage, ContextualCommand, Cover, EventBook,
};
#[allow(unused_imports)]
use angzarr_client::router::{command_handler, handles};
use angzarr_client::router::{Built, Router};
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
msg!(Charge);

fn pack<M: ::prost::Message + ::prost::Name>(m: &M) -> Any {
    Any {
        type_url: full_type_url::<M>(),
        value: ::prost::Message::encode_to_vec(m),
    }
}

#[derive(Default)]
struct NoState;

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

/// A command handler is instantiated once at build (C-0065).
#[test]
fn a_command_handler_is_probed_once_at_build() {
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
