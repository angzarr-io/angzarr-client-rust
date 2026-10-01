//! Shared helpers for saga / process-manager scenarios that assert the
//! `angzarr_deferred` provenance on emitted commands.

use angzarr_client::proto::{
    command_page, event_page, page_header::SequenceType, AngzarrDeferredSequence, CommandBook,
    CommandPage, Cover, EventBook, EventPage, PageHeader, Uuid as ProtoUuid,
};
use prost::{Message, Name};
use prost_types::Any;

/// Entity root bytes for a scenario label: uuid5(NAMESPACE_OID, label), the
/// cross-language convention for test roots.
pub fn root_for(label: &str) -> Vec<u8> {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, label.as_bytes())
        .as_bytes()
        .to_vec()
}

pub fn pack<M: Message + Name>(msg: &M) -> Any {
    Any {
        type_url: angzarr_client::full_type_url::<M>(),
        value: msg.encode_to_vec(),
    }
}

/// A one-event EventBook in `domain`/`root_label` whose event sits at `seq`.
pub fn trigger_book<M: Message + Name>(
    event: &M,
    domain: &str,
    root_label: &str,
    seq: u32,
) -> EventBook {
    EventBook {
        cover: Some(Cover {
            domain: domain.into(),
            root: Some(ProtoUuid {
                value: root_for(root_label),
            }),
            correlation_id: "corr-1".into(),
            ..Default::default()
        }),
        pages: vec![EventPage {
            header: Some(PageHeader {
                sequence_type: Some(SequenceType::Sequence(seq)),
                sync_mode: None,
            }),
            payload: Some(event_page::Payload::Event(pack(event))),
            ..Default::default()
        }],
        next_sequence: seq + 1,
        ..Default::default()
    }
}

/// A command for `domain` with one page carrying `cmd` and no header — the
/// shape a saga / PM handler emits before the router stamps provenance.
pub fn unsequenced_command<M: Message + Name>(cmd: &M, domain: &str) -> CommandBook {
    CommandBook {
        cover: Some(Cover {
            domain: domain.into(),
            ..Default::default()
        }),
        pages: vec![CommandPage {
            payload: Some(command_page::Payload::Command(pack(cmd))),
            ..Default::default()
        }],
    }
}

/// Type URL of the first page's command payload.
pub fn command_type_url(cmd: &CommandBook) -> String {
    match cmd.pages.first().and_then(|p| p.payload.as_ref()) {
        Some(command_page::Payload::Command(any)) => any.type_url.clone(),
        other => panic!("command has no command payload: {other:?}"),
    }
}

/// The emitted command whose payload is of type `M`.
pub fn command_of<M: Name>(commands: &[CommandBook]) -> &CommandBook {
    let url = angzarr_client::full_type_url::<M>();
    commands
        .iter()
        .find(|c| command_type_url(c) == url)
        .unwrap_or_else(|| panic!("no {url} command among {commands:?}"))
}

/// The `angzarr_deferred` header of every page of `cmd`; panics when a page
/// carries anything else.
pub fn deferred_headers(cmd: &CommandBook) -> Vec<&AngzarrDeferredSequence> {
    assert!(!cmd.pages.is_empty(), "command has no pages");
    cmd.pages
        .iter()
        .map(
            |p| match p.header.as_ref().and_then(|h| h.sequence_type.as_ref()) {
                Some(SequenceType::AngzarrDeferred(d)) => d,
                other => panic!("page header is not angzarr_deferred: {other:?}"),
            },
        )
        .collect()
}

/// The single deferred header shared by every page of `cmd`.
pub fn deferred_header(cmd: &CommandBook) -> &AngzarrDeferredSequence {
    let headers = deferred_headers(cmd);
    let first = headers[0];
    for h in &headers[1..] {
        assert_eq!(*h, first, "pages carry differing deferred headers");
    }
    first
}

/// True when any page of `cmd` carries an explicit `sequence`.
pub fn has_explicit_sequence(cmd: &CommandBook) -> bool {
    cmd.pages.iter().any(|p| {
        matches!(
            p.header.as_ref().and_then(|h| h.sequence_type.as_ref()),
            Some(SequenceType::Sequence(_))
        )
    })
}

/// A `ContextualCommand` delivering the rejection of `rejected` (addressed to
/// `rejected_domain`) to the compensating aggregate in `aggregate_domain`.
pub fn rejection_delivery<M: Message + Name>(
    rejected: &M,
    rejected_domain: &str,
    aggregate_domain: &str,
    prior: Option<EventBook>,
) -> angzarr_client::proto::ContextualCommand {
    use angzarr_client::proto::{Notification, RejectionNotification};
    let rejection = RejectionNotification {
        rejected_command: Some(unsequenced_command(rejected, rejected_domain)),
        rejection_reason: "rejected for test".into(),
    };
    let notification = Notification {
        payload: Some(pack(&rejection)),
        ..Default::default()
    };
    angzarr_client::proto::ContextualCommand {
        command: Some(CommandBook {
            cover: Some(Cover {
                domain: aggregate_domain.into(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                payload: Some(command_page::Payload::Command(pack(&notification))),
                ..Default::default()
            }],
        }),
        events: prior,
    }
}

/// Decode every event page of `book` whose payload is an `M`.
pub fn events_of<M: Message + Name + Default>(book: &EventBook) -> Vec<M> {
    let url = angzarr_client::full_type_url::<M>();
    book.pages
        .iter()
        .filter_map(|p| match &p.payload {
            Some(event_page::Payload::Event(any)) if any.type_url == url => {
                Some(M::decode(any.value.as_slice()).expect("decode event"))
            }
            _ => None,
        })
        .collect()
}

/// An unsequenced event page carrying `event`.
pub fn event_page_of<M: Message + Name>(event: &M) -> EventPage {
    EventPage {
        payload: Some(event_page::Payload::Event(pack(event))),
        ..Default::default()
    }
}

/// A `ContextualCommand` delivering `cmd` to an empty aggregate in `domain`.
pub fn command_delivery<M: Message + Name>(
    cmd: &M,
    domain: &str,
) -> angzarr_client::proto::ContextualCommand {
    angzarr_client::proto::ContextualCommand {
        command: Some(unsequenced_command(cmd, domain)),
        events: None,
    }
}
