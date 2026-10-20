//! Book extension traits for EventBook and CommandBook.
//!
//! Provides convenience methods for working with pages and sequence numbers.

use crate::proto::{CommandBook, CommandPage, ConflictResolution, EventBook, EventPage, Snapshot};

use super::cover::CoverExt;

/// Extension trait for EventBook proto type (beyond CoverExt).
///
/// Provides convenience methods for working with event pages.
pub trait EventBookExt: CoverExt {
    /// Get the next sequence number from the pre-computed field.
    ///
    /// The framework sets this on load. Returns 0 if not set.
    fn next_sequence(&self) -> u64;

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
pub fn calculate_next_sequence(pages: &[EventPage], snapshot: Option<&Snapshot>) -> u64 {
    if let Some(last_page) = pages.last() {
        last_page.sequence + 1
    } else {
        snapshot.map(|s| s.sequence + 1).unwrap_or(0)
    }
}

/// Calculate and set the next_sequence field on an EventBook.
pub fn calculate_set_next_seq(book: &mut EventBook) {
    book.next_sequence = calculate_next_sequence(&book.pages, book.snapshot.as_ref());
}

impl EventBookExt for EventBook {
    fn next_sequence(&self) -> u64 {
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
/// A CommandBook carries one command page and the command's `Handling`.
pub trait CommandBookExt: CoverExt {
    /// The expected sequence the command was issued against
    /// (`Handling.expected_sequence`); `None` when the framework is to
    /// stamp the stream's head.
    fn expected_sequence(&self) -> Option<u64>;

    /// The command page, if any.
    fn command_page(&self) -> Option<&CommandPage>;

    /// The requested conflict resolution (`Handling.conflict_resolution`);
    /// `Unspecified` (the configured default applies) when unset.
    fn conflict_resolution(&self) -> ConflictResolution;
}

impl CommandBookExt for CommandBook {
    fn expected_sequence(&self) -> Option<u64> {
        self.handling.as_ref().and_then(|h| h.expected_sequence)
    }

    fn command_page(&self) -> Option<&CommandPage> {
        self.page.as_ref()
    }

    fn conflict_resolution(&self) -> ConflictResolution {
        self.handling
            .as_ref()
            .map(|h| h.conflict_resolution())
            .unwrap_or(ConflictResolution::Unspecified)
    }
}

#[cfg(test)]
#[path = "books.test.rs"]
mod tests;
