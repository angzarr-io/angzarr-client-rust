//! R8 — multi-handler merge.
//!
//! Two aggregate factories registered in the same domain, both claiming
//! `#[handles(Cmd)]`, must BOTH run on dispatch. Their emitted EventBooks
//! concatenate in registration order and each handler rebuilds its own state.
//!
//! Factory invocation count per dispatch: one per matched handler.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use angzarr_client::proto::{
    business_response, command_page, event_page, CommandBook, CommandPage, ContextualCommand,
    EventBook, EventPage,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{aggregate, full_type_url, CommandResult};

use prost::Message;
use prost_types::Any;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Ping {}
impl ::prost::Name for Ping {
    const NAME: &'static str = "Ping";
    const PACKAGE: &'static str = "test";
}

#[derive(Default)]
struct State;

// Handler A — emits one page tagged "A".
struct AggA;
#[aggregate(domain = "shared", state = State)]
impl AggA {
    #[handles(Ping)]
    #[allow(unused_variables, dead_code)]
    fn on_ping(&self, cmd: Ping, state: &State, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook {
            pages: vec![tagged_page("A")],
            ..Default::default()
        })
    }
}

// Handler B — emits one page tagged "B".
struct AggB;
#[aggregate(domain = "shared", state = State)]
impl AggB {
    #[handles(Ping)]
    #[allow(unused_variables, dead_code)]
    fn on_ping(&self, cmd: Ping, state: &State, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook {
            pages: vec![tagged_page("B")],
            ..Default::default()
        })
    }
}

fn tagged_page(tag: &str) -> EventPage {
    EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: tag.to_string(),
            value: vec![],
        })),
        ..Default::default()
    }
}

fn ping_command() -> ContextualCommand {
    let cmd = Any {
        type_url: full_type_url::<Ping>(),
        value: Ping {}.encode_to_vec(),
    };
    ContextualCommand {
        command: Some(CommandBook {
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(cmd)),
                ..Default::default()
            }],
            ..Default::default()
        }),
        events: Some(EventBook::default()),
    }
}

fn extract_pages_tags(response: angzarr_client::proto::BusinessResponse) -> Vec<String> {
    let Some(business_response::Result::Events(book)) = response.result else {
        panic!("expected Events");
    };
    book.pages
        .into_iter()
        .map(|p| match p.payload {
            Some(event_page::Payload::Event(any)) => any.type_url,
            _ => panic!("expected Event payload"),
        })
        .collect()
}

#[test]
fn multi_handler_concatenates_events_in_registration_order() {
    let built = Router::new("agg-shared")
        .with_handler(|| AggA)
        .with_handler(|| AggB)
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    let response = ch.dispatch(ping_command()).unwrap();
    let tags = extract_pages_tags(response);
    assert_eq!(tags, vec!["A".to_string(), "B".to_string()]);
}

#[test]
fn multi_handler_invokes_each_matched_factory_exactly_once() {
    let a_calls = Arc::new(AtomicU32::new(0));
    let b_calls = Arc::new(AtomicU32::new(0));

    let a_clone = Arc::clone(&a_calls);
    let b_clone = Arc::clone(&b_calls);

    let built = Router::new("agg-shared")
        .with_handler(move || {
            a_clone.fetch_add(1, Ordering::SeqCst);
            AggA
        })
        .with_handler(move || {
            b_clone.fetch_add(1, Ordering::SeqCst);
            AggB
        })
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };
    let _ = ch.dispatch(ping_command()).unwrap();

    assert_eq!(a_calls.load(Ordering::SeqCst), 1, "A factory call count");
    assert_eq!(b_calls.load(Ordering::SeqCst), 1, "B factory call count");
}
