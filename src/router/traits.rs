//! Handler traits for each component type.
//!
//! Each trait defines the contract for domain handlers. Implementors
//! encapsulate their routing logic internally and declare which types
//! they handle via `command_types()` or `event_types()`.

use prost_types::Any;
use std::error::Error;
use tonic::Status;

use crate::proto::{CommandBook, Cover, EventBook, Notification, Projection};
use crate::router::state::Destinations;
use crate::router::StateRouter;

// ============================================================================
// Common Types
// ============================================================================

/// Error type for command/event rejection with a human-readable reason.
#[derive(Debug, Clone)]
pub struct CommandRejectedError {
    pub reason: String,
}

impl CommandRejectedError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for CommandRejectedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Command rejected: {}", self.reason)
    }
}

impl std::error::Error for CommandRejectedError {}

impl From<CommandRejectedError> for Status {
    fn from(err: CommandRejectedError) -> Self {
        Status::failed_precondition(err.reason)
    }
}

/// Result type for handlers.
pub type CommandResult<T> = std::result::Result<T, CommandRejectedError>;

/// Response from rejection handlers.
///
/// Handlers may return:
/// - Events to compensate/fix state
/// - Notification to forward upstream
/// - Both
#[derive(Default)]
pub struct RejectionHandlerResponse {
    /// Events to persist (compensation).
    pub events: Option<EventBook>,
    /// Notification to forward upstream.
    pub notification: Option<Notification>,
}

/// Response from saga handlers.
#[derive(Default)]
pub struct SagaHandlerResponse {
    /// Commands to send to other aggregates.
    pub commands: Vec<CommandBook>,
    /// Facts/events to inject to other aggregates.
    pub events: Vec<EventBook>,
}

/// Response from process manager handlers.
#[derive(Default)]
pub struct ProcessManagerResponse {
    /// Commands to send to other aggregates.
    pub commands: Vec<CommandBook>,
    /// Events to persist to the PM's own domain.
    pub process_events: Option<EventBook>,
    /// Facts to inject to other aggregates.
    pub facts: Vec<EventBook>,
}

/// Helper trait for unpacking Any messages.
pub trait UnpackAny {
    /// Unpack an Any to a specific message type.
    fn unpack<M: prost::Message + Default>(&self) -> Result<M, prost::DecodeError>;
}

impl UnpackAny for Any {
    fn unpack<M: prost::Message + Default>(&self) -> Result<M, prost::DecodeError> {
        M::decode(self.value.as_slice())
    }
}

// ============================================================================
// Command Handler
// ============================================================================

/// Handler for a single domain's command handler logic.
///
/// Command handlers receive commands and emit events. They maintain state
/// that is rebuilt from events using a `StateRouter`.
///
/// # Example
///
/// ```rust,ignore
/// struct PlayerHandler {
///     state_router: StateRouter<PlayerState>,
/// }
///
/// impl CommandHandlerDomainHandler for PlayerHandler {
///     type State = PlayerState;
///
///     fn command_types(&self) -> Vec<String> {
///         vec!["RegisterPlayer".into(), "DepositFunds".into()]
///     }
///
///     fn state_router(&self) -> &StateRouter<Self::State> {
///         &self.state_router
///     }
///
///     fn handle(
///         &self,
///         cmd: &CommandBook,
///         payload: &Any,
///         state: &Self::State,
///         seq: u32,
///     ) -> CommandResult<EventBook> {
///         dispatch_command!(payload, cmd, state, seq, {
///             "RegisterPlayer" => self.handle_register,
///             "DepositFunds" => self.handle_deposit,
///         })
///     }
/// }
/// ```
pub trait CommandHandlerDomainHandler: Send + Sync {
    /// The state type for this aggregate.
    type State: Default + 'static;

    /// Command type suffixes this handler processes.
    ///
    /// Used for subscription derivation and routing.
    fn command_types(&self) -> Vec<String>;

    /// Get the state router for rebuilding state from events.
    fn state_router(&self) -> &StateRouter<Self::State>;

    /// Rebuild state from events.
    ///
    /// Default implementation uses `state_router().with_event_book()`.
    fn rebuild(&self, events: &EventBook) -> Self::State {
        self.state_router().with_event_book(events)
    }

