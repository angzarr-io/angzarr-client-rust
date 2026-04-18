//! R9 — sequence increments across the merged stream.
//!
//! When multiple aggregate handlers match the same command, the framework
//! threads `seq` through them: handler A called with `seq=N` emits `k`
//! events, handler B called with `seq=N+k`. The final merged event stream
//! has monotonically increasing sequence numbers.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use angzarr_client::proto::{
    command_page, CommandBook, CommandPage, ContextualCommand, EventBook, EventPage,
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

/// Handler that always emits two empty event pages.
struct EmitsTwo;

#[aggregate(domain = "shared", state = State)]
impl EmitsTwo {
    #[handles(Ping)]
    #[allow(unused_variables, dead_code)]
    fn on_ping(&self, cmd: Ping, state: &State, seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook {
            pages: vec![EventPage::default(); 2],
            ..Default::default()
        })
    }
}

/// Handler that records the `seq` it was called with and emits nothing.
struct RecordsSeq {
    seen: Arc<AtomicU32>,
}

#[aggregate(domain = "shared", state = State)]
impl RecordsSeq {
    #[handles(Ping)]
    #[allow(unused_variables, dead_code)]
    fn on_ping(&self, cmd: Ping, state: &State, seq: u32) -> CommandResult<EventBook> {
        self.seen.store(seq, Ordering::SeqCst);
        Ok(EventBook::default())
    }
}

fn ping_command_with_prior_next_seq(next_seq: u32) -> ContextualCommand {
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
        events: Some(EventBook {
            next_sequence: next_seq,
            ..Default::default()
        }),
    }
}

#[test]
fn second_handler_receives_seq_offset_by_first_handler_emissions() {
    let seen = Arc::new(AtomicU32::new(0));
    let seen_clone = Arc::clone(&seen);

    let built = Router::new("agg-shared")
        .with_handler(|| EmitsTwo)
        .with_handler(move || RecordsSeq {
            seen: Arc::clone(&seen_clone),
        })
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };

    // Prior next_sequence = 5. EmitsTwo emits 2. RecordsSeq must see seq = 7.
    let _ = ch.dispatch(ping_command_with_prior_next_seq(5)).unwrap();
    assert_eq!(
        seen.load(Ordering::SeqCst),
        7,
        "RecordsSeq should see seq = prior(5) + EmitsTwo's 2 pages = 7"
    );
}

#[test]
fn single_handler_receives_initial_next_sequence() {
    // Sanity: with one handler, seq received equals prior next_sequence.
    let seen = Arc::new(AtomicU32::new(0));
    let seen_clone = Arc::clone(&seen);

    let built = Router::new("agg-shared")
        .with_handler(move || RecordsSeq {
            seen: Arc::clone(&seen_clone),
        })
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };

    let _ = ch.dispatch(ping_command_with_prior_next_seq(42)).unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 42);
}

#[test]
fn three_handlers_thread_seq_monotonically() {
    let seen = Arc::new(AtomicU32::new(0));
    let seen_clone = Arc::clone(&seen);

    let built = Router::new("agg-shared")
        .with_handler(|| EmitsTwo) // 10 -> emits 2, next seq = 12
        .with_handler(|| EmitsTwo) // 12 -> emits 2, next seq = 14
        .with_handler(move || RecordsSeq {
            seen: Arc::clone(&seen_clone),
        })
        .build()
        .unwrap();
    let Built::CommandHandler(ch) = built else {
        panic!("expected CommandHandler");
    };

    let _ = ch.dispatch(ping_command_with_prior_next_seq(10)).unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 14);
}
