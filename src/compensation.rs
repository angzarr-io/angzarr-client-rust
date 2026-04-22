//! Compensation flow helpers for saga/PM revocation handling.
//!
//! When a saga/PM command is rejected by a target aggregate, the framework
//! sends a `Notification` with `RejectionNotification` payload to the
//! triggering aggregate. These helpers make it easy to implement compensation
//! logic from inside a `#[rejected(domain, command)]` method.
//!
//! # Example in aggregate
//!
//! ```rust,ignore
//! #[rejected(domain = "inventory", command = "ReserveStock")]
//! fn on_reserve_rejected(&self, notification: &Notification, state: &OrderState)
//!     -> CommandResult<BusinessResponse>
//! {
//!     let ctx = CompensationContext::from_notification(notification);
//!     // Emit compensation events or delegate to the framework via the helpers
//!     // re-exported from this module.
//!     Ok(delegate_to_framework("No custom compensation"))
//! }
//! ```

use crate::convert::{wire_name, TYPE_URL_PREFIX};
use crate::proto::{
    business_response, command_page, page_header, BusinessResponse, CommandBook, Cover, EventBook,
    Notification, RejectionNotification, RevocationResponse,
};
use prost::Message;

/// Fully-qualified proto type name for Notification.
const NOTIFICATION_TYPE_NAME: &str = "angzarr_client.proto.angzarr.Notification";

/// Parsed context from a rejection notification.
///
/// Provides easy access to rejection details extracted from the Notification
/// payload and the rejected command's deferred sequence header.
pub struct CompensationContext {
    /// Sequence of the event that triggered the saga/PM command.
    pub source_event_sequence: u32,

    /// Why the command was rejected (e.g. "insufficient_funds").
    pub rejection_reason: String,

    /// The command that was rejected (full context).
    pub rejected_command: Option<CommandBook>,

    /// Cover of the aggregate that triggered the saga/PM flow.
    pub source_aggregate: Option<Cover>,
}

impl CompensationContext {
    /// Extract compensation context from a Notification.
    ///
    /// Decodes the RejectionNotification from the notification payload, then
    /// pulls source info from `rejected_command.pages[0].header.angzarr_deferred`.
    pub fn from_notification(notification: &Notification) -> Self {
        let mut ctx = CompensationContext {
            source_event_sequence: 0,
            rejection_reason: String::new(),
            rejected_command: None,
            source_aggregate: None,
        };

        if let Some(payload) = &notification.payload {
            if let Ok(rejection) = RejectionNotification::decode(payload.value.as_slice()) {
                ctx.rejection_reason = rejection.rejection_reason;
                ctx.rejected_command = rejection.rejected_command.clone();

                // Extract source info from rejected_command.pages[0].header.angzarr_deferred
                if let Some(ref cmd) = rejection.rejected_command {
                    if let Some(page) = cmd.pages.first() {
                        if let Some(ref header) = page.header {
                            if let Some(page_header::SequenceType::AngzarrDeferred(ref deferred)) =
                                header.sequence_type
                            {
                                ctx.source_aggregate = deferred.source.clone();
                                ctx.source_event_sequence = deferred.source_seq;
                            }
                        }
                    }
                }
            }
        }

        ctx
    }

    /// Returns the type URL of the rejected command, if available.
    ///
    /// Extracts from `rejected_command.pages[0].command.type_url`.
    pub fn rejected_command_type(&self) -> &str {
        self.rejected_command
            .as_ref()
            .and_then(|cmd| cmd.pages.first())
            .and_then(|page| match &page.payload {
                Some(command_page::Payload::Command(c)) => Some(c.type_url.as_str()),
                _ => None,
            })
            .unwrap_or("")
    }

    /// Returns the domain and command type suffix as a "domain:CommandType" key.
    ///
    /// Useful for routing rejection handling by target domain and command type.
    pub fn dispatch_key(&self) -> String {
        let domain = self
            .rejected_command
            .as_ref()
            .and_then(|cmd| cmd.cover.as_ref())
            .map(|c| c.domain.as_str())
            .unwrap_or("");

        let cmd_type = self.rejected_command_type();
        let suffix = cmd_type.rsplit('/').next().unwrap_or(cmd_type);

        format!("{}:{}", domain, suffix)
    }
}

// =============================================================================
// Aggregate helpers
// =============================================================================