    /// Handle a command and return resulting events.
    ///
    /// The handler should dispatch internally based on `payload.type_url`.
    fn handle(
        &self,
        cmd: &CommandBook,
        payload: &Any,
        state: &Self::State,
        seq: u32,
    ) -> CommandResult<EventBook>;

    /// Handle fact events — update aggregate state based on external realities.
    ///
    /// Called when the coordinator routes facts through the aggregate's business logic.
    /// The handler receives the facts plus prior events (for state reconstruction) and
    /// returns an EventBook with events to persist.
    ///
    /// Default implementation returns the facts as-is (pass-through).
    fn handle_fact(&self, facts: &EventBook, _state: &Self::State) -> CommandResult<EventBook> {
        Ok(facts.clone())
    }

    /// Handle a rejection notification.
    ///
    /// Called when a command issued by a saga/PM targeting this aggregate's
    /// domain was rejected. Override to provide custom compensation logic.
    ///
    /// Default implementation returns an empty response (framework handles).
    fn on_rejected(
        &self,
        _notification: &Notification,
        _state: &Self::State,
        _target_domain: &str,
        _target_command: &str,
    ) -> CommandResult<RejectionHandlerResponse> {
        Ok(RejectionHandlerResponse::default())
    }
}

// ============================================================================
// Saga Handler
// ============================================================================

/// Handler for a single domain's events in a saga.
///
/// Sagas are **pure translators**: they receive source events and produce
/// commands for target domains.
///
/// # Design Philosophy
///
/// Sagas should NOT rebuild destination state to make decisions. Instead:
/// - Use destination sequences for command stamping via `destinations.stamp_command()`
/// - Let destination aggregates make business decisions
/// - Use sync mode for immediate feedback and error handling
/// - Inject facts if external information is needed
///
/// # Example
///
/// ```rust,ignore
/// struct OrderSagaHandler;
///
/// impl SagaDomainHandler for OrderSagaHandler {
///     fn event_types(&self) -> Vec<String> {
///         vec!["OrderCompleted".into(), "OrderCancelled".into()]
///     }
///
///     fn handle(
///         &self,
///         source: &EventBook,
///         event: &Any,
///         destinations: &Destinations,
///     ) -> CommandResult<SagaHandlerResponse> {
///         // Create command and stamp with destination sequence
///         let mut cmd = create_reserve_inventory_command(event);
///         destinations.stamp_command(&mut cmd, "inventory")
///             .map_err(|e| CommandRejectedError::new(&e))?;
///
///         // Aggregate makes business decisions (e.g., out of stock)
///         // Saga handles rejection via on_rejected
///         Ok(SagaHandlerResponse::with_commands(vec![cmd]))
///     }
/// }
/// ```
pub trait SagaDomainHandler: Send + Sync {
    /// Event type suffixes this handler processes.
    ///
    /// Used for subscription derivation.
    fn event_types(&self) -> Vec<String>;

    /// Translate source events into commands for target domains.
    ///
    /// The `destinations` parameter provides destination sequences for command
    /// stamping. Use `destinations.stamp_command(&mut cmd, "domain")` to stamp
    /// the correct sequence before returning commands.
    fn handle(
        &self,
        source: &EventBook,
        event: &Any,
        destinations: &Destinations,
    ) -> CommandResult<SagaHandlerResponse>;

    /// Handle a rejection notification.
    ///
    /// Called when a saga-issued command was rejected. Override to provide
    /// custom compensation logic.
    ///
    /// Default implementation returns an empty response (framework handles).
    fn on_rejected(
        &self,
        _notification: &Notification,
        _target_domain: &str,
        _target_command: &str,
    ) -> CommandResult<RejectionHandlerResponse> {
        Ok(RejectionHandlerResponse::default())
    }
}

// ============================================================================
// Process Manager Handler
// ============================================================================

