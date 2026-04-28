//! Error types and result aliases for the client.
//!
//! Audit finding #59 (structural error model). Errors carry:
//!
//! - A static `message: &'static str` — same exact string for the same
//!   predicate failure across all call sites. Suitable for log
//!   greppability and cross-language equality with the Python client.
//! - A stable `code: &'static str` (`SCREAMING_SNAKE`) — programmatic
//!   dispatch and cucumber assertions key off this.
//! - Structured `details: BTreeMap<String, String>` — runtime context
//!   (field name, type URL, domain, etc.) that varies per call site.
//!
//! Callers MUST NOT interpolate runtime values into `message`. Put them
//! in `details`.

use std::collections::BTreeMap;

use tonic::{Code, Status};

/// Result type for client operations.
pub type Result<T> = std::result::Result<T, ClientError>;

/// Structured error detail — the common shape carried by every
/// dispatch/validation/conversion error variant.
///
/// `message` is a static string (no runtime interpolation). Anything
/// dynamic — failed field name, offending type URL, originating domain
/// — rides in `details`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorDetail {
    pub code: &'static str,
    pub message: &'static str,
    pub details: BTreeMap<String, String>,
}

impl ErrorDetail {
    /// Build an `ErrorDetail` from a code, static message, and an iterable
    /// of `(key, value)` pairs for the structured details.
    pub fn new<I, K, V>(code: &'static str, message: &'static str, details: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            code,
            message,
            details: details
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

/// Errors that can occur during client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Failed to establish connection to the server.
    #[error("{}", .0.message)]
    Connection(ErrorDetail),

    /// Transport-level error from tonic.
    #[error("{}", _0)]
    Transport(#[from] tonic::transport::Error),

    /// gRPC error from the server.
    #[error("{}", .0.message())]
    Grpc(Box<Status>),

    /// Invalid argument provided by caller.
    #[error("{}", .0.message)]
    InvalidArgument(ErrorDetail),

    /// Failed to parse timestamp.
    #[error("{}", .0.message)]
    InvalidTimestamp(ErrorDetail),

    /// Business-rule rejection raised by a command handler.
    ///
    /// Wraps [`CommandRejectedError`] so the same status-class predicates
    /// (`is_not_found`, `is_precondition_failed`, `is_invalid_argument`)
    /// answer correctly whether the failure was a gRPC status from the
    /// server or a local rejection.
    #[error("{}", .0.message)]
    Rejected(CommandRejectedError),
}

impl From<CommandRejectedError> for ClientError {
    fn from(err: CommandRejectedError) -> Self {
        ClientError::Rejected(err)
    }
}

impl From<Status> for ClientError {
    fn from(status: Status) -> Self {
        ClientError::Grpc(Box::new(status))
    }
}

impl ClientError {
    /// Build an `InvalidArgument` variant with structured details.
    ///
    /// `code` is the SCREAMING_SNAKE stable identifier; `message` is the
    /// static human-readable string. Runtime context goes in `details`.
    pub fn invalid_argument<I, K, V>(
        code: &'static str,
        message: &'static str,
        details: I,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self::InvalidArgument(ErrorDetail::new(code, message, details))
    }

    /// Build an `InvalidTimestamp` variant with structured details.
    pub fn invalid_timestamp<I, K, V>(
        code: &'static str,
        message: &'static str,
        details: I,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self::InvalidTimestamp(ErrorDetail::new(code, message, details))
    }

    /// Build a `Connection` variant with structured details.
    pub fn connection<I, K, V>(
        code: &'static str,
        message: &'static str,
        details: I,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self::Connection(ErrorDetail::new(code, message, details))
    }

    /// Returns the (static) error message.
    pub fn message(&self) -> String {
        match self {
            ClientError::Connection(d) => d.message.to_string(),
            ClientError::Transport(e) => e.to_string(),
            ClientError::Grpc(s) => s.message().to_string(),
            ClientError::InvalidArgument(d) => d.message.to_string(),
            ClientError::InvalidTimestamp(d) => d.message.to_string(),
            ClientError::Rejected(r) => r.message.to_string(),
        }
    }

    /// Returns the SCREAMING_SNAKE error code, or `""` for the foreign-error
    /// variants (`Transport` / `Grpc`) which carry their own classification.
    pub fn code(&self) -> &'static str {
        match self {
            ClientError::Connection(d) => d.code,
            ClientError::Transport(_) => "TRANSPORT_ERROR",
            ClientError::Grpc(_) => "",
            ClientError::InvalidArgument(d) => d.code,
            ClientError::InvalidTimestamp(d) => d.code,
            ClientError::Rejected(r) => r.code,
        }
    }

    /// Returns the gRPC status code if this is a gRPC error.
    pub fn grpc_code(&self) -> Option<Code> {
        match self {
            ClientError::Grpc(s) => Some(s.code()),
            _ => None,
        }
    }

    /// Returns the underlying gRPC Status if this is a gRPC error.
    pub fn status(&self) -> Option<&Status> {
        match self {
            ClientError::Grpc(s) => Some(s),
            _ => None,
        }
    }

    /// Returns true if this is a "not found" error.
    pub fn is_not_found(&self) -> bool {
        matches!(self.grpc_code(), Some(Code::NotFound))
            || matches!(self, ClientError::Rejected(r) if r.is_not_found())
    }

    /// Returns true if this is a "precondition failed" error.
    pub fn is_precondition_failed(&self) -> bool {
        matches!(self.grpc_code(), Some(Code::FailedPrecondition))
            || matches!(self, ClientError::Rejected(r) if r.is_precondition_failed())
    }

    /// Returns true if this is an "invalid argument" error.
    pub fn is_invalid_argument(&self) -> bool {
        matches!(self.grpc_code(), Some(Code::InvalidArgument))
            || matches!(self, ClientError::InvalidArgument(_))
            || matches!(self, ClientError::Rejected(r) if r.is_invalid_argument())
    }

    /// Returns true for connection/transport-class errors — including a gRPC
    /// `UNAVAILABLE` status.
    pub fn is_connection_error(&self) -> bool {
        match self {
            ClientError::Connection(_) | ClientError::Transport(_) => true,
            ClientError::Grpc(s) => s.code() == Code::Unavailable,
            _ => false,
        }
    }
}

/// Business-rule rejection raised by command handlers.
///
/// # Status codes and retry semantics
///
/// The framework's retry policy keys off `status_code`:
/// - `"FAILED_PRECONDITION"`: state-based rejection. Retryable after refreshing state.
/// - `"INVALID_ARGUMENT"`: bad input. Not retryable.
/// - `"NOT_FOUND"`: aggregate does not exist. Not retryable — refetching won't help.
///
/// Audit finding #59 fields:
///   - `message: &'static str` — static human-readable string.
///   - `code: &'static str` — SCREAMING_SNAKE stable identifier.
///   - `status_code: &'static str` — `FAILED_PRECONDITION` / `INVALID_ARGUMENT` / `NOT_FOUND`.
///   - `details: BTreeMap<String, String>` — runtime context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRejectedError {
    pub code: &'static str,
    pub message: &'static str,
    pub status_code: &'static str,
    pub details: BTreeMap<String, String>,
}

