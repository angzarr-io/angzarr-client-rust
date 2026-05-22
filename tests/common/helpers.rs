//! Proto-builder helpers for step defs.
//!
//! Port of `client-python/main/tests/client/steps/_helpers.py`. Each function
//! mirrors a single Python helper one-for-one. Generic over `prost::Message +
//! prost::Name` so type URLs derive via `angzarr_client::full_type_url`.

use std::collections::HashMap;

use angzarr_client::full_type_url;
use angzarr_client::proto::{
    command_page, event_page, page_header, CommandBook, CommandPage, ContextualCommand, Cover,
    EventBook, EventPage, Notification, PageHeader, ProcessManagerHandleRequest,
    RejectionNotification, SagaHandleRequest,
};
use prost::{Message, Name};
use prost_types::Any;

// ---------------------------------------------------------------------------
// Event / EventBook
// ---------------------------------------------------------------------------

pub fn pack_any<M: Message + Name>(msg: &M) -> Any {
    Any {
        type_url: full_type_url::<M>(),
        value: msg.encode_to_vec(),
    }
}

pub fn pack_event_page<M: Message + Name>(msg: &M, seq: u32) -> EventPage {
    EventPage {
        header: Some(PageHeader {
            sequence_type: Some(page_header::SequenceType::Sequence(seq)),
            ..Default::default()
        }),
        payload: Some(event_page::Payload::Event(pack_any(msg))),
        ..Default::default()
    }
}

pub fn event_book<M: Message + Name>(msgs: &[M], domain: &str) -> EventBook {
    let pages: Vec<EventPage> = msgs
        .iter()
        .enumerate()
        .map(|(i, m)| pack_event_page(m, i as u32))
        .collect();
    let next_sequence = pages.len() as u32;
    EventBook {
        cover: Some(Cover {
            domain: domain.to_string(),
            ..Default::default()
        }),
        pages,
        next_sequence,
        ..Default::default()
    }
}

/// EventBook from a heterogeneous list of pre-packed `Any` values. Mirrors
/// Python's loop-over-msgs but for callers that already hold `Any`s (e.g.
/// when mixing types in one book).
pub fn event_book_from_any(anys: Vec<Any>, domain: &str) -> EventBook {
    let pages: Vec<EventPage> = anys
        .into_iter()
        .enumerate()
        .map(|(i, any)| EventPage {
            header: Some(PageHeader {
                sequence_type: Some(page_header::SequenceType::Sequence(i as u32)),
                ..Default::default()
            }),
            payload: Some(event_page::Payload::Event(any)),
            ..Default::default()
        })
        .collect();
    let next_sequence = pages.len() as u32;
    EventBook {
        cover: Some(Cover {
            domain: domain.to_string(),
            ..Default::default()
        }),
        pages,
        next_sequence,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Command / CommandBook / ContextualCommand
// ---------------------------------------------------------------------------

pub fn command_book<C: Message + Name>(cmd: &C, target_domain: &str, seq: u32) -> CommandBook {
    CommandBook {
        cover: Some(Cover {
            domain: target_domain.to_string(),
            ..Default::default()
        }),
        pages: vec![CommandPage {
            header: Some(PageHeader {
                sequence_type: Some(page_header::SequenceType::Sequence(seq)),
                ..Default::default()
            }),
            payload: Some(command_page::Payload::Command(pack_any(cmd))),
            ..Default::default()
        }],
    }
}

pub fn contextual_command<C: Message + Name>(
    cmd: &C,
    domain: &str,
    prior: Option<EventBook>,
) -> ContextualCommand {
    ContextualCommand {
        command: Some(command_book(cmd, domain, 0)),
        events: prior,
    }
}

// ---------------------------------------------------------------------------
// Saga / PM requests
// ---------------------------------------------------------------------------

pub fn saga_request<E: Message + Name>(
    events: &[E],
    source_domain: &str,
    dest_seqs: Option<HashMap<String, u32>>,
) -> SagaHandleRequest {
    SagaHandleRequest {
        source: Some(event_book(events, source_domain)),
        destination_sequences: dest_seqs.unwrap_or_default(),
        ..Default::default()
    }
}

/// Generic two-type-parameter PM request: triggers of type `T`, process_state
/// of type `S`. For callers that don't pass any process_state events, use
/// [`pm_request_no_state`] to dodge the `S` type-inference problem.
pub fn pm_request<T: Message + Name, S: Message + Name>(
    triggers: &[T],
    source_domain: &str,
    process_state: &[S],
    pm_domain: &str,
    dest_seqs: Option<HashMap<String, u32>>,
) -> ProcessManagerHandleRequest {
    ProcessManagerHandleRequest {
        trigger: Some(event_book(triggers, source_domain)),
        process_state: Some(event_book(process_state, pm_domain)),
        destination_sequences: dest_seqs.unwrap_or_default(),
        ..Default::default()
    }
}

/// PM request with empty `process_state`. Mirrors the common Python call site
/// `pm_request(triggers, source_domain="...", process_state_msgs=None, ...)`.
pub fn pm_request_no_state<T: Message + Name>(
    triggers: &[T],
    source_domain: &str,
    pm_domain: &str,
    dest_seqs: Option<HashMap<String, u32>>,
) -> ProcessManagerHandleRequest {
    ProcessManagerHandleRequest {
        trigger: Some(event_book(triggers, source_domain)),
        process_state: Some(EventBook {
            cover: Some(Cover {
                domain: pm_domain.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        destination_sequences: dest_seqs.unwrap_or_default(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// Build a [`Notification`] whose `RejectionNotification` targets `cmd`. The
/// outer `Notification` carries no `cover`; callers wrap it in a
/// [`ContextualCommand`] via [`contextual_notification`] for delivery.
pub fn notification_for<C: Message + Name>(cmd: &C, target_domain: &str) -> Notification {
    let rejection = RejectionNotification {
        rejected_command: Some(command_book(cmd, target_domain, 0)),
        rejection_reason: "test rejection".to_string(),
    };
    Notification {
        payload: Some(pack_any(&rejection)),
        ..Default::default()
    }
}

/// Wrap a [`Notification`] in a [`ContextualCommand`] addressed to
/// `aggregate_domain`. Mirrors Python's `contextual_notification`.
pub fn contextual_notification(
    notification: Notification,
    aggregate_domain: &str,
) -> ContextualCommand {
    let n_any = pack_any(&notification);
    ContextualCommand {
        command: Some(CommandBook {
            cover: Some(Cover {
                domain: aggregate_domain.to_string(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                header: Some(PageHeader {
                    sequence_type: Some(page_header::SequenceType::Sequence(0)),
                    ..Default::default()
                }),
                payload: Some(command_page::Payload::Command(n_any)),
                ..Default::default()
            }],
        }),
        events: None,
    }
}