/// Handler for a single domain's events in a process manager.
///
/// Process managers correlate events across multiple domains and maintain
/// their own state. Each domain gets its own handler, but they all share
/// the same PM state type.
///
/// # Design Philosophy
///
/// PMs should NOT rebuild destination state to make decisions. Instead:
/// - Use destination sequences for command stamping via `destinations.stamp_command()`
/// - Let destination aggregates make business decisions
/// - Use sync mode for immediate feedback and error handling
/// - Inject facts if external information is needed
///
/// # Example
///
/// ```rust,ignore
/// struct BuyInPmHandler;
///
/// impl ProcessManagerDomainHandler<BuyInState> for BuyInPmHandler {
///     fn event_types(&self) -> Vec<String> {
///         vec!["BuyInRequested".into()]
///     }
///
///     fn prepare(&self, trigger: &EventBook, state: &BuyInState, event: &Any) -> Vec<Cover> {
///         // Declare needed destinations (for sequence fetching)
///         vec![Cover { domain: "table".into(), .. }]
///     }
///
///     fn handle(
///         &self,
///         trigger: &EventBook,
///         state: &BuyInState,
///         event: &Any,
///         destinations: &Destinations,
///     ) -> CommandResult<ProcessManagerResponse> {
///         // Create command and stamp with destination sequence
///         let mut cmd = create_add_player_command(event);
///         destinations.stamp_command(&mut cmd, "table")
///             .map_err(|e| CommandRejectedError::new(&e))?;
///
///         // Aggregate makes business decisions (e.g., table full)
///         // PM handles rejection via on_rejected
///         Ok(ProcessManagerResponse::with_commands(vec![cmd]))
///     }
/// }
/// ```
pub trait ProcessManagerDomainHandler<S>: Send + Sync {
    /// Event type suffixes this handler processes.
    fn event_types(&self) -> Vec<String>;

    /// Prepare phase — declare destination covers needed.
    fn prepare(&self, trigger: &EventBook, state: &S, event: &Any) -> Vec<Cover>;

    /// Handle phase — produce commands and PM events.
    ///
    /// The `destinations` parameter provides destination sequences for command
    /// stamping. Use `destinations.stamp_command(&mut cmd, "domain")` to stamp
    /// the correct sequence before returning commands.
    fn handle(
        &self,
        trigger: &EventBook,
        state: &S,
        event: &Any,
        destinations: &Destinations,
    ) -> CommandResult<ProcessManagerResponse>;

    /// Handle a rejection notification.
    ///
    /// Called when a PM-issued command was rejected. Override to provide
    /// custom compensation logic.
    fn on_rejected(
        &self,
        _notification: &Notification,
        _state: &S,
        _target_domain: &str,
        _target_command: &str,
    ) -> CommandResult<RejectionHandlerResponse> {
        Ok(RejectionHandlerResponse::default())
    }
}

// ============================================================================
// Projector Handler
// ============================================================================

/// Handler for a single domain's events in a projector.
///
/// Projectors consume events and produce external output (read models,
/// caches, external systems).
///
/// # Example
///
/// ```rust,ignore
/// struct PlayerProjectorHandler;
///
/// impl ProjectorDomainHandler for PlayerProjectorHandler {
///     fn event_types(&self) -> Vec<String> {
///         vec!["PlayerRegistered".into(), "FundsDeposited".into()]
///     }
///
///     fn project(&self, events: &EventBook) -> Result<Projection, Box<dyn Error + Send + Sync>> {
///         // Update external read model
///         Ok(Projection::default())
///     }
/// }
/// ```
pub trait ProjectorDomainHandler: Send + Sync {
    /// Event type suffixes this handler processes.
    fn event_types(&self) -> Vec<String>;

    /// Project events to external output.
    fn project(&self, events: &EventBook) -> Result<Projection, Box<dyn Error + Send + Sync>>;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_rejected_error_display() {
        let err = CommandRejectedError::new("insufficient funds");
        assert_eq!(err.to_string(), "Command rejected: insufficient funds");
    }

    #[test]
    fn command_rejected_error_to_status() {
        let err = CommandRejectedError::new("invalid input");
        let status: Status = err.into();
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
    }

    #[test]
    fn rejection_handler_response_default() {
        let response = RejectionHandlerResponse::default();
        assert!(response.events.is_none());
        assert!(response.notification.is_none());
    }

    #[test]
    fn process_manager_response_default() {
        let response = ProcessManagerResponse::default();
        assert!(response.commands.is_empty());
        assert!(response.process_events.is_none());
        assert!(response.facts.is_empty());
    }

    #[test]
    fn saga_handler_response_default() {
        let response = SagaHandlerResponse::default();
        assert!(response.commands.is_empty());
        assert!(response.events.is_empty());
    }
}
