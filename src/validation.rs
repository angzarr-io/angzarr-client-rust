//! Validation helpers for command handler precondition checks.
//!
//! Eliminates repeated validation boilerplate across aggregate handlers.
//!
//! # Example
//!
//! ```rust,ignore
//! use angzarr_client::validation::{require_exists, require_positive};
//!
//! fn handle_deposit(state: &PlayerState, amount: i64) -> CommandResult<EventBook> {
//!     require_exists(state.exists(), "Player does not exist")?;
//!     require_positive(amount, "amount")?;
//!     // ... rest of handler
//! }
//! ```

use crate::error_codes::{codes, keys, messages};
use crate::CommandRejectedError;

/// Emit a structured `info`-level log for a validation rejection.
///
/// Audit #30 + #59: structured fields (`field`, `predicate`, `status_code`)
/// ride alongside the static `message` so observability layers can
/// aggregate validation rejections without parsing the message string.
fn log_rejection(field: &str, predicate: &str, status_code: &str, message: &str) {
    tracing::info!(
        field = field,
        predicate = predicate,
        status_code = status_code,
        message = message,
        "validation rejection",
    );
}

/// Require that an aggregate exists (has prior events).
///
/// Returns a `NOT_FOUND` rejection — not retryable, since refetching events
/// cannot change the outcome. Mirrors Python's `require_exists`.
/// Audit #59: `code = "ENTITY_NOT_FOUND"`, static message `"entity not found"`,
/// caller-provided context goes through ``details["context"]``.
pub fn require_exists(exists: bool, context: &str) -> Result<(), CommandRejectedError> {
    if !exists {
        log_rejection("<entity>", "exists", "NOT_FOUND", "entity not found");
        return Err(CommandRejectedError::not_found(
            codes::ENTITY_NOT_FOUND,
            messages::ENTITY_NOT_FOUND,
            [(keys::CONTEXT, context)],
        ));
    }
    Ok(())
}

/// Require that an aggregate does not exist.
pub fn require_not_exists(exists: bool, context: &str) -> Result<(), CommandRejectedError> {
    if exists {
        log_rejection("<entity>", "not_exists", "FAILED_PRECONDITION", "entity already exists");
        return Err(CommandRejectedError::precondition_failed(
            codes::ENTITY_ALREADY_EXISTS,
            "entity already exists",
            [("context", context)],
        ));
    }
    Ok(())
}

/// Require that a value is positive (greater than zero).
pub fn require_positive<T: PartialOrd + Default>(
    value: T,
    field_name: &str,
) -> Result<(), CommandRejectedError> {
    if value <= T::default() {
        log_rejection(field_name, "positive", "INVALID_ARGUMENT", "value must be positive");
        return Err(CommandRejectedError::invalid_argument(
            codes::VALUE_NOT_POSITIVE,
            "value must be positive",
            [("field", field_name)],
        ));
    }
    Ok(())
}

/// Require that a value is non-negative (zero or greater).
pub fn require_non_negative<T: PartialOrd + Default>(
    value: T,
    field_name: &str,
) -> Result<(), CommandRejectedError> {
    if value < T::default() {
        log_rejection(field_name, "non_negative", "INVALID_ARGUMENT", "value must be non-negative");
        return Err(CommandRejectedError::invalid_argument(
            codes::VALUE_NOT_NON_NEGATIVE,
            "value must be non-negative",
            [("field", field_name)],
        ));
    }
    Ok(())
}

/// Require that a string is not empty.
pub fn require_not_empty_str(value: &str, field_name: &str) -> Result<(), CommandRejectedError> {
    if value.is_empty() {
        log_rejection(field_name, "not_empty_str", "INVALID_ARGUMENT", "value must not be empty");
        return Err(CommandRejectedError::invalid_argument(
            codes::VALUE_EMPTY,
            "value must not be empty",
            [("field", field_name)],
        ));
    }
    Ok(())
}

/// Require that a collection is not empty.
pub fn require_not_empty<T>(items: &[T], field_name: &str) -> Result<(), CommandRejectedError> {
    if items.is_empty() {
        log_rejection(field_name, "not_empty", "INVALID_ARGUMENT", "collection must not be empty");
        return Err(CommandRejectedError::invalid_argument(
            codes::COLLECTION_EMPTY,
            "collection must not be empty",
            [("field", field_name)],
        ));
    }
    Ok(())
}

/// Require that a status matches an expected value.
pub fn require_status<T: PartialEq>(
    actual: T,
    expected: T,
    context: &str,
) -> Result<(), CommandRejectedError> {
    if actual != expected {
        log_rejection("status", "status_eq", "FAILED_PRECONDITION", "status does not match expected");
        return Err(CommandRejectedError::precondition_failed(
            codes::STATUS_MISMATCH,
            "status does not match expected",
            [("context", context)],
        ));
    }
    Ok(())
}

