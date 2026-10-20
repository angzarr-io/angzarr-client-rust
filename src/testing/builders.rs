//! Proto message builders for testing.
//!
//! Simplified constructors for `EventBook`, `CommandBook`, `Cover`, and
//! related proto types. Mirrors Python's `angzarr_client.testing.builders`.

use prost::{Message, Name};
use prost_types::{Any, Timestamp};

use crate::proto::{
    command_page, Command, CommandBook, CommandPage, Cover, EventBook, EventMeta, EventPage,
    Handling, Uuid as ProtoUuid,
};

/// Create a timestamp for now. Alias for `crate::now()`.
///
/// **Non-deterministic** — wraps `SystemTime::now()`. For tests that
/// compare serialized bytes across runs (or across language siblings),
/// use [`make_event_page_at`] with an explicit timestamp instead.
#[must_use]
pub fn make_timestamp() -> Timestamp {
    crate::now()
}

/// Pack a protobuf message into an `Any` with the canonical type URL.
///
/// The type URL is [`crate::TYPE_URL_PREFIX`] (`/`) + `M::full_name()`.
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
pub fn make_cover(domain: impl Into<String>, root: [u8; 16]) -> Cover {
    Cover {
        domain: domain.into(),
        root: Some(ProtoUuid::from(root)),
        ..Default::default()
    }
}

/// Build an `EventPage` with `sequence` and payload, stamping
/// `meta.recorded_at` from the wall clock.
///
/// For deterministic byte-equal tests across runs / language siblings,
/// use [`make_event_page_at`] with an explicit timestamp.
#[must_use]
pub fn make_event_page(sequence: u64, event: Any) -> EventPage {
    make_event_page_at(sequence, event, make_timestamp())
}

/// Like [`make_event_page`] but takes an explicit `recorded_at` so the
/// caller controls determinism. Use a fixed timestamp (e.g.
/// `Timestamp { seconds: 0, nanos: 0 }`) when comparing serialized
/// bytes across cross-language parity tests.
#[must_use]
pub fn make_event_page_at(sequence: u64, event: Any, recorded_at: Timestamp) -> EventPage {
    EventPage {
        payload: Some(event),
        meta: Some(EventMeta {
            recorded_at: Some(recorded_at),
            ..Default::default()
        }),
        sequence,
    }
}

/// Build an `EventBook` from a cover, optional page list, and optional
/// `next_sequence` (defaults to `pages.len()`).
#[must_use]
pub fn make_event_book(
    cover: Cover,
    pages: Vec<EventPage>,
    next_sequence: Option<u64>,
) -> EventBook {
    let next = next_sequence.unwrap_or(pages.len() as u64);
    EventBook {
        cover: Some(cover),
        pages,
        snapshot: None,
        next_sequence: next,
    }
}

/// Build a `CommandPage` carrying `command` in its Command arm.
#[must_use]
pub fn make_command_page(command: Any) -> CommandPage {
    CommandPage {
        body: Some(command_page::Body::Command(Command {
            payload: Some(command),
        })),
        ..Default::default()
    }
}

/// Build a single-command `CommandBook`.
///
/// `sequence` (`Handling.expected_sequence`) defaults to `0` when `None` —
/// mirrors Python's `make_command_book(cover, command, sequence=0)`. Pass
/// `Some(n)` to override.
#[must_use]
pub fn make_command_book(cover: Cover, command: Any, sequence: Option<u64>) -> CommandBook {
    CommandBook {
        cover: Some(cover),
        page: Some(make_command_page(command)),
        handling: Some(Handling {
            expected_sequence: Some(sequence.unwrap_or(0)),
            ..Default::default()
        }),
    }
}

#[cfg(test)]
#[path = "builders.test.rs"]
mod tests;
