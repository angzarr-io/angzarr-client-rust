//! Page extension traits for EventPage and CommandPage.
//!
//! Provides convenient accessors for sequence, type URL, and payload decoding.

use crate::proto::{command_page, CommandPage, EventPage};
use prost::Name;
use prost_types::Any;

/// The payload `any` decoded as `M` when its type URL names `M` (any
/// prefix).
fn decode_any<M: prost::Message + Default + Name>(any: &Any) -> Option<M> {
    if !crate::convert::type_url_is::<M>(&any.type_url) {
        return None;
    }
    M::decode(any.value.as_slice()).ok()
}

/// The payload of a command page's Command arm, when it carries one.
fn command_payload(page: &CommandPage) -> Option<&Any> {
    match &page.body {
        Some(command_page::Body::Command(command)) => command.payload.as_ref(),
        _ => None,
    }
}

/// Extension trait for EventPage proto type.
///
/// Provides convenient accessors for sequence, type URL, and payload decoding.
pub trait EventPageExt {
    /// Get the sequence number of this page.
    fn sequence_num(&self) -> u64;

    /// Get the type URL of the event, if present.
    fn type_url(&self) -> Option<&str>;

    /// Get the raw payload bytes, if present.
    fn payload(&self) -> Option<&[u8]>;

    /// Type-safe decode using prost::Name reflection.
    ///
    /// Returns None if the event is missing, type URL doesn't match exactly,
    /// or decoding fails. The expected type URL is derived from M::full_name().
    fn decode_typed<M: prost::Message + Default + Name>(&self) -> Option<M>;
}

impl EventPageExt for EventPage {
    fn sequence_num(&self) -> u64 {
        self.sequence
    }

    fn type_url(&self) -> Option<&str> {
        self.payload.as_ref().map(|e| e.type_url.as_str())
    }

    fn payload(&self) -> Option<&[u8]> {
        self.payload.as_ref().map(|e| e.value.as_slice())
    }

    fn decode_typed<M: prost::Message + Default + Name>(&self) -> Option<M> {
        self.payload.as_ref().and_then(decode_any)
    }
}

/// Extension trait for CommandPage proto type.
///
/// The accessors read the page's Command arm; a Fact page has none.
pub trait CommandPageExt {
    /// Get the type URL of the command, if present.
    fn type_url(&self) -> Option<&str>;

    /// Get the raw command payload bytes, if present.
    fn payload(&self) -> Option<&[u8]>;

    /// Type-safe decode using prost::Name reflection.
    fn decode_typed<M: prost::Message + Default + Name>(&self) -> Option<M>;
}

impl CommandPageExt for CommandPage {
    fn type_url(&self) -> Option<&str> {
        command_payload(self).map(|c| c.type_url.as_str())
    }

    fn payload(&self) -> Option<&[u8]> {
        command_payload(self).map(|c| c.value.as_slice())
    }

    fn decode_typed<M: prost::Message + Default + Name>(&self) -> Option<M> {
        command_payload(self).and_then(decode_any)
    }
}

#[cfg(test)]
#[path = "pages.test.rs"]
mod tests;