/// Create a response that delegates compensation to the framework.
///
/// The framework will emit a SagaCompensationFailed event. Use when the
/// aggregate doesn't have custom compensation logic for a rejection.
pub fn delegate_to_framework(reason: impl Into<String>) -> BusinessResponse {
    BusinessResponse {
        result: Some(business_response::Result::Revocation(RevocationResponse {
            emit_system_revocation: true,
            reason: reason.into(),
            ..Default::default()
        })),
    }
}

/// Create a response with custom revocation flags.
///
/// Provides fine-grained control over framework compensation behavior:
/// - `emit_system_event`: emit SagaCompensationFailed event
/// - `send_to_dlq`: send to dead letter queue
/// - `escalate`: flag for alerting/human intervention
/// - `abort`: stop saga chain, propagate error to caller
pub fn delegate_to_framework_with_options(
    reason: impl Into<String>,
    emit_system_event: bool,
    send_to_dlq: bool,
    escalate: bool,
    abort: bool,
) -> BusinessResponse {
    BusinessResponse {
        result: Some(business_response::Result::Revocation(RevocationResponse {
            emit_system_revocation: emit_system_event,
            send_to_dead_letter_queue: send_to_dlq,
            escalate,
            abort,
            reason: reason.into(),
        })),
    }
}

/// Create a response containing compensation events.
///
/// The framework will persist these events. No system event is emitted.
pub fn emit_compensation_events(events: EventBook) -> BusinessResponse {
    BusinessResponse {
        result: Some(business_response::Result::Events(events)),
    }
}

// =============================================================================
// Process Manager helpers
// =============================================================================

/// PM compensation result containing optional process events and revocation flags.
pub struct PMRevocationResponse {
    /// PM events to persist (compensation state tracking).
    pub process_events: Option<EventBook>,

    /// Framework action flags.
    pub revocation: RevocationResponse,
}

/// Create a PM response that delegates compensation to the framework.
///
/// Use when the PM doesn't have custom compensation logic.
pub fn pm_delegate_to_framework(reason: impl Into<String>) -> PMRevocationResponse {
    PMRevocationResponse {
        process_events: None,
        revocation: RevocationResponse {
            emit_system_revocation: true,
            reason: reason.into(),
            ..Default::default()
        },
    }
}

/// Create a PM response with compensation events and revocation flags.
///
/// Use when the PM emits events to record the failure in its own state.
pub fn pm_emit_compensation_events(
    events: EventBook,
    also_emit_system_event: bool,
    reason: impl Into<String>,
) -> PMRevocationResponse {
    PMRevocationResponse {
        process_events: Some(events),
        revocation: RevocationResponse {
            emit_system_revocation: also_emit_system_event,
            reason: reason.into(),
            ..Default::default()
        },
    }
}

// =============================================================================
// Helper functions
// =============================================================================

