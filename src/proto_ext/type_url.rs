//! Type URL constants for protobuf Any messages.
//!
//! Provides constants and helpers for working with protobuf type URLs
//! in the angzarr framework.

/// Type URL prefix for angzarr types.
pub const PREFIX: &str = "type.angzarr.io/";

/// Suffix for notification types (used for routing).
pub const NOTIFICATION_SUFFIX: &str = "Notification";

/// Suffix for CloudEvents response types.
pub const CLOUD_EVENTS_RESPONSE_SUFFIX: &str = "CloudEventsResponse";

// Full type URLs for angzarr framework types
/// Type URL for Notification messages.
pub const NOTIFICATION: &str = "type.angzarr.io/angzarr.Notification";
/// Type URL for RejectionNotification messages.
pub const REJECTION_NOTIFICATION: &str = "type.angzarr.io/angzarr.RejectionNotification";
/// Type URL for SagaCompensationFailed messages.
pub const SAGA_COMPENSATION_FAILED: &str = "type.angzarr.io/angzarr.SagaCompensationFailed";

// Two-phase commit framework events
/// Type URL for Confirmation messages (2PC commit).
pub const CONFIRMATION: &str = "type.angzarr.io/angzarr.Confirmation";
/// Type URL for Revocation messages (2PC rollback).
pub const REVOCATION: &str = "type.angzarr.io/angzarr.Revocation";
/// Type URL for Compensate messages (client-implemented rollback).
pub const COMPENSATE: &str = "type.angzarr.io/angzarr.Compensate";
/// Type URL for NoOp messages (filtered event placeholder).
pub const NOOP: &str = "type.angzarr.io/angzarr.NoOp";

/// Build a type URL for a message type.
///
/// The Rust-only `angzarr_client.proto.` prefix is stripped so wire URLs
/// use the cross-language short name.
///
/// # Example
/// ```
/// use angzarr::proto_ext::type_url;
/// assert_eq!(type_url::for_type("angzarr_client.proto.examples.OrderCreated"), "type.angzarr.io/examples.OrderCreated");
/// ```
pub fn for_type(message_type: &str) -> String {
    format!("{}{}", PREFIX, crate::convert::wire_name(message_type))
}

/// Strip the type URL prefix and return the Rust-internal full name
/// (re-adding the `angzarr_client.proto.` package prefix).
///
/// # Example
/// ```
/// use angzarr::proto_ext::type_url;
/// assert_eq!(type_url::strip_prefix("type.angzarr.io/examples.OrderCreated"), "angzarr_client.proto.examples.OrderCreated");
/// ```
pub fn strip_prefix(type_url: &str) -> String {
    match type_url.strip_prefix(PREFIX) {
        None => type_url.to_string(),
        Some(wire) if wire.starts_with(crate::convert::INTERNAL_PACKAGE_PREFIX) => wire.to_string(),
        Some(wire) => format!("{}{}", crate::convert::INTERNAL_PACKAGE_PREFIX, wire),
    }
}

#[cfg(test)]
#[path = "type_url.test.rs"]
mod tests;
