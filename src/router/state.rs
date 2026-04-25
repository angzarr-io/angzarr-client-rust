//! Destination-sequence bookkeeping for sagas and process managers.
//!
//! Sagas and process managers that emit commands for other domains must stamp
//! each outbound command with the target domain's next sequence number. The
//! framework supplies this map as `destination_sequences` on the request; user
//! code wraps it with [`Destinations`] and calls [`Destinations::stamp_command`]
//! when building the outbound `CommandBook`.

use std::collections::HashMap;

use crate::proto::CommandBook;

/// Wraps the `destination_sequences` map from a saga/PM request and exposes
/// helpers for stamping outbound commands with the correct sequence.
///
/// # Example
///
/// ```rust,ignore
/// fn handle_order_completed(
///     &self,
///     event: OrderCompleted,
///     destinations: &Destinations,
/// ) -> CommandResult<SagaResponse> {
///     let mut cmd = create_some_command();
///     destinations.stamp_command(&mut cmd, "inventory")?;
///     Ok(SagaResponse { commands: vec![cmd], events: vec![] })
/// }
/// ```
pub struct Destinations {
    sequences: HashMap<String, u32>,
}

impl Default for Destinations {
    fn default() -> Self {
        Self::new()
    }
}

impl Destinations {
    /// Create empty Destinations.
    pub fn new() -> Self {
        Self {
            sequences: HashMap::new(),
        }
    }

    /// Build Destinations from a destination_sequences map.
    pub fn from_sequences(sequences: HashMap<String, u32>) -> Self {
        Self { sequences }
    }

    /// Get the next sequence number for a domain.
    ///
    /// Returns `None` if no sequence is available for this domain.
    pub fn sequence_for(&self, domain: &str) -> Option<u32> {
        self.sequences.get(domain).copied()
    }

    /// Stamp a command with the correct sequence for the destination domain.
    ///
    /// Sets the sequence number on all command pages for the given domain.
    ///
    /// # Errors
    ///
    /// Returns an error if no sequence is available for this domain.
    pub fn stamp_command(&self, cmd: &mut CommandBook, domain: &str) -> Result<(), String> {
        use crate::proto::page_header::SequenceType;

        let seq = self.sequences.get(domain).ok_or_else(|| {
            format!(
                "No sequence for domain '{}' - check output_domains config",
                domain
            )
        })?;

        for page in &mut cmd.pages {
            let header = page.header.get_or_insert_with(Default::default);
            header.sequence_type = Some(SequenceType::Sequence(*seq));
        }

        Ok(())
    }

    /// Check if a domain has a sequence available.
    pub fn has_sequence(&self, domain: &str) -> bool {
        self.sequences.contains_key(domain)
    }

    /// Get all domain names that have sequences.
    pub fn domains(&self) -> impl Iterator<Item = &str> {
        self.sequences.keys().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_from_sequences() {
        let mut seqs = HashMap::new();
        seqs.insert("order".to_string(), 5u32);
        seqs.insert("inventory".to_string(), 10u32);

        let destinations = Destinations::from_sequences(seqs);

        assert_eq!(destinations.sequence_for("order"), Some(5));
        assert_eq!(destinations.sequence_for("inventory"), Some(10));
        assert_eq!(destinations.sequence_for("unknown"), None);
    }

    #[test]
    fn destinations_has_sequence() {
        let mut seqs = HashMap::new();
        seqs.insert("order".to_string(), 5u32);
        let destinations = Destinations::from_sequences(seqs);

        assert!(destinations.has_sequence("order"));
        assert!(!destinations.has_sequence("inventory"));
    }

    #[test]
    fn destinations_domains() {
        let mut seqs = HashMap::new();
        seqs.insert("order".to_string(), 5u32);
        seqs.insert("inventory".to_string(), 10u32);
        let destinations = Destinations::from_sequences(seqs);

        let domains: Vec<_> = destinations.domains().collect();
        assert_eq!(domains.len(), 2);
        assert!(domains.contains(&"order"));
        assert!(domains.contains(&"inventory"));
    }

    #[test]
    fn destinations_stamp_command() {
        use crate::proto::{page_header::SequenceType, CommandPage, PageHeader};

        let mut seqs = HashMap::new();
        seqs.insert("order".to_string(), 42u32);
        let destinations = Destinations::from_sequences(seqs);

        let mut cmd = CommandBook {
            pages: vec![CommandPage {
                header: Some(PageHeader::default()),
                ..Default::default()
            }],
            ..Default::default()
        };

        destinations.stamp_command(&mut cmd, "order").unwrap();
        assert_eq!(
            cmd.pages[0].header.as_ref().unwrap().sequence_type,
            Some(SequenceType::Sequence(42))
        );
    }

    #[test]
    fn destinations_stamp_command_missing_domain() {
        let destinations = Destinations::new();
        let mut cmd = CommandBook::default();

        let result = destinations.stamp_command(&mut cmd, "unknown");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown"));
    }

    /// Wire-format parity with the Python client. Locks the SHA-256 of the
    /// deterministically-encoded stamped CommandBook. The Python sibling
    /// at `client-python/main/tests/test_destinations_wire_parity.py`
    /// asserts the same hash for the same input. If either side changes
    /// how `stamp_command` modifies wire bytes, both tests must agree on
    /// the new value — drift fails on at least one side.
    #[test]
    fn destinations_stamp_command_wire_parity() {
        use crate::proto::{
            command_page::Payload as CmdPayload, CommandPage, Cover, Uuid as ProtoUuid,
        };
        use prost::Message;
        use prost_types::Any as ProtoAny;
        use sha2::{Digest, Sha256};

        // Same fixed input as the Python test.
        let root_bytes: Vec<u8> = (0u8..16).collect();
        let domain = "saga-x";
        let correlation_id = "corr-1";
        let command_type_url = "type.googleapis.com/example.Foo";
        let command_payload: Vec<u8> = vec![1, 2, 3, 4];
        let target_domain = "inventory";
        let target_sequence = 5u32;
        const GOLDEN_SHA256: &str =
            "8a6da2dfa422553d73fcd840f6ad501c91ac6ffcac2f591183146ab6c042ace9";

        let payload_any = ProtoAny {
            type_url: command_type_url.into(),
            value: command_payload,
        };
        let mut book = CommandBook {
            cover: Some(Cover {
                domain: domain.into(),
                root: Some(ProtoUuid {
                    value: root_bytes,
                }),
                correlation_id: correlation_id.into(),
                edition: None,
            }),
            pages: vec![CommandPage {
                header: None,
                merge_strategy: 0,
                payload: Some(CmdPayload::Command(payload_any)),
            }],
            ..Default::default()
        };

        let mut seqs = HashMap::new();
        seqs.insert(target_domain.to_string(), target_sequence);
        Destinations::from_sequences(seqs)
            .stamp_command(&mut book, target_domain)
            .expect("stamp must succeed");

        let raw = book.encode_to_vec();
        let digest = format!("{:x}", Sha256::digest(&raw));

        assert_eq!(
            digest, GOLDEN_SHA256,
            "Stamped CommandBook wire bytes drifted from Python client.\n\
             If this is intentional, update the golden in BOTH this test and\n\
             client-python/main/tests/test_destinations_wire_parity.py in tandem.\n\
             actual:   {}\n\
             expected: {}",
            digest, GOLDEN_SHA256
        );
    }
}
