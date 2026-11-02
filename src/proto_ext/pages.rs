//! Page extension traits for EventPage and CommandPage.
//!
//! Provides convenient accessors for sequence, type URL, and payload decoding.

use super::constants::TYPE_URL_PREFIX;
use crate::proto::page_header::SequenceType;
use crate::proto::{
    AngzarrDeferredSequence, CommandPage, EventPage, ExternalDeferredSequence, MergeStrategy,
    PageHeader,
};
use prost::Name;

/// Extension trait for PageHeader.
pub trait PageHeaderExt {
    /// Get the explicit sequence number, if set.
    /// Returns None for deferred sequences (external or angzarr).
    fn explicit_sequence(&self) -> Option<u32>;

    /// Check if this is a deferred sequence (not yet stamped).
    fn is_deferred(&self) -> bool;

    /// Get external deferred info, if present.
    fn external_deferred(&self) -> Option<&ExternalDeferredSequence>;

    /// Get angzarr deferred info (saga-produced), if present.
    fn angzarr_deferred(&self) -> Option<&AngzarrDeferredSequence>;
}

impl PageHeaderExt for PageHeader {
    fn explicit_sequence(&self) -> Option<u32> {
        match &self.sequence_type {
            Some(SequenceType::Sequence(seq)) => Some(*seq),
            _ => None,
        }
    }

    fn is_deferred(&self) -> bool {
        matches!(
            &self.sequence_type,
            Some(SequenceType::ExternalDeferred(_)) | Some(SequenceType::AngzarrDeferred(_))
        )
    }

    fn external_deferred(&self) -> Option<&ExternalDeferredSequence> {
        match &self.sequence_type {
            Some(SequenceType::ExternalDeferred(ext)) => Some(ext),
            _ => None,
        }
    }

    fn angzarr_deferred(&self) -> Option<&AngzarrDeferredSequence> {
        match &self.sequence_type {
            Some(SequenceType::AngzarrDeferred(ang)) => Some(ang),
            _ => None,
        }
    }
}

/// Extension trait for EventPage proto type.
///
/// Provides convenient accessors for sequence, type URL, and payload decoding.
pub trait EventPageExt {
    /// Get the sequence number from this page.
    /// Returns 0 for deferred sequences (not yet stamped).
    fn sequence_num(&self) -> u32;

    /// Get the page header, if present.
    fn header(&self) -> Option<&PageHeader>;

    /// Check if this page has a deferred sequence.
    fn is_deferred(&self) -> bool;

    /// Get the type URL of the event, if present.
    fn type_url(&self) -> Option<&str>;

    /// Get the raw payload bytes, if present.
    fn payload(&self) -> Option<&[u8]>;

    /// Type-safe decode using prost::Name reflection.
    ///
    /// `Ok(None)` when there is no event payload or its type URL is not
    /// exactly the one derived from `M::full_name()`: a mismatch. A payload
    /// whose type matches but whose bytes do not decode is an error, not a
    /// mismatch: `Err` with code `ANY_DECODE_FAILED`, as
    /// [`crate::convert::unpack`], [`crate::convert::try_unpack`] and
    /// [`crate::decode_event`] report it.
    fn decode_typed<M: prost::Message + Default + Name>(&self) -> crate::error::Result<Option<M>>;
}

impl EventPageExt for EventPage {
    fn sequence_num(&self) -> u32 {
        self.header
            .as_ref()
            .and_then(|h| h.explicit_sequence())
            .unwrap_or(0)
    }

    fn header(&self) -> Option<&PageHeader> {
        self.header.as_ref()
    }

    fn is_deferred(&self) -> bool {
        self.header
            .as_ref()
            .map(|h| h.is_deferred())
            .unwrap_or(false)
    }

    fn type_url(&self) -> Option<&str> {
        match &self.payload {
            Some(crate::proto::event_page::Payload::Event(e)) => Some(e.type_url.as_str()),
            _ => None,
        }
    }

    fn payload(&self) -> Option<&[u8]> {
        match &self.payload {
            Some(crate::proto::event_page::Payload::Event(e)) => Some(e.value.as_slice()),
            _ => None,
        }
    }

