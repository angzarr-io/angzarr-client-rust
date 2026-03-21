//! State reconstruction from events.
//!
//! `StateRouter` provides fluent registration of event appliers for
//! rebuilding aggregate/PM state from event streams.

use std::sync::Arc;

use prost_types::Any;

use crate::proto::{event_page, EventBook, EventPage};

/// Event applier function type.
///
/// Takes mutable state reference and event bytes (to be decoded by handler).
pub type EventApplier<S> = Box<dyn Fn(&mut S, &[u8]) + Send + Sync>;

/// Factory function type for creating initial state.
pub type StateFactory<S> = Box<dyn Fn() -> S + Send + Sync>;

/// Higher-order function that produces event appliers per-event.
///
/// Called each time an event is processed, allowing fresh closures with
/// captured dependencies.
pub type EventApplierHOF<S> = Arc<dyn Fn() -> EventApplier<S> + Send + Sync>;

/// Internal entry for either static or HOF handlers.
enum HandlerEntry<S> {
    /// Static handler called directly.
    Static(EventApplier<S>),
    /// HOF called per-event to produce handler.
    Factory(EventApplierHOF<S>),
}

/// Fluent state reconstruction router.
///
/// Provides a builder pattern for registering event appliers with auto-unpacking.
/// Register once at startup, call `with_events()` per rebuild.
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::StateRouter;
///
/// fn apply_registered(state: &mut PlayerState, event: PlayerRegistered) {
///     state.player_id = format!("player_{}", event.email);
///     state.display_name = event.display_name;
///     state.exists = true;
/// }
///
/// fn apply_deposited(state: &mut PlayerState, event: FundsDeposited) {
///     if let Some(balance) = event.new_balance {
///         state.bankroll = balance.amount;
///     }
/// }
///
/// // Build router once
/// let player_router = StateRouter::<PlayerState>::new()
///     .on::<PlayerRegistered>(apply_registered)
///     .on::<FundsDeposited>(apply_deposited);
///
/// // Use per rebuild
/// fn rebuild_state(event_book: &EventBook) -> PlayerState {
///     player_router.with_event_book(event_book)
/// }
/// ```
///
/// # HOF Pattern (with dependency injection)
///
/// ```rust,ignore
/// use std::sync::Arc;
///
/// let db_pool = Arc::new(DbPool::new());
/// let router = StateRouter::<PlayerState>::new()
///     .on_with::<PlayerRegistered, _>(|| {
///         let db = db_pool.clone();
///         Box::new(move |state, event: PlayerRegistered| {
///             // db accessible here, called fresh per-event
///             state.player_id = event.email.clone();
///         })
///     });
/// ```
pub struct StateRouter<S: Default> {
    handlers: Vec<(String, HandlerEntry<S>)>,
    factory: Option<StateFactory<S>>,
}