impl CommandRejectedError {
    /// Create a FAILED_PRECONDITION rejection.
    pub fn precondition_failed<I, K, V>(
        code: &'static str,
        message: &'static str,
        details: I,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            code,
            message,
            status_code: "FAILED_PRECONDITION",
            details: details
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Create an INVALID_ARGUMENT rejection for input validation failures.
    pub fn invalid_argument<I, K, V>(
        code: &'static str,
        message: &'static str,
        details: I,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            code,
            message,
            status_code: "INVALID_ARGUMENT",
            details: details
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Create a NOT_FOUND rejection for missing-aggregate failures.
    ///
    /// Not retryable — refetching events cannot change the outcome.
    pub fn not_found<I, K, V>(
        code: &'static str,
        message: &'static str,
        details: I,
    ) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            code,
            message,
            status_code: "NOT_FOUND",
            details: details
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    pub fn is_precondition_failed(&self) -> bool {
        self.status_code == "FAILED_PRECONDITION"
    }

    pub fn is_invalid_argument(&self) -> bool {
        self.status_code == "INVALID_ARGUMENT"
    }

    pub fn is_not_found(&self) -> bool {
        self.status_code == "NOT_FOUND"
    }
}

impl std::fmt::Display for CommandRejectedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Static message only — no prefix, no concatenation. Audit #59
        // also resolves #38 (Display-format trivia).
        f.write_str(self.message)
    }
}

