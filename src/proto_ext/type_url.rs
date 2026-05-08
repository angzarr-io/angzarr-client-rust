//! Type URL constants for protobuf Any messages.
//!
//! Constants for the angzarr-internal `type.angzarr.io/` URL scheme used by
//! framework messages (Notification / Revocation / Confirmation / etc.).
//!
//! Each constant is `concat!`-ed from the shared
//! [`ANGZARR_TYPE_URL_PREFIX`](super::constants::ANGZARR_TYPE_URL_PREFIX)
//! so a typo in the prefix shows up in every entry instead of silently
//! diverging across constants.

// Full type URLs for angzarr framework types
/// Type URL for Notification messages.
pub const NOTIFICATION: &str = concat!("type.angzarr.io/", "angzarr.Notification");
/// Type URL for RejectionNotification messages.
pub const REJECTION_NOTIFICATION: &str =
    concat!("type.angzarr.io/", "angzarr.RejectionNotification");
/// Type URL for SagaCompensationFailed messages.
pub const SAGA_COMPENSATION_FAILED: &str =
    concat!("type.angzarr.io/", "angzarr.SagaCompensationFailed");

// Two-phase commit framework events
/// Type URL for Confirmation messages (2PC commit).
pub const CONFIRMATION: &str = concat!("type.angzarr.io/", "angzarr.Confirmation");
/// Type URL for Revocation messages (2PC rollback).
pub const REVOCATION: &str = concat!("type.angzarr.io/", "angzarr.Revocation");
/// Type URL for Compensate messages (client-implemented rollback).
pub const COMPENSATE: &str = concat!("type.angzarr.io/", "angzarr.Compensate");
/// Type URL for NoOp messages (filtered event placeholder).
pub const NOOP: &str = concat!("type.angzarr.io/", "angzarr.NoOp");

#[cfg(test)]
mod tests {
    use super::super::constants::ANGZARR_TYPE_URL_PREFIX;
    use super::*;

    /// Pin every constant in this module against the shared
    /// `ANGZARR_TYPE_URL_PREFIX`. A typo in either source surfaces
    /// here rather than as a wire-format divergence in production.
    #[test]
    fn type_url_constants_share_prefix() {
        for (name, url) in [
            ("NOTIFICATION", NOTIFICATION),
            ("REJECTION_NOTIFICATION", REJECTION_NOTIFICATION),
            ("SAGA_COMPENSATION_FAILED", SAGA_COMPENSATION_FAILED),
            ("CONFIRMATION", CONFIRMATION),
            ("REVOCATION", REVOCATION),
            ("COMPENSATE", COMPENSATE),
            ("NOOP", NOOP),
        ] {
            assert!(
                url.starts_with(ANGZARR_TYPE_URL_PREFIX),
                "{} = {:?} should begin with {:?}",
                name,
                url,
                ANGZARR_TYPE_URL_PREFIX,
            );
        }
    }
}
