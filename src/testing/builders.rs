//! Proto message builders for testing.
//!
//! Simplified constructors for `EventBook`, `CommandBook`, `Cover`, and
//! related proto types. Mirrors Python's `angzarr_client.testing.builders`.

use prost::Message;
use prost_types::{Any, Timestamp};

use crate::proto::{
    command_page, event_page, page_header::SequenceType, CommandBook, CommandPage, Cover,
    EventBook, EventPage, PageHeader, Uuid as ProtoUuid,
};

/// Create a timestamp for now. Alias for `crate::now()`.
pub fn make_timestamp() -> Timestamp {
    crate::now()
}

/// Pack a protobuf message into an `Any` using the given type-URL prefix.
/// Mirrors Python's `pack_event(msg, type_url_prefix)`. `type_name` should be
/// the message's fully-qualified proto type name (e.g. `"orders.OrderCreated"`).
pub fn pack_event<M: Message>(msg: &M, type_name: &str) -> Any {
    Any {
        type_url: crate::type_url(type_name),
        value: msg.encode_to_vec(),
    }
}

/// Build a `Cover` from domain + 16-byte root.
pub fn make_cover(domain: impl Into<String>, root: [u8; 16], correlation_id: impl Into<String>) -> Cover {
    Cover {
        domain: domain.into(),
        root: Some(ProtoUuid { value: root.to_vec() }),
        correlation_id: correlation_id.into(),
        edition: None,
    }
}

/// Build an `EventPage` with `sequence` and payload.
pub fn make_event_page(sequence: u32, event: Any) -> EventPage {
    EventPage {
        header: Some(PageHeader {
            sequence_type: Some(SequenceType::Sequence(sequence)),
        }),
        created_at: Some(make_timestamp()),
        payload: Some(event_page::Payload::Event(event)),
        cascade_id: None,
        no_commit: false,
    }
}

/// Build an `EventBook` from a cover, optional page list, and optional
/// `next_sequence` (defaults to `pages.len()`).
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
pub fn make_command_page(sequence: u32, command: Any) -> CommandPage {
    CommandPage {
        header: Some(PageHeader {
            sequence_type: Some(SequenceType::Sequence(sequence)),
        }),
        payload: Some(command_page::Payload::Command(command)),
        merge_strategy: 0, // MERGE_COMMUTATIVE default
    }
}

/// Build a single-command `CommandBook`.
pub fn make_command_book(cover: Cover, command: Any, sequence: u32) -> CommandBook {
    CommandBook {
        cover: Some(cover),
        pages: vec![make_command_page(sequence, command)],
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
            make_event_page(0, Any { type_url: "t".into(), value: vec![] }),
            make_event_page(1, Any { type_url: "t".into(), value: vec![] }),
        ];
        let book = make_event_book(cover, pages, None);
        assert_eq!(book.pages.len(), 2);
        assert_eq!(book.next_sequence, 2);
    }

    #[test]
    fn make_event_page_sets_sequence() {
        let page = make_event_page(7, Any { type_url: "t".into(), value: vec![] });
        assert_eq!(page.sequence_num(), 7);
        assert!(page.created_at.is_some());
    }

    #[test]
    fn make_command_book_single_page() {
        let cover = make_cover("x", [0u8; 16], "");
        let book = make_command_book(cover, Any { type_url: "t".into(), value: vec![] }, 0);
        assert_eq!(book.pages.len(), 1);
    }
}
