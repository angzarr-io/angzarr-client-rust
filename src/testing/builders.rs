//! Proto message builders for testing.
//!
//! Simplified constructors for `EventBook`, `CommandBook`, `Cover`, and
//! related proto types. Mirrors Python's `angzarr_client.testing.builders`.

use prost::{Message, Name};
use prost_types::{Any, Timestamp};

use crate::proto::{
    command_page, event_page, page_header::SequenceType, CommandBook, CommandPage, Cover,
    EventBook, EventPage, MergeStrategy, PageHeader, Uuid as ProtoUuid,
};

/// Create a timestamp for now. Alias for `crate::now()`.
///
/// **Non-deterministic** — wraps `SystemTime::now()`. For tests that
/// compare serialized bytes across runs (or across language siblings),
/// use [`make_event_page_at`] / [`make_event_book_at`] etc. with an
/// explicit timestamp instead.
#[must_use]
pub fn make_timestamp() -> Timestamp {
    crate::now()
}

/// Pack a protobuf message into an `Any` with the canonical type URL.
///
/// The type URL is derived from `M::full_name()` (the proto descriptor),
/// prefixed with the standard `type.googleapis.com/` per the
/// `google.protobuf.Any` spec.
///
/// Audit finding #47 (Option C — drop the second arg, derive name from
/// the message): mirrors Python's `testing.builders.pack_event(msg)`.
/// Removes the previous `type_name` string parameter (which was both a
/// typo-prone footgun and diverged in meaning from Python's 2nd-arg
/// convention).
#[must_use]
pub fn pack_event<M: Message + Name>(msg: &M) -> Any {
    Any {
        type_url: crate::type_url(&M::full_name()),
        value: msg.encode_to_vec(),
    }
}

/// Build a `Cover` from domain + 16-byte root.
#[must_use]
pub fn make_cover(
    domain: impl Into<String>,
    root: [u8; 16],
    correlation_id: impl Into<String>,
) -> Cover {
    Cover {
        domain: domain.into(),
        root: Some(ProtoUuid {
            value: root.to_vec(),
        }),
        correlation_id: correlation_id.into(),
        edition: None,
    }
}

/// Build an `EventPage` with `sequence` and payload, stamping
/// `created_at` from the wall clock.
///
/// For deterministic byte-equal tests across runs / language siblings,
/// use [`make_event_page_at`] with an explicit timestamp.
#[must_use]
pub fn make_event_page(sequence: u32, event: Any) -> EventPage {
    make_event_page_at(sequence, event, make_timestamp())
}

/// Like [`make_event_page`] but takes an explicit `created_at` so the
/// caller controls determinism. Use a fixed timestamp (e.g.
/// `Timestamp { seconds: 0, nanos: 0 }`) when comparing serialized
/// bytes across cross-language parity tests.
#[must_use]
pub fn make_event_page_at(sequence: u32, event: Any, created_at: Timestamp) -> EventPage {
    EventPage {
        header: Some(PageHeader {
            sequence_type: Some(SequenceType::Sequence(sequence)),
            sync_mode: None,
        }),
        created_at: Some(created_at),
        payload: Some(event_page::Payload::Event(event)),
        cascade_id: None,
        no_commit: false,
    }
}

/// Build an `EventBook` from a cover, optional page list, and optional
/// `next_sequence` (defaults to `pages.len()`).
#[must_use]
pub fn make_event_book(
    cover: Cover,
    pages: Vec<EventPage>,
    next_sequence: Option<u32>,
) -> EventBook {
    let next = next_sequence.unwrap_or(pages.len() as u32);
    EventBook {
        cover: Some(cover),
        pages,
        snapshot: None,
        next_sequence: next,
    }
}

/// Build a `CommandPage` with `sequence` and payload.
#[must_use]
pub fn make_command_page(sequence: u32, command: Any) -> CommandPage {
    CommandPage {
        header: Some(PageHeader {
            sequence_type: Some(SequenceType::Sequence(sequence)),
            sync_mode: None,
        }),
        payload: Some(command_page::Payload::Command(command)),
        merge_strategy: MergeStrategy::MergeCommutative as i32,
    }
}

/// Build a single-command `CommandBook`.
///
/// `sequence` defaults to `0` when `None` — mirrors Python's
/// `make_command_book(cover, command, sequence=0)`. Pass `Some(n)` to
/// override.
#[must_use]
pub fn make_command_book(cover: Cover, command: Any, sequence: Option<u32>) -> CommandBook {
    CommandBook {
        cover: Some(cover),
        pages: vec![make_command_page(sequence.unwrap_or(0), command)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto_ext::EventPageExt;

    #[test]
    fn make_cover_sets_root() {
        let root = [1u8; 16];
        let cover = make_cover("player", root, "corr-1");
        assert_eq!(cover.domain, "player");
        assert_eq!(cover.correlation_id, "corr-1");
        assert_eq!(cover.root.unwrap().value, root.to_vec());
    }

    #[test]
    fn make_event_book_defaults_next_sequence() {
        let cover = make_cover("x", [0u8; 16], "");
        let pages = vec![
            make_event_page(
                0,
                Any {
                    type_url: "t".into(),
                    value: vec![],
                },
            ),
            make_event_page(
                1,
                Any {
                    type_url: "t".into(),
                    value: vec![],
                },
            ),
        ];
        let book = make_event_book(cover, pages, None);
        assert_eq!(book.pages.len(), 2);
        assert_eq!(book.next_sequence, 2);
    }

    #[test]
    fn make_event_page_sets_sequence() {
        let page = make_event_page(
            7,
            Any {
                type_url: "t".into(),
                value: vec![],
            },
        );
        assert_eq!(page.sequence_num(), 7);
        assert!(page.created_at.is_some());
    }

    #[test]
    fn make_command_book_single_page() {
        let cover = make_cover("x", [0u8; 16], "");
        let book = make_command_book(
            cover,
            Any {
                type_url: "t".into(),
                value: vec![],
            },
            None,
        );
        assert_eq!(book.pages.len(), 1);
    }

    #[test]
    fn make_command_book_default_sequence_is_zero() {
        let cover = make_cover("x", [0u8; 16], "");
        let book = make_command_book(
            cover,
            Any {
                type_url: "t".into(),
                value: vec![],
            },
            None,
        );
        let header = book.pages[0].header.as_ref().expect("header");
        match header.sequence_type.as_ref().expect("sequence_type") {
            SequenceType::Sequence(s) => assert_eq!(*s, 0),
            _ => panic!("expected explicit sequence"),
        }
    }
}
