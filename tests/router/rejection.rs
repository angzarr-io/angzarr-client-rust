//! R10 — rejection handler routing.
//!
//! When an aggregate's emitted command is rejected by its target, the
//! framework sends a `Notification` back. Methods annotated with
//! `#[rejected(domain = "...", command = "...")]` receive this, matched by
//! the rejected command's (domain, command_suffix). Multiple matching
//! handlers all run; their compensation events concatenate.

use angzarr_client::proto::{
    business_response, command_page, BusinessResponse, CommandBook, CommandPage, ContextualCommand,
    Cover, EventBook, EventPage, Notification, RejectionNotification,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{aggregate, full_type_url, CommandResult};

use prost::Message;
use prost_types::Any;

// Placeholder for the target aggregate's rejected command.
#[derive(Clone, PartialEq, ::prost::Message)]
struct Forbidden {}
impl ::prost::Name for Forbidden {
    const NAME: &'static str = "Forbidden";
    const PACKAGE: &'static str = "target";
}

#[derive(Default)]
struct State;

struct Player;

#[aggregate(domain = "player", state = State)]
impl Player {
    #[rejected(domain = "target", command = "Forbidden")]
    #[allow(unused_variables, dead_code)]
    fn on_rejected(
        &self,
        notif: &Notification,
        state: &State,
    ) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(EventBook {
                pages: vec![tagged_page("compensation")],
                ..Default::default()
            })),
        })
    }
}

fn tagged_page(tag: &str) -> EventPage {
    use angzarr_client::proto::event_page;
    EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: tag.to_string(),
            value: vec![],
        })),
        ..Default::default()
    }
}

/// Wrap a `Notification` in a `ContextualCommand` the router can dispatch.
fn notification_command(notif: Notification) -> ContextualCommand {
    let any = Any {
        type_url: full_type_url::<Notification>(),
        value: notif.encode_to_vec(),
    };
    ContextualCommand {
        command: Some(CommandBook {
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(any)),
                ..Default::default()
            }],
            ..Default::default()
        }),
        events: Some(EventBook::default()),
    }
}

/// Build a Notification that reports a rejection of `target::Forbidden`.
fn rejection_of_forbidden() -> Notification {
    let rejected_cmd_any = Any {
        type_url: full_type_url::<Forbidden>(),
        value: Forbidden {}.encode_to_vec(),
    };
    let rejected_command = CommandBook {
        cover: Some(Cover {
            domain: "target".to_string(),
            ..Default::default()
        }),
        pages: vec![CommandPage {
            payload: Some(command_page::Payload::Command(rejected_cmd_any)),
            ..Default::default()
        }],
    };
    let rejection = RejectionNotification {
        rejected_command: Some(rejected_command),
        rejection_reason: "test".to_string(),
    };
    let rejection_any = Any {
        type_url: full_type_url::<RejectionNotification>(),
        value: rejection.encode_to_vec(),
    };
    Notification {
        payload: Some(rejection_any),
        ..Default::default()
    }
}

#[test]
fn rejected_handler_receives_notification_and_emits_compensation() {
    let built = Router::new("agg-player")
        .with_handler(|| Player)
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    let response = ch
        .dispatch(notification_command(rejection_of_forbidden()))
        .unwrap();
    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected Events, got {:?}", response.result);
    };
    assert_eq!(book.pages.len(), 1);
}

// Second handler registered for the SAME rejection key — both must run.
struct PlayerB;

#[aggregate(domain = "player", state = State)]
impl PlayerB {
    #[rejected(domain = "target", command = "Forbidden")]
    #[allow(unused_variables, dead_code)]
    fn on_rejected(
        &self,
        notif: &Notification,
        state: &State,
    ) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse {
            result: Some(business_response::Result::Events(EventBook {
                pages: vec![tagged_page("compensation-B"), tagged_page("extra-B")],
                ..Default::default()
            })),
        })
    }
}

#[test]
fn multiple_rejection_handlers_for_same_key_all_run_and_merge() {
    let built = Router::new("agg-player")
        .with_handler(|| Player) // emits 1 page tagged "compensation"
        .with_handler(|| PlayerB) // emits 2 pages tagged "compensation-B", "extra-B"
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    let response = ch
        .dispatch(notification_command(rejection_of_forbidden()))
        .unwrap();
    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected Events");
    };
    let tags: Vec<String> = book
        .pages
        .into_iter()
        .map(|p| match p.payload {
            Some(angzarr_client::proto::event_page::Payload::Event(any)) => any.type_url,
            _ => "?".to_string(),
        })
        .collect();
    assert_eq!(
        tags,
        vec![
            "compensation".to_string(),
            "compensation-B".to_string(),
            "extra-B".to_string()
        ]
    );
}

#[test]
fn rejection_with_no_matching_handler_returns_empty_events() {
    struct UnrelatedPlayer;
    #[aggregate(domain = "unrelated", state = State)]
    impl UnrelatedPlayer {
        #[rejected(domain = "something-else", command = "OtherCmd")]
        #[allow(unused_variables, dead_code)]
        fn on_rejected(
            &self,
            notif: &Notification,
            state: &State,
        ) -> CommandResult<BusinessResponse> {
            Ok(BusinessResponse::default())
        }
    }

    let built = Router::new("agg-unrelated")
        .with_handler(|| UnrelatedPlayer)
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    let response = ch
        .dispatch(notification_command(rejection_of_forbidden()))
        .unwrap();
    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected empty Events");
    };
    assert!(book.pages.is_empty());
}
