//! Cross-language response carriers surfaced at the public root.
//!
//! These types mirror the Python client's saga/process-manager/rejection
//! handler returns. They are intentionally small plain structs — the
//! framework wraps their contents into the underlying `SagaResponse` /
//! `ProcessManagerHandleResponse` / `BusinessResponse` proto responses
//! during dispatch.

use prost::{Message, Name};
use prost_types::Any;

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

/// Return shape of a `#[handles_fact]` method: the fact to record, then
/// the events that flag it (e.g. a discrepancy with the aggregate's own
/// view). A fact cannot be refused; flags are how the aggregate answers it.
///
/// A handler may instead return the fact message itself, which records it
/// with no flags.
#[derive(Debug, Clone, PartialEq)]
pub struct FactRecord {
    fact: Any,
    flags: Vec<Any>,
}

impl FactRecord {
    /// Record `fact` with no flags.
    pub fn new<M: Message + Name>(fact: &M) -> Self {
        FactRecord {
            fact: crate::router::component::pack(fact),
            flags: Vec::new(),
        }
    }

    /// Append `event` to the events recorded after the fact.
    #[must_use]
    pub fn flag<M: Message + Name>(mut self, event: &M) -> Self {
        self.flags.push(crate::router::component::pack(event));
        self
    }

    /// The packed fact.
    pub fn fact(&self) -> &Any {
        &self.fact
    }

    /// The packed flag events, in order.
    pub fn flags(&self) -> &[Any] {
        &self.flags
    }
}

/// What a `#[handles_fact]` method may return: a [`FactRecord`], or the
/// fact message itself (recorded with no flags).
pub trait IntoFactRecord {
    /// The router's record of the fact and its flags.
    fn into_fact_record(self) -> angzarr_router::aggregate::FactRecord;
}

impl IntoFactRecord for FactRecord {
    fn into_fact_record(self) -> angzarr_router::aggregate::FactRecord {
        angzarr_router::aggregate::FactRecord {
            fact: self.fact,
            flags: self.flags,
        }
    }
}

impl<M: Message + Name> IntoFactRecord for M {
    fn into_fact_record(self) -> angzarr_router::aggregate::FactRecord {
        FactRecord::new(&self).into_fact_record()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::Cover;

    fn cover(domain: &str) -> Cover {
        Cover {
            domain: domain.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_fact_record_packs_the_fact_then_its_flags_in_order() {
        let record = FactRecord::new(&cover("fact"))
            .flag(&cover("first"))
            .flag(&cover("second"));
        assert_eq!(
            record.fact(),
            &crate::router::component::pack(&cover("fact"))
        );
        assert_eq!(
            record.flags(),
            &[
                crate::router::component::pack(&cover("first")),
                crate::router::component::pack(&cover("second")),
            ]
        );
        let routed = record.clone().into_fact_record();
        assert_eq!(&routed.fact, record.fact());
        assert_eq!(routed.flags, record.flags());
    }

    #[test]
    fn a_fact_message_records_itself_with_no_flags() {
        let routed = cover("fact").into_fact_record();
        assert_eq!(routed.fact, crate::router::component::pack(&cover("fact")));
        assert!(routed.flags.is_empty());
        assert!(FactRecord::new(&cover("fact")).flags().is_empty());
    }
}
