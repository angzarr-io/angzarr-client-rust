//! Cross-language response carriers surfaced at the public root.
//!
//! These types mirror the Python client's saga/process-manager/rejection
//! handler returns. They are intentionally small plain structs — the
//! framework wraps their contents into the underlying `SagaResponse` /
//! `ProcessManagerHandleResponse` / `BusinessResponse` proto responses
//! during dispatch.

use crate::proto::{CommandBook, EventBook, Notification};

/// Return shape of a saga handler method.
///
/// `commands` are dispatched outbound; `events` are retained locally by
/// the saga runtime (for audit / replay). Mirrors Python's
/// `SagaHandlerResponse`.
#[derive(Debug, Clone, Default)]
pub struct SagaHandlerResponse {
    pub commands: Vec<CommandBook>,
    pub events: Vec<EventBook>,
}

/// Return shape of a process-manager handler method.
///
/// `commands` orchestrate downstream aggregates; `process_events` are
/// persisted on the PM aggregate itself; `facts` are informational
/// events emitted into the meta domain. Mirrors Python's
/// `ProcessManagerResponse`.
#[derive(Debug, Clone, Default)]
pub struct ProcessManagerResponse {
    pub commands: Vec<CommandBook>,
    pub process_events: Vec<EventBook>,
    pub facts: Vec<EventBook>,
}

/// Return shape of a rejection (saga `#[rejected]`) handler method.
///
/// `events` are the compensation events the handler wants to persist;
/// `notification` is the upstream ack/nack carried back to the caller.
/// Mirrors Python's `RejectionHandlerResponse`.
#[derive(Debug, Clone, Default)]
pub struct RejectionHandlerResponse {
    pub events: Vec<EventBook>,
    pub notification: Option<Notification>,
}