/// Check if a type URL refers to a rejection Notification.
pub fn is_notification(type_url: &str) -> bool {
    type_url == format!("{}{}", TYPE_URL_PREFIX, wire_name(NOTIFICATION_TYPE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{AngzarrDeferredSequence, CommandPage, PageHeader, Uuid as ProtoUuid};
    use prost::Message;
    use prost_types::Any;

    fn make_rejection_notification(reason: &str, domain: &str, cmd_type_url: &str) -> Notification {
        let deferred = AngzarrDeferredSequence {
            source: Some(Cover {
                domain: "source-domain".to_string(),
                root: Some(ProtoUuid {
                    value: b"test-root-id".to_vec(),
                }),
                ..Default::default()
            }),
            source_seq: 42,
        };

        let rejected_command = CommandBook {
            cover: Some(Cover {
                domain: domain.to_string(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                header: Some(PageHeader {
                    sequence_type: Some(page_header::SequenceType::AngzarrDeferred(deferred)),
                }),
                merge_strategy: 0,
                payload: Some(command_page::Payload::Command(Any {
                    type_url: cmd_type_url.to_string(),
                    value: vec![],
                })),
            }],
        };

        let rejection = RejectionNotification {
            rejected_command: Some(rejected_command),
            rejection_reason: reason.to_string(),
        };

        let mut buf = Vec::new();
        rejection.encode(&mut buf).unwrap();

        Notification {
            payload: Some(Any {
                type_url: format!("{}angzarr.RejectionNotification", TYPE_URL_PREFIX),
                value: buf,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn from_notification_extracts_rejection_reason() {
        let notification = make_rejection_notification(
            "insufficient_funds",
            "payments",
            "type.googleapis.com/examples.ChargeCard",
        );
        let ctx = CompensationContext::from_notification(&notification);
        assert_eq!(ctx.rejection_reason, "insufficient_funds");
    }

    #[test]
    fn from_notification_extracts_source_info() {
        let notification = make_rejection_notification(
            "out_of_stock",
            "inventory",
            "type.googleapis.com/examples.ReserveStock",
        );
        let ctx = CompensationContext::from_notification(&notification);
        assert_eq!(ctx.source_event_sequence, 42);
        assert_eq!(
            ctx.source_aggregate.as_ref().unwrap().domain,
            "source-domain"
        );
    }

    #[test]
    fn from_notification_handles_missing_payload() {
        let notification = Notification::default();
        let ctx = CompensationContext::from_notification(&notification);
        assert_eq!(ctx.rejection_reason, "");
        assert_eq!(ctx.source_event_sequence, 0);
        assert!(ctx.rejected_command.is_none());
        assert!(ctx.source_aggregate.is_none());
    }

    #[test]
    fn rejected_command_type_returns_type_url() {
        let notification = make_rejection_notification(
            "fail",
            "orders",
            "type.googleapis.com/examples.CreateShipment",
        );
        let ctx = CompensationContext::from_notification(&notification);
        assert_eq!(
            ctx.rejected_command_type(),
            "type.googleapis.com/examples.CreateShipment"
        );
    }

    #[test]
    fn rejected_command_type_returns_empty_when_missing() {
        let ctx = CompensationContext::from_notification(&Notification::default());
        assert_eq!(ctx.rejected_command_type(), "");
    }

    #[test]
    fn dispatch_key_formats_domain_and_suffix() {
        let notification = make_rejection_notification(
            "fail",
            "fulfillment",
            "type.googleapis.com/examples.CreateShipment",
        );
        let ctx = CompensationContext::from_notification(&notification);
        assert_eq!(ctx.dispatch_key(), "fulfillment:examples.CreateShipment");
    }

    #[test]
    fn delegate_to_framework_sets_revocation_flags() {
        let response = delegate_to_framework("test reason");
        match response.result {
            Some(business_response::Result::Revocation(r)) => {
                assert!(r.emit_system_revocation);
                assert_eq!(r.reason, "test reason");
                assert!(!r.send_to_dead_letter_queue);
                assert!(!r.escalate);
                assert!(!r.abort);
            }
            _ => panic!("Expected Revocation variant"),
        }
    }

    #[test]
    fn delegate_to_framework_with_options_sets_all_flags() {
        let response = delegate_to_framework_with_options("escalated", true, true, true, true);
        match response.result {
            Some(business_response::Result::Revocation(r)) => {
                assert!(r.emit_system_revocation);
                assert!(r.send_to_dead_letter_queue);
                assert!(r.escalate);
                assert!(r.abort);
                assert_eq!(r.reason, "escalated");
            }
            _ => panic!("Expected Revocation variant"),
        }
    }

    #[test]
    fn emit_compensation_events_wraps_event_book() {
        let events = EventBook {
            cover: Some(Cover {
                domain: "orders".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let response = emit_compensation_events(events);
        match response.result {
            Some(business_response::Result::Events(book)) => {
                assert_eq!(book.cover.unwrap().domain, "orders");
            }
            _ => panic!("Expected Events variant"),
        }
    }

    #[test]
    fn pm_delegate_to_framework_returns_nil_events() {
        let response = pm_delegate_to_framework("pm reason");
        assert!(response.process_events.is_none());
        assert!(response.revocation.emit_system_revocation);
        assert_eq!(response.revocation.reason, "pm reason");
    }

    #[test]
    fn pm_emit_compensation_events_includes_both() {
        let events = EventBook::default();
        let response = pm_emit_compensation_events(events, true, "pm compensation");
        assert!(response.process_events.is_some());
        assert!(response.revocation.emit_system_revocation);
        assert_eq!(response.revocation.reason, "pm compensation");
    }

    #[test]
    fn is_notification_matches_correct_type_url() {
        assert!(is_notification("type.googleapis.com/angzarr.Notification"));
    }

    #[test]
    fn is_notification_rejects_wrong_type_url() {
        assert!(!is_notification(
            "type.googleapis.com/angzarr.RejectionNotification"
        ));
        assert!(!is_notification("angzarr_client.proto.angzarr.Notification"));
        assert!(!is_notification(""));
    }
}
