//! R6 — single-handler command dispatch.
//!
//! Sends a `ContextualCommand` through `CommandHandlerRouter::dispatch` and
//! verifies the handler's emitted `EventBook` round-trips back as a
//! `BusinessResponse::Events` variant.

use angzarr_client::proto::{
    business_response, command_page, BusinessResponse, CommandBook, CommandPage, ContextualCommand,
    EventBook, EventPage,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{aggregate, full_type_url, CommandResult};

use prost::Message;
use prost_types::Any;

// Test-local proto stub — empty message, carries only metadata.
#[derive(Clone, PartialEq, ::prost::Message)]
struct RegisterPlayer {}

impl ::prost::Name for RegisterPlayer {
    const NAME: &'static str = "RegisterPlayer";
    const PACKAGE: &'static str = "test";
}

#[derive(Default)]
struct PlayerState;

struct Player;

#[aggregate(domain = "player", state = PlayerState)]
impl Player {
    #[handles(RegisterPlayer)]
    #[allow(unused_variables, dead_code)]
    fn register(
        &self,
        cmd: RegisterPlayer,
        state: &PlayerState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        // Emit one empty-page EventBook so the test can observe dispatch reached the method.
        Ok(EventBook {
            pages: vec![EventPage::default()],
            ..Default::default()
        })
    }
}

fn make_register_command() -> ContextualCommand {
    let any = Any {
        type_url: full_type_url::<RegisterPlayer>(),
        value: RegisterPlayer {}.encode_to_vec(),
    };
    let page = CommandPage {
        payload: Some(command_page::Payload::Command(any)),
        ..Default::default()
    };
    let book = CommandBook {
        pages: vec![page],
        ..Default::default()
    };
    ContextualCommand {
        command: Some(book),
        events: Some(EventBook::default()),
    }
}

#[test]
fn single_handler_dispatch_returns_events_variant() {
    let built = Router::new("agg-player")
        .with_handler(|| Player)
        .build()
        .unwrap();
    let Built::CommandHandler(router) = built else {
        panic!("expected CommandHandler");
    };

    let response: BusinessResponse = router.dispatch(make_register_command()).unwrap();

    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected BusinessResponse::Events, got {:?}", response.result);
    };
    assert_eq!(book.pages.len(), 1, "handler must have been invoked once");
}
