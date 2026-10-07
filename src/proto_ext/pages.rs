//! Page extension traits for EventPage and CommandPage.
//!
//! Provides convenient accessors for sequence, type URL, and payload decoding.

use crate::proto::page_header::SequenceType;
use crate::proto::{
    AngzarrDeferredSequence, CommandPage, EventPage, ExternalDeferredSequence, MergeStrategy,
    PageHeader,
};
use prost::Name;

/// True when a wire `type_url` names message type `M` (any prefix).
fn type_url_matches<M: Name>(type_url: &str) -> bool {
    crate::convert::type_url_is::<M>(type_url)
}

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
    /// Returns None if the event is missing, type URL doesn't match exactly,
    /// or decoding fails. The expected type URL is derived from M::full_name().
    fn decode_typed<M: prost::Message + Default + Name>(&self) -> Option<M>;
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

    fn decode_typed<M: prost::Message + Default + Name>(&self) -> Option<M> {
        let event = match &self.payload {
            Some(crate::proto::event_page::Payload::Event(e)) => e,
            _ => return None,
        };
        if !type_url_matches::<M>(&event.type_url) {
            return None;
        }
        M::decode(event.value.as_slice()).ok()
    }
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
    /// Returns None if the command is missing, type URL doesn't match exactly,
    /// or decoding fails. The expected type URL is derived from M::full_name().
    fn decode_typed<M: prost::Message + Default + Name>(&self) -> Option<M>;

    /// Get the merge strategy for this command.
    ///
    /// Returns the MergeStrategy enum value; `MERGE_UNSPECIFIED` (unset) and
    /// unknown values read as the documented default, Commutative.
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

    fn decode_typed<M: prost::Message + Default + Name>(&self) -> Option<M> {
        let command = match &self.payload {
            Some(crate::proto::command_page::Payload::Command(c)) => c,
            _ => return None,
        };
        if !type_url_matches::<M>(&command.type_url) {
            return None;
        }
        M::decode(command.value.as_slice()).ok()
    }

    fn merge_strategy(&self) -> MergeStrategy {
        match MergeStrategy::try_from(self.merge_strategy) {
            Ok(MergeStrategy::MergeUnspecified) | Err(_) => MergeStrategy::MergeCommutative,
            Ok(s) => s,
        }
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
    use crate::proto::CommandPage;

    #[test]
    fn decode_typed_matches_by_full_name_whatever_the_prefix() {
        use crate::proto::{command_page, event_page, Cover, EventPage};
        let value = prost::Message::encode_to_vec(&Cover {
            domain: "d".into(),
            ..Default::default()
        });
        for url in [
            "/io.angzarr.v1.Cover",
            "type.googleapis.com/io.angzarr.v1.Cover",
        ] {
            let any = prost_types::Any {
                type_url: url.into(),
                value: value.clone(),
            };
            let ev = EventPage {
                payload: Some(event_page::Payload::Event(any.clone())),
                ..Default::default()
            };
            assert_eq!(
                ev.decode_typed::<Cover>().map(|c| c.domain),
                Some("d".into())
            );
            let cmd = CommandPage {
                payload: Some(command_page::Payload::Command(any)),
                ..Default::default()
            };
            assert_eq!(
                cmd.decode_typed::<Cover>().map(|c| c.domain),
                Some("d".into())
            );
        }
        let other = EventPage {
            payload: Some(event_page::Payload::Event(prost_types::Any {
                type_url: "/io.angzarr.v1.Edition".into(),
                value,
            })),
            ..Default::default()
        };
        assert!(other.decode_typed::<Cover>().is_none());
    }

    /// MERGE_UNSPECIFIED (unset) and unknown values read as the documented
    /// default, Commutative; set values read by name.
    #[test]
    fn merge_strategy_reads_unset_as_commutative() {
        let page = |v: i32| CommandPage {
            merge_strategy: v,
            ..Default::default()
        };
        let read = |p: CommandPage| CommandPageExt::merge_strategy(&p);
        assert_eq!(read(page(0)), MergeStrategy::MergeCommutative);
        assert_eq!(read(page(99)), MergeStrategy::MergeCommutative);
        for s in [
            MergeStrategy::MergeCommutative,
            MergeStrategy::MergeStrict,
            MergeStrategy::MergeAggregateHandles,
            MergeStrategy::MergeManual,
        ] {
            assert_eq!(read(page(s as i32)), s);
        }
    }
}
