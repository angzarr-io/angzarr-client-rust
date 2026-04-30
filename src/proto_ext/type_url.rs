//! Type URL constants for protobuf Any messages.
//!
//! Constants for the angzarr-internal `type.angzarr.io/` URL scheme used by
//! framework messages (Notification / Revocation / Confirmation / etc.).

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