    fn decode_typed<M: prost::Message + Default + Name>(&self) -> crate::error::Result<Option<M>> {
        let event = match &self.payload {
            Some(crate::proto::event_page::Payload::Event(e)) => e,
            _ => return Ok(None),
        };
        decode_matching(event)
    }
}

/// Decode a page's payload as `M` when its type URL is exactly `M`'s:
/// `Ok(None)` on a mismatch, `ANY_DECODE_FAILED` (via the shared
/// [`crate::convert::decode_payload`]) when matching bytes do not decode.
fn decode_matching<M: prost::Message + Default + Name>(
    any: &prost_types::Any,
) -> crate::error::Result<Option<M>> {
    let expected = format!("{}{}", TYPE_URL_PREFIX, M::full_name());
    if any.type_url != expected {
        return Ok(None);
    }
    crate::convert::decode_payload(&any.value, &expected).map(Some)
}

/// Extension trait for CommandPage proto type.
///
/// Provides convenient accessors for sequence, type URL, and payload decoding.
pub trait CommandPageExt {
    /// Get the sequence number from this page.
    /// Returns 0 for deferred sequences (not yet stamped).
    fn sequence_num(&self) -> u32;

    /// Get the page header, if present.
    fn header(&self) -> Option<&PageHeader>;

    /// Check if this page has a deferred sequence.
    fn is_deferred(&self) -> bool;

    /// Get the type URL of the command, if present.
    fn type_url(&self) -> Option<&str>;

    /// Get the raw payload bytes, if present.
    fn payload(&self) -> Option<&[u8]>;

    /// Type-safe decode using prost::Name reflection.
    ///
    /// `Ok(None)` when there is no command payload or its type URL is not
    /// exactly the one derived from `M::full_name()`: a mismatch. A payload
    /// whose type matches but whose bytes do not decode is an error, not a
    /// mismatch: `Err` with code `ANY_DECODE_FAILED`, as
    /// [`crate::convert::unpack`], [`crate::convert::try_unpack`] and
    /// [`crate::decode_event`] report it.
    fn decode_typed<M: prost::Message + Default + Name>(&self) -> crate::error::Result<Option<M>>;

    /// Get the merge strategy for this command.
    ///
    /// Returns the MergeStrategy enum value. Defaults to Commutative (0) if unset.
    fn merge_strategy(&self) -> MergeStrategy;
}

impl CommandPageExt for CommandPage {
    fn sequence_num(&self) -> u32 {
        self.header
            .as_ref()
            .and_then(|h| h.explicit_sequence())
            .unwrap_or(0)
    }

    fn header(&self) -> Option<&PageHeader> {
        self.header.as_ref()
    }

    fn is_deferred(&self) -> bool {
        self.header
            .as_ref()
            .map(|h| h.is_deferred())
            .unwrap_or(false)
    }

    fn type_url(&self) -> Option<&str> {
        match &self.payload {
            Some(crate::proto::command_page::Payload::Command(c)) => Some(c.type_url.as_str()),
            _ => None,
        }
    }

    fn payload(&self) -> Option<&[u8]> {
        match &self.payload {
            Some(crate::proto::command_page::Payload::Command(c)) => Some(c.value.as_slice()),
            _ => None,
        }
    }

    fn decode_typed<M: prost::Message + Default + Name>(&self) -> crate::error::Result<Option<M>> {
        let command = match &self.payload {
            Some(crate::proto::command_page::Payload::Command(c)) => c,
            _ => return Ok(None),
        };
        decode_matching(command)
    }

    fn merge_strategy(&self) -> MergeStrategy {
        MergeStrategy::try_from(self.merge_strategy).unwrap_or(MergeStrategy::MergeCommutative)
    }
}

/// Extension trait for AngzarrDeferredSequence.
///
/// Provides idempotency key generation for saga-produced commands/facts.
pub trait AngzarrDeferredSequenceExt {
    /// Generate the composite idempotency key for logging and display.
    ///
    /// Format: `{source.edition}:{source.domain}:{source.root_hex}:{source_seq}`
    ///
    /// Example: `angzarr:order:550e8400e29b41d4a716446655440000:7`
    ///
    /// Returns `Err("source required")` when the deferred sequence has no
    /// source cover — a malformed wire input is a recoverable error, not a
    /// process abort. Audit finding #55.
    fn idempotency_key(&self) -> Result<String, &'static str>;
}

