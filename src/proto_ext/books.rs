//! Book extension traits for EventBook and CommandBook.
//!
//! Provides convenience methods for working with pages and sequence numbers.

use crate::proto::{CommandBook, CommandPage, EventBook, EventPage, MergeStrategy, Snapshot};

use super::cover::CoverExt;
use super::pages::{CommandPageExt, EventPageExt};

/// Extension trait for EventBook proto type (beyond CoverExt).
///
/// Provides convenience methods for working with event pages.
pub trait EventBookExt: CoverExt {
    /// Get the next sequence number from the pre-computed field.
    ///
    /// The framework sets this on load. Returns 0 if not set.
    fn next_sequence(&self) -> u32;

    /// Check if the event book has no pages.
    fn is_empty(&self) -> bool;

    /// Get the last event page, if any.
    fn last_page(&self) -> Option<&EventPage>;

    /// Get the first event page, if any.
    fn first_page(&self) -> Option<&EventPage>;
}

/// Compute next sequence number from pages and optional snapshot.
///
/// Returns (last page sequence + 1) OR (snapshot sequence + 1) if no pages, OR 0 if neither.
/// Use this when manually constructing EventBooks or when the framework hasn't set next_sequence.
pub fn calculate_next_sequence(pages: &[EventPage], snapshot: Option<&Snapshot>) -> u32 {
    if let Some(last_page) = pages.last() {
        last_page.sequence_num() + 1
    } else {
        snapshot.map(|s| s.sequence + 1).unwrap_or(0)
    }
}

/// Calculate and set the next_sequence field on an EventBook.
pub fn calculate_set_next_seq(book: &mut EventBook) {
    book.next_sequence = calculate_next_sequence(&book.pages, book.snapshot.as_ref());
}

/// Build a map from root UUID hex to EventBook, for destination lookup.
///
/// Cross-language alias for Python's `destination_map(destinations)`. Used
/// in multi-destination sagas to look up the correct EventBook by aggregate
/// root when stamping command sequences. Entries without a root are skipped.
pub fn destination_map(
    destinations: &[EventBook],
) -> std::collections::HashMap<String, &EventBook> {
    destinations
        .iter()
        .filter_map(|book| book.root_id_hex().map(|hex| (hex, book)))
        .collect()
}

impl EventBookExt for EventBook {
    fn next_sequence(&self) -> u32 {
        self.next_sequence
    }

    fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    fn last_page(&self) -> Option<&EventPage> {
        self.pages.last()
    }

    fn first_page(&self) -> Option<&EventPage> {
        self.pages.first()
    }
}

/// Extension trait for CommandBook proto type (beyond CoverExt).
///
/// Provides convenience methods for working with command pages.
pub trait CommandBookExt: CoverExt {
    /// Get the sequence number from the first command page.
    fn command_sequence(&self) -> u32;

    /// Get the first command page, if any.
    fn first_command(&self) -> Option<&CommandPage>;

    /// Get the merge strategy from the first command page.
    ///
    /// Returns the MergeStrategy enum value. Defaults to Commutative if no pages.
    fn merge_strategy(&self) -> MergeStrategy;
}

impl CommandBookExt for CommandBook {
    fn command_sequence(&self) -> u32 {
        self.pages.first().map(|p| p.sequence_num()).unwrap_or(0)
    }

    fn first_command(&self) -> Option<&CommandPage> {
        self.pages.first()
    }

    fn merge_strategy(&self) -> MergeStrategy {
        self.pages
            .first()
            .map(|p| p.merge_strategy())
            .unwrap_or(MergeStrategy::MergeCommutative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{Cover, Uuid as ProtoUuid};

    fn book_with_root(bytes: [u8; 16]) -> EventBook {
        EventBook {
            cover: Some(Cover {
                root: Some(ProtoUuid {
                    value: bytes.to_vec(),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn destination_map_keys_by_root_hex_and_skips_rootless() {
        let with_root = book_with_root([0xAB; 16]);
        let without_root = EventBook {
            cover: Some(Cover::default()),
            ..Default::default()
        };
        let dests = vec![with_root.clone(), without_root];
        let map = destination_map(&dests);

        assert_eq!(map.len(), 1, "rootless EventBook must be skipped");
        let key = with_root.root_id_hex().unwrap();
        assert!(map.contains_key(&key));
    }
}