impl std::error::Error for CommandRejectedError {}

impl From<CommandRejectedError> for Status {
    fn from(err: CommandRejectedError) -> Self {
        match err.status_code {
            "INVALID_ARGUMENT" => Status::invalid_argument(err.message),
            "NOT_FOUND" => Status::not_found(err.message),
            _ => Status::failed_precondition(err.message),
        }
    }
}

/// Result type for command/event handlers.
pub type CommandResult<T> = std::result::Result<T, CommandRejectedError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_static_message_and_code() {
        let err = CommandRejectedError::invalid_argument(
            "VALUE_NOT_POSITIVE",
            "value must be positive",
            [("field", "amount")],
        );
        assert_eq!(err.message, "value must be positive");
        assert_eq!(err.code, "VALUE_NOT_POSITIVE");
        assert_eq!(err.status_code, "INVALID_ARGUMENT");
        assert_eq!(err.details["field"], "amount");
        assert!(err.is_invalid_argument());
        assert_eq!(err.to_string(), "value must be positive");
    }

    #[test]
    fn is_connection_error_includes_unavailable_grpc() {
        let err = ClientError::from(Status::unavailable("backend down"));
        assert!(err.is_connection_error());
    }

    #[test]
    fn is_connection_error_excludes_other_grpc_codes() {
        for status in [
            Status::not_found("missing"),
            Status::failed_precondition("conflict"),
            Status::invalid_argument("bad"),
            Status::internal("oops"),
        ] {
            let err = ClientError::from(status);
            assert!(!err.is_connection_error());
        }
    }

    #[test]
    fn rejected_into_client_error_routes_predicates() {
        let rej = CommandRejectedError::not_found(
            "ENTITY_NOT_FOUND",
            "entity does not exist",
            std::iter::empty::<(String, String)>(),
        );
        let ce: ClientError = rej.into();
        assert!(ce.is_not_found());
        assert!(!ce.is_precondition_failed());
        assert!(!ce.is_invalid_argument());
        assert!(!ce.is_connection_error());
    }

    #[test]
    fn invalid_argument_carries_structured_details() {
        let err = ClientError::invalid_argument(
            "SAGA_INVALID_TYPE_URL",
            "saga trigger has invalid type_url",
            [("type_url", "type.example.com/foo")],
        );
        assert_eq!(err.code(), "SAGA_INVALID_TYPE_URL");
        assert_eq!(err.message(), "saga trigger has invalid type_url");
        if let ClientError::InvalidArgument(detail) = &err {
            assert_eq!(detail.details["type_url"], "type.example.com/foo");
        } else {
            panic!("expected InvalidArgument variant");
        }
    }

    #[test]
    fn rejected_message_is_static_no_prefix() {
        // Audit #38 (subsumed by #59): Display emits the static message
        // only — no "Command rejected: " prefix.
        let err = CommandRejectedError::precondition_failed(
            "ALREADY_OPEN",
            "registration already open",
            std::iter::empty::<(String, String)>(),
        );
        assert_eq!(err.to_string(), "registration already open");
    }
}