impl AngzarrDeferredSequenceExt for AngzarrDeferredSequence {
    fn idempotency_key(&self) -> Result<String, &'static str> {
        use super::cover::CoverExt;
        let source = self.source.as_ref().ok_or("source required")?;
        Ok(format!(
            "{}:{}:{}:{}",
            source.edition(),
            source.domain,
            source.root_id_hex().unwrap_or_default(),
            self.source_seq
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{full_type_url, type_url};
    use crate::error::ClientError;
    use crate::error_codes::{codes, keys};
    use crate::proto::{command_page, event_page, Cover};
    use prost::Message;
    use prost_types::Any;

    fn cover() -> Cover {
        Cover {
            domain: "orders".to_string(),
            ..Default::default()
        }
    }

    /// Bytes that are no protobuf encoding of anything: a field key whose
    /// varint never ends.
    fn undecodable() -> Vec<u8> {
        vec![0xFF, 0xFF, 0xFF, 0xFF]
    }

    fn any(type_url: String, value: Vec<u8>) -> Any {
        Any { type_url, value }
    }

    fn event_page(any: Any) -> EventPage {
        EventPage {
            payload: Some(event_page::Payload::Event(any)),
            ..Default::default()
        }
    }

    fn command_page(any: Any) -> CommandPage {
        CommandPage {
            payload: Some(command_page::Payload::Command(any)),
            ..Default::default()
        }
    }

    /// An undecodable payload of the matching type is `ANY_DECODE_FAILED`,
    /// naming the type URL expected and the decoder's cause.
    fn assert_decode_failed(err: ClientError) {
        assert_eq!(err.code(), codes::ANY_DECODE_FAILED);
        let ClientError::InvalidArgument(detail) = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert_eq!(detail.details[keys::EXPECTED], full_type_url::<Cover>());
        assert!(!detail.details[keys::CAUSE].is_empty());
    }

    #[test]
    fn event_decode_typed_decodes_a_payload_of_its_type() {
        let page = event_page(any(full_type_url::<Cover>(), cover().encode_to_vec()));
        assert_eq!(page.decode_typed::<Cover>().unwrap(), Some(cover()));
    }

    #[test]
    fn event_decode_typed_is_none_on_a_type_mismatch() {
        // A mismatch is no error, whatever the bytes are.
        let page = event_page(any(type_url("orders.Other"), undecodable()));
        assert_eq!(page.decode_typed::<Cover>().unwrap(), None);
    }

    #[test]
    fn event_decode_typed_is_none_without_an_event() {
        assert_eq!(EventPage::default().decode_typed::<Cover>().unwrap(), None);
    }

    #[test]
    fn event_decode_typed_surfaces_a_decode_failure() {
        let page = event_page(any(full_type_url::<Cover>(), undecodable()));
        assert_decode_failed(page.decode_typed::<Cover>().unwrap_err());
    }

    #[test]
    fn command_decode_typed_decodes_a_payload_of_its_type() {
        let page = command_page(any(full_type_url::<Cover>(), cover().encode_to_vec()));
        assert_eq!(page.decode_typed::<Cover>().unwrap(), Some(cover()));
    }

    #[test]
    fn command_decode_typed_is_none_on_a_type_mismatch() {
        let page = command_page(any(type_url("orders.Other"), undecodable()));
        assert_eq!(page.decode_typed::<Cover>().unwrap(), None);
    }

    #[test]
    fn command_decode_typed_is_none_without_a_command() {
        assert_eq!(
            CommandPage::default().decode_typed::<Cover>().unwrap(),
            None
        );
    }

    #[test]
    fn command_decode_typed_surfaces_a_decode_failure() {
        let page = command_page(any(full_type_url::<Cover>(), undecodable()));
        assert_decode_failed(page.decode_typed::<Cover>().unwrap_err());
    }
}
