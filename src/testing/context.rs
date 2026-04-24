//! Scenario context for BDD-style testing.
//!
//! Tracks state across Given/When/Then steps — current aggregate, event
//! history, command results, and rebuilt state. Mirrors Python's
//! `angzarr_client.testing.ScenarioContext`.

use prost::Message;
use prost_types::Any;

use crate::error::CommandRejectedError;
use crate::proto::EventBook;

use super::builders::{make_cover, make_event_book, make_event_page, pack_event};

/// Shared context for BDD test scenarios.
#[derive(Debug, Default)]
pub struct ScenarioContext {
    /// Current aggregate domain being tested.
    pub domain: String,
    /// Current aggregate root as 16 bytes.
    pub root: [u8; 16],
    /// Event history (packed `Any` events).
    pub events: Vec<Any>,
    /// Last command handler result (opaque — caller casts as needed).
    pub result: Option<Box<dyn std::any::Any + Send>>,
    /// Last `CommandRejectedError` if command was rejected.
    pub error: Option<CommandRejectedError>,
    /// Rebuilt aggregate state after applying events (opaque).
    pub state: Option<Box<dyn std::any::Any + Send>>,
}

impl ScenarioContext {
    /// Fresh empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build an `EventBook` from accumulated events. Sequences are 0..N.
    pub fn event_book(&self) -> EventBook {
        let pages = self
            .events
            .iter()
            .enumerate()
            .map(|(i, ev)| make_event_page(i as u32, ev.clone()))
            .collect::<Vec<_>>();
        let cover = make_cover(self.domain.clone(), self.root, "");
        make_event_book(cover, pages, None)
    }

    /// Pack `event_msg` and append to history.
    pub fn add_event<M: Message>(&mut self, event_msg: &M, type_name: &str) {
        self.events.push(pack_event(event_msg, type_name));
    }

    /// Clear all events from history.
    pub fn clear_events(&mut self) {
        self.events.clear();
    }

    /// Clear the last result and error.
    pub fn clear_result(&mut self) {
        self.result = None;
        self.error = None;
    }

    /// Reset context to initial state.
    pub fn reset(&mut self) {
        self.domain.clear();
        self.root = [0u8; 16];
        self.events.clear();
        self.result = None;
        self.error = None;
        self.state = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_book_sequences_and_cover() {
        let mut ctx = ScenarioContext::new();
        ctx.domain = "player".into();
        ctx.root = [2u8; 16];
        ctx.events.push(Any {
            type_url: "t".into(),
            value: vec![],
        });
        ctx.events.push(Any {
            type_url: "t".into(),
            value: vec![],
        });
        let book = ctx.event_book();
        assert_eq!(book.pages.len(), 2);
        assert_eq!(book.next_sequence, 2);
        assert_eq!(book.cover.as_ref().unwrap().domain, "player");
    }

    #[test]
    fn reset_clears_all() {
        let mut ctx = ScenarioContext::new();
        ctx.domain = "x".into();
        ctx.root = [9u8; 16];
        ctx.events.push(Any {
            type_url: "t".into(),
            value: vec![],
        });
        ctx.reset();
        assert!(ctx.domain.is_empty());
        assert_eq!(ctx.root, [0u8; 16]);
        assert!(ctx.events.is_empty());
    }
}