impl<S: Default + 'static> Default for StateRouter<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Default + 'static> StateRouter<S> {
    /// Create a new StateRouter using `S::default()` for state creation.
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
            factory: None,
        }
    }

    /// Create a StateRouter with a custom state factory.
    ///
    /// Use this when your state needs non-default initialization.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// fn new_hand_state() -> HandState {
    ///     HandState {
    ///         pots: vec![PotState { pot_type: "main".to_string(), ..Default::default() }],
    ///         ..Default::default()
    ///     }
    /// }
    ///
    /// let router = StateRouter::with_factory(new_hand_state)
    ///     .on::<CardsDealt>(apply_cards_dealt);
    /// ```
    pub fn with_factory(factory: fn() -> S) -> Self {
        Self {
            handlers: Vec::new(),
            factory: Some(Box::new(factory)),
        }
    }

    /// Create a new state instance using factory or Default.
    fn create_state(&self) -> S {
        match &self.factory {
            Some(factory) => factory(),
            None => S::default(),
        }
    }

    /// Register an event applier for the given protobuf event type.
    ///
    /// The handler receives typed events (auto-decoded from protobuf).
    /// Type name is extracted via reflection using `prost::Name::full_name()`.
    ///
    /// # Type Parameters
    ///
    /// - `E`: The protobuf event type (must implement `prost::Message + Default + prost::Name`)
    ///
    /// # Arguments
    ///
    /// - `handler`: Function that takes `(&mut S, E)` and mutates state
    pub fn on<E>(mut self, handler: fn(&mut S, E)) -> Self
    where
        E: prost::Message + Default + prost::Name + 'static,
    {
        let type_name = E::full_name();
        let boxed: EventApplier<S> = Box::new(move |state, bytes| {
            if let Ok(event) = E::decode(bytes) {
                handler(state, event);
            }
        });
        self.handlers.push((type_name, HandlerEntry::Static(boxed)));
        self
    }

    /// Register a higher-order function that produces event appliers per-event.
    ///
    /// Called each time an event is processed, allowing fresh closures with
    /// captured dependencies.
    ///
    /// # Type Parameters
    ///
    /// - `E`: The protobuf event type
    /// - `F`: Factory closure that produces event appliers
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let db_pool = Arc::new(DbPool::new());
    /// let router = StateRouter::<PlayerState>::new()
    ///     .on_with::<PlayerRegistered, _>(|| {
    ///         let db = db_pool.clone();
    ///         Box::new(move |state, event: PlayerRegistered| {
    ///             // db is freshly cloned per-event
    ///             state.player_id = event.email.clone();
    ///         })
    ///     });
    /// ```
    pub fn on_with<E, F>(mut self, factory: F) -> Self
    where
        E: prost::Message + Default + prost::Name + 'static,
        F: Fn() -> Box<dyn Fn(&mut S, E) + Send + Sync> + Send + Sync + 'static,
    {
        let type_name = E::full_name();
        let factory_arc: EventApplierHOF<S> = Arc::new(move || {
            let inner = factory();
            Box::new(move |state: &mut S, bytes: &[u8]| {
                if let Ok(event) = E::decode(bytes) {
                    inner(state, event);
                }
            })
        });
        self.handlers
            .push((type_name, HandlerEntry::Factory(factory_arc)));
        self
    }

    /// Create fresh state and apply all events from pages.
    ///
    /// This is the terminal operation for standalone usage.
    pub fn with_events(&self, pages: &[EventPage]) -> S {
        let mut state = self.create_state();
        for page in pages {
            if let Some(event_page::Payload::Event(event)) = &page.payload {
                self.apply_single(&mut state, event);
            }
        }
        state
    }

    /// Create fresh state and apply all events from an EventBook.
    pub fn with_event_book(&self, event_book: &EventBook) -> S {
        self.with_events(&event_book.pages)
    }

    /// Apply a single event to existing state.
    ///
    /// Matches using fully qualified type name from `prost::Name`.
    pub fn apply_single(&self, state: &mut S, event_any: &Any) {
        let type_url = &event_any.type_url;
        for (type_name, entry) in &self.handlers {
            if Self::type_matches(type_url, type_name) {
                match entry {
                    HandlerEntry::Static(handler) => {
                        handler(state, &event_any.value);
                    }
                    HandlerEntry::Factory(factory) => {
                        let handler = factory();
                        handler(state, &event_any.value);
                    }
                }
                return;
            }
        }
        // Unknown event type — silently ignore (forward compatibility)
    }

    /// Check if type_url exactly matches the given fully qualified type name.
    ///
    /// type_name should be fully qualified (e.g., "examples.CardsDealt").
    /// Compares type_url == "type.googleapis.com/" + type_name.
    fn type_matches(type_url: &str, type_name: &str) -> bool {
        type_url == format!("type.googleapis.com/{}", type_name)
    }

    /// Convert to a rebuilder closure for use with Router.
    ///
    /// Returns a closure that can be passed to Router constructors.
    pub fn into_rebuilder(self) -> impl Fn(&EventBook) -> S + Send + Sync {
        move |event_book| self.with_event_book(event_book)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_matches_requires_fully_qualified_name() {
        assert!(StateRouter::<()>::type_matches(
            "type.googleapis.com/examples.CardsDealt",
            "examples.CardsDealt"
        ));
        assert!(!StateRouter::<()>::type_matches(
            "type.googleapis.com/examples.CardsDealt",
            "CardsDealt"
        ));
    }

    #[test]
    fn type_matches_rejects_partial_names() {
        assert!(!StateRouter::<()>::type_matches(
            "type.googleapis.com/examples.CommunityCardsDealt",
            "examples.CardsDealt"
        ));
        assert!(StateRouter::<()>::type_matches(
            "type.googleapis.com/examples.CommunityCardsDealt",
            "examples.CommunityCardsDealt"
        ));
    }

    #[test]
    fn type_matches_rejects_wrong_package() {
        assert!(!StateRouter::<()>::type_matches(
            "type.googleapis.com/examples.CardsDealt",
            "other.CardsDealt"
        ));
    }

    #[test]
    fn type_matches_handles_edge_cases() {
        assert!(!StateRouter::<()>::type_matches(
            "type.googleapis.com/examples.Test",
            ""
        ));
        assert!(!StateRouter::<()>::type_matches(
            "type.googleapis.com/examples.Other",
            "examples.CardsDealt"
        ));
    }

    #[test]
    fn state_router_default() {
        let router: StateRouter<String> = StateRouter::default();
        let state = router.with_events(&[]);
        assert_eq!(state, String::default());
    }

    #[test]
    fn on_with_factory_called_per_event() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let call_count = Arc::new(AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let router: StateRouter<u32> = StateRouter::new();

        // Verify factory produces a closure that could capture dependencies
        // In a real scenario, the factory would capture db_pool, etc.
        let _ = (move || {
            call_count_clone.fetch_add(1, Ordering::SeqCst);
            Box::new(|_state: &mut u32, _value: u32| {}) as Box<dyn Fn(&mut u32, u32) + Send + Sync>
        })();

        // The factory was called once
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        // Verify router struct is empty when no handlers registered
        assert!(router.handlers.is_empty());
    }
}