/// Require that a status does not match a forbidden value.
pub fn require_status_not<T: PartialEq>(
    actual: T,
    forbidden: T,
    context: &str,
) -> Result<(), CommandRejectedError> {
    if actual == forbidden {
        log_rejection("status", "status_not", "FAILED_PRECONDITION", "status is the forbidden value");
        return Err(CommandRejectedError::precondition_failed(
            codes::STATUS_FORBIDDEN,
            "status is the forbidden value",
            [("context", context)],
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_exists_passes() {
        assert!(require_exists(true, "should exist").is_ok());
    }

    #[test]
    fn test_require_exists_fails() {
        let err = require_exists(false, "Player does not exist").expect_err("must error");
        assert_eq!(err.code, codes::ENTITY_NOT_FOUND);
        assert_eq!(err.message, "entity not found");
        assert_eq!(err.details["context"], "Player does not exist");
        assert!(err.is_not_found());
    }

    #[test]
    fn test_require_not_exists_passes() {
        assert!(require_not_exists(false, "should not exist").is_ok());
    }

    #[test]
    fn test_require_not_exists_fails() {
        let err = require_not_exists(true, "Player already exists").expect_err("must error");
        assert_eq!(err.code, codes::ENTITY_ALREADY_EXISTS);
        assert_eq!(err.message, "entity already exists");
        assert_eq!(err.details["context"], "Player already exists");
    }

    #[test]
    fn test_require_positive_passes() {
        assert!(require_positive(1i64, "amount").is_ok());
        assert!(require_positive(100i32, "value").is_ok());
    }

    #[test]
    fn test_require_positive_fails_zero() {
        let err = require_positive(0i64, "amount").expect_err("must error");
        assert_eq!(err.code, codes::VALUE_NOT_POSITIVE);
        assert_eq!(err.message, "value must be positive");
        assert_eq!(err.details["field"], "amount");
    }

    #[test]
    fn test_require_positive_fails_negative() {
        let err = require_positive(-5i64, "amount").expect_err("must error");
        assert_eq!(err.code, codes::VALUE_NOT_POSITIVE);
    }

    #[test]
    fn test_require_non_negative_passes() {
        assert!(require_non_negative(0i64, "balance").is_ok());
        assert!(require_non_negative(100i64, "balance").is_ok());
    }

    #[test]
    fn test_require_non_negative_fails() {
        let err = require_non_negative(-1i64, "balance").expect_err("must error");
        assert_eq!(err.code, codes::VALUE_NOT_NON_NEGATIVE);
        assert_eq!(err.message, "value must be non-negative");
        assert_eq!(err.details["field"], "balance");
    }

    #[test]
    fn test_require_not_empty_str_passes() {
        assert!(require_not_empty_str("hello", "name").is_ok());
    }

    #[test]
    fn test_require_not_empty_str_fails() {
        let err = require_not_empty_str("", "name").expect_err("must error");
        assert_eq!(err.code, codes::VALUE_EMPTY);
        assert_eq!(err.message, "value must not be empty");
        assert_eq!(err.details["field"], "name");
    }

    #[test]
    fn test_require_not_empty_passes() {
        assert!(require_not_empty(&[1, 2, 3], "items").is_ok());
    }

    #[test]
    fn test_require_not_empty_fails() {
        let empty: Vec<i32> = vec![];
        let err = require_not_empty(&empty, "items").expect_err("must error");
        assert_eq!(err.code, codes::COLLECTION_EMPTY);
        assert_eq!(err.message, "collection must not be empty");
        assert_eq!(err.details["field"], "items");
    }

    #[test]
    fn test_require_status_passes() {
        assert!(require_status("active", "active", "must be active").is_ok());
    }

    #[test]
    fn test_require_status_fails() {
        let err = require_status("pending", "active", "must be active").expect_err("must error");
        assert_eq!(err.code, codes::STATUS_MISMATCH);
        assert_eq!(err.message, "status does not match expected");
    }

    #[test]
    fn test_require_status_not_passes() {
        assert!(require_status_not("active", "deleted", "cannot be deleted").is_ok());
    }

    #[test]
    fn test_require_status_not_fails() {
        let err = require_status_not("deleted", "deleted", "cannot be deleted")
            .expect_err("must error");
        assert_eq!(err.code, codes::STATUS_FORBIDDEN);
        assert_eq!(err.message, "status is the forbidden value");
    }
}
