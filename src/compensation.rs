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
//!     Ok(delegate_to_framework("No custom compensation", DelegationOptions::default()))
//! }
//! ```

use crate::convert::TYPE_URL_PREFIX;
use crate::error::ClientError;
use crate::error_codes::{codes, keys, messages};
use crate::proto::{
    business_response, command_page, page_header, BusinessResponse, CommandBook, Cover, EventBook,
    Notification, RejectionNotification, RevocationResponse,
};
use prost::Message;

/// Fully-qualified proto type name for Notification.
const NOTIFICATION_TYPE_NAME: &str = "angzarr_client.proto.angzarr.Notification";

/// Pre-computed full type URL for Notification — avoids per-call `format!`
/// in [`is_notification`]. Pinned via the `notification_type_url_matches_prefix_plus_name`
/// test below so any drift in `TYPE_URL_PREFIX` or `NOTIFICATION_TYPE_NAME`
/// fails compilation tests immediately.
const NOTIFICATION_TYPE_URL: &str = "type.googleapis.com/angzarr_client.proto.angzarr.Notification";

/// Parsed context from a rejection notification.
///
/// Provides easy access to rejection details extracted from the Notification
/// payload and the rejected command's deferred sequence header.
#[derive(Debug, Clone, PartialEq)]
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
    ///
    /// Returns an error — instead of a default-zero context — when:
    /// - the Notification has no payload (`MISSING_NOTIFICATION_PAYLOAD`),
    /// - the payload bytes don't decode as a RejectionNotification
    ///   (`REJECTION_NOTIFICATION_DECODE_FAILED`),
    /// - the rejected command is absent (`MISSING_REJECTED_COMMAND`),
    /// - the rejected command's first page is missing the
    ///   AngzarrDeferred sequence header (`MISSING_DEFERRED_HEADER`).
    ///
    /// Audit: previously this constructor silently swallowed every
    /// failure path and returned a default `source_event_sequence = 0`,
    /// which would compensate against a real, valid sequence 0.
    pub fn from_notification(notification: &Notification) -> Result<Self, ClientError> {
        let payload = notification.payload.as_ref().ok_or_else(|| {
            ClientError::invalid_argument(
                codes::MISSING_NOTIFICATION_PAYLOAD,
                messages::MISSING_NOTIFICATION_PAYLOAD,
                std::iter::empty::<(String, String)>(),
            )
        })?;

        let rejection = RejectionNotification::decode(payload.value.as_slice()).map_err(|e| {
            ClientError::invalid_argument(
                codes::REJECTION_NOTIFICATION_DECODE_FAILED,
                messages::REJECTION_NOTIFICATION_DECODE_FAILED,
                [(keys::CAUSE, e.to_string())],
            )
        })?;

        let cmd = rejection.rejected_command.ok_or_else(|| {
            ClientError::invalid_argument(
                codes::MISSING_REJECTED_COMMAND,
                messages::MISSING_REJECTED_COMMAND,
                std::iter::empty::<(String, String)>(),
            )
        })?;

        let deferred = cmd
            .pages
            .first()
            .and_then(|p| p.header.as_ref())
            .and_then(|h| match &h.sequence_type {
                Some(page_header::SequenceType::AngzarrDeferred(d)) => Some(d.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                ClientError::invalid_argument(
                    codes::MISSING_DEFERRED_HEADER,
                    messages::MISSING_DEFERRED_HEADER,
                    std::iter::empty::<(String, String)>(),
                )
            })?;

        Ok(CompensationContext {
            source_event_sequence: deferred.source_seq,
            rejection_reason: rejection.rejection_reason,
            rejected_command: Some(cmd),
            source_aggregate: deferred.source,
        })
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

    /// Returns the domain and command type suffix as a `"domain/CommandType"`
    /// key, or `None` when the rejected command, its cover, the domain, or
    /// the command type URL is missing.
    ///
    /// Mirrors Python's `CompensationContext.dispatch_key`
    /// (`f"{domain}/{type_name_from_url(cmd_type)}"`). Returning `Option`
    /// rather than an empty `String` prevents silently bucketing every
    /// malformed notification under the same `""` key in caller HashMaps.
    pub fn dispatch_key(&self) -> Option<String> {
        let domain = self
            .rejected_command
            .as_ref()
            .and_then(|cmd| cmd.cover.as_ref())
            .map(|c| c.domain.as_str())
            .filter(|d| !d.is_empty())?;

        let cmd_type = self.rejected_command_type();
        if cmd_type.is_empty() {
            return None;
        }
        let suffix = crate::convert::type_name_from_url(cmd_type);
        Some(format!("{}/{}", domain, suffix))
    }
}

// =============================================================================
// Aggregate helpers
// =============================================================================

/// Options struct for [`delegate_to_framework`] / [`pm_delegate_to_framework`].
///
/// Audit #65 / #66: collapses the previously-divergent function shapes
/// (this crate's old two-function split, Python's kwargs) into a single
/// shared options type. Cross-language symmetric — Python
/// `compensation.DelegationOptions` mirrors this struct field-for-field
/// with the same defaults.
///
/// `Default` matches the previous Python kwargs and the old Rust basic
/// `delegate_to_framework` (which hardcoded `emit_system_event = true`
/// and the rest false).
///
/// # Example
///
/// ```rust,ignore
/// use angzarr_client::compensation::{delegate_to_framework, DelegationOptions};
///
/// // Default — emit system event, nothing else.
/// let resp = delegate_to_framework("no custom compensation", DelegationOptions::default());
///
/// // Escalate without emitting a system event.
/// let resp = delegate_to_framework(
///     "operator intervention",
///     DelegationOptions { emit_system_event: false, escalate: true, ..Default::default() },
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegationOptions {
    /// Emit SagaCompensationFailed to the fallback domain.
    pub emit_system_event: bool,
    /// Move the failed event to the dead-letter queue.
    pub send_to_dead_letter: bool,
    /// Mark for operator intervention.
    pub escalate: bool,
    /// Stop the saga entirely without retry.
    pub abort: bool,
}

impl Default for DelegationOptions {
    fn default() -> Self {
        Self {
            emit_system_event: true,
            send_to_dead_letter: false,
            escalate: false,
            abort: false,
        }
    }
}

/// Create a response that delegates compensation to the framework.
///
/// Use when the aggregate doesn't have custom compensation logic for a
/// rejection. Pass [`DelegationOptions::default()`] for the standard
/// "emit system event" behavior, or override individual flags via a
/// struct literal — see [`DelegationOptions`].
///
/// Audit #65: replaces the previous two-function split
/// (`delegate_to_framework(reason)` + `delegate_to_framework_with_options(reason, ...)`)
/// with a single function taking the shared options struct, matching
/// Python's [`compensation::delegate_to_framework`].
pub fn delegate_to_framework(
    reason: impl Into<String>,
    options: DelegationOptions,
) -> BusinessResponse {
    BusinessResponse {
        result: Some(business_response::Result::Revocation(RevocationResponse {
            emit_system_revocation: options.emit_system_event,
            send_to_dead_letter_queue: options.send_to_dead_letter,
            escalate: options.escalate,
            abort: options.abort,
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
/// Use when the PM doesn't have custom compensation logic. Pass
/// [`DelegationOptions::default()`] for the standard behavior.
///
/// Audit #66: takes the same [`DelegationOptions`] struct as
/// [`delegate_to_framework`]. Previously hardcoded
/// `emit_system_revocation = true` with no way to override; now matches
/// Python's option-bearing signature.
pub fn pm_delegate_to_framework(
    reason: impl Into<String>,
    options: DelegationOptions,
) -> PMRevocationResponse {
    PMRevocationResponse {
        process_events: None,
        revocation: RevocationResponse {
            emit_system_revocation: options.emit_system_event,
            send_to_dead_letter_queue: options.send_to_dead_letter,
            escalate: options.escalate,
            abort: options.abort,
            reason: reason.into(),
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
///
/// Audit finding #58: matches against the fully qualified type name per
/// `google.protobuf.Any` spec. The previous short-form expectation
/// diverged from Python-emitted URLs.
pub fn is_notification(type_url: &str) -> bool {
    type_url == NOTIFICATION_TYPE_URL
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
                    sync_mode: None,
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
        let ctx = CompensationContext::from_notification(&notification).unwrap();
        assert_eq!(ctx.rejection_reason, "insufficient_funds");
    }

    #[test]
    fn from_notification_extracts_source_info() {
        let notification = make_rejection_notification(
            "out_of_stock",
            "inventory",
            "type.googleapis.com/examples.ReserveStock",
        );
        let ctx = CompensationContext::from_notification(&notification).unwrap();
        assert_eq!(ctx.source_event_sequence, 42);
        assert_eq!(
            ctx.source_aggregate.as_ref().unwrap().domain,
            "source-domain"
        );
    }

    #[test]
    fn from_notification_errors_on_missing_payload() {
        let notification = Notification::default();
        let err = CompensationContext::from_notification(&notification).unwrap_err();
        assert_eq!(err.code(), codes::MISSING_NOTIFICATION_PAYLOAD);
    }

    #[test]
    fn from_notification_errors_on_garbage_payload() {
        let notification = Notification {
            payload: Some(Any {
                type_url: format!("{}angzarr.RejectionNotification", TYPE_URL_PREFIX),
                value: vec![0xff, 0xff, 0xff, 0xff, 0xff],
            }),
            ..Default::default()
        };
        let err = CompensationContext::from_notification(&notification).unwrap_err();
        assert_eq!(err.code(), codes::REJECTION_NOTIFICATION_DECODE_FAILED);
    }

    #[test]
    fn from_notification_errors_when_rejected_command_missing() {
        let rejection = RejectionNotification {
            rejected_command: None,
            rejection_reason: "no command".into(),
        };
        let mut buf = Vec::new();
        rejection.encode(&mut buf).unwrap();
        let notification = Notification {
            payload: Some(Any {
                type_url: format!("{}angzarr.RejectionNotification", TYPE_URL_PREFIX),
                value: buf,
            }),
            ..Default::default()
        };
        let err = CompensationContext::from_notification(&notification).unwrap_err();
        assert_eq!(err.code(), codes::MISSING_REJECTED_COMMAND);
    }

    #[test]
    fn from_notification_errors_when_deferred_header_missing() {
        // Rejected command is present but its first page has no
        // AngzarrDeferred sequence header — previously silently produced
        // source_event_sequence=0.
        let rejected_command = CommandBook {
            cover: Some(Cover::default()),
            pages: vec![CommandPage {
                header: None,
                merge_strategy: 0,
                payload: None,
            }],
        };
        let rejection = RejectionNotification {
            rejected_command: Some(rejected_command),
            rejection_reason: "x".into(),
        };
        let mut buf = Vec::new();
        rejection.encode(&mut buf).unwrap();
        let notification = Notification {
            payload: Some(Any {
                type_url: format!("{}angzarr.RejectionNotification", TYPE_URL_PREFIX),
                value: buf,
            }),
            ..Default::default()
        };
        let err = CompensationContext::from_notification(&notification).unwrap_err();
        assert_eq!(err.code(), codes::MISSING_DEFERRED_HEADER);
    }

    #[test]
    fn rejected_command_type_returns_type_url() {
        let notification = make_rejection_notification(
            "fail",
            "orders",
            "type.googleapis.com/examples.CreateShipment",
        );
        let ctx = CompensationContext::from_notification(&notification).unwrap();
        assert_eq!(
            ctx.rejected_command_type(),
            "type.googleapis.com/examples.CreateShipment"
        );
    }

    #[test]
    fn dispatch_key_formats_domain_and_suffix() {
        let notification = make_rejection_notification(
            "fail",
            "fulfillment",
            "type.googleapis.com/examples.CreateShipment",
        );
        let ctx = CompensationContext::from_notification(&notification).unwrap();
        assert_eq!(
            ctx.dispatch_key().as_deref(),
            Some("fulfillment/examples.CreateShipment"),
        );
    }

    #[test]
    fn dispatch_key_none_when_domain_missing() {
        let notification =
            make_rejection_notification("fail", "", "type.googleapis.com/examples.CreateShipment");
        let ctx = CompensationContext::from_notification(&notification).unwrap();
        assert_eq!(ctx.dispatch_key(), None);
    }

    #[test]
    fn dispatch_key_none_when_cmd_type_missing() {
        let notification = make_rejection_notification("fail", "fulfillment", "");
        let ctx = CompensationContext::from_notification(&notification).unwrap();
        assert_eq!(ctx.dispatch_key(), None);
    }

    #[test]
    fn delegate_to_framework_default_options_sets_emit_system() {
        // Audit #65: DelegationOptions::default() = previous "basic"
        // delegate_to_framework behavior (emit_system=true, others false).
        let response = delegate_to_framework("test reason", DelegationOptions::default());
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
    fn delegate_to_framework_custom_options_sets_all_flags() {
        // Audit #65: pass an options struct literal. Replaces the
        // previous 5-positional-arg `delegate_to_framework_with_options`.
        let response = delegate_to_framework(
            "escalated",
            DelegationOptions {
                emit_system_event: true,
                send_to_dead_letter: true,
                escalate: true,
                abort: true,
            },
        );
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
        let response = pm_delegate_to_framework("pm reason", DelegationOptions::default());
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
        // Audit finding #58: spec-compliant fully qualified name.
        assert!(is_notification(
            "type.googleapis.com/angzarr_client.proto.angzarr.Notification"
        ));
    }

    #[test]
    fn notification_type_url_matches_prefix_plus_name() {
        // Pins the precomputed constant against the canonical prefix +
        // name so any drift in either source is caught here.
        assert_eq!(
            NOTIFICATION_TYPE_URL,
            format!("{}{}", TYPE_URL_PREFIX, NOTIFICATION_TYPE_NAME)
        );
    }

    #[test]
    fn is_notification_rejects_wrong_type_url() {
        assert!(!is_notification(
            "type.googleapis.com/angzarr_client.proto.angzarr.RejectionNotification"
        ));
        // Pre-#58 short form is no longer accepted.
        assert!(!is_notification("type.googleapis.com/angzarr.Notification"));
        assert!(!is_notification(
            "angzarr_client.proto.angzarr.Notification"
        ));
        assert!(!is_notification(""));
    }
}
