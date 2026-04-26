//! Error types and result aliases for the client.

use tonic::{Code, Status};

/// Result type for client operations.
pub type Result<T> = std::result::Result<T, ClientError>;

/// Error message constants for client operations.
pub mod errmsg {
    pub const CONNECTION_FAILED: &str = "connection failed: ";
    pub const TRANSPORT_ERROR: &str = "transport error: ";
    pub const GRPC_ERROR: &str = "grpc error: ";
    pub const INVALID_ARGUMENT: &str = "invalid argument: ";
    pub const INVALID_TIMESTAMP: &str = "invalid timestamp: ";
    pub const REJECTED: &str = "command rejected: ";
}

/// Errors that can occur during client operations.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Failed to establish connection to the server.
    #[error("{}{}", errmsg::CONNECTION_FAILED, .0)]
    Connection(String),

    /// Transport-level error from tonic.
    #[error("{}{}", errmsg::TRANSPORT_ERROR, .0)]
    Transport(#[from] tonic::transport::Error),

    /// gRPC error from the server.
    #[error("{}{}", errmsg::GRPC_ERROR, .0)]
    Grpc(Box<Status>),

    /// Invalid argument provided by caller.
    #[error("{}{}", errmsg::INVALID_ARGUMENT, .0)]
    InvalidArgument(String),

    /// Failed to parse timestamp.
    #[error("{}{}", errmsg::INVALID_TIMESTAMP, .0)]
    InvalidTimestamp(String),

    /// Business-rule rejection raised by a command handler.
    ///
    /// Wraps [`CommandRejectedError`] so the same status-class predicates
    /// (`is_not_found`, `is_precondition_failed`, `is_invalid_argument`)
    /// answer correctly whether the failure was a gRPC status from the
    /// server or a local rejection. Mirrors Python's class hierarchy where
    /// `CommandRejectedError(ClientError)` participates in the same
    /// polymorphic predicate surface.
    #[error("{}{}", errmsg::REJECTED, .0.reason)]
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
    /// Returns the error message.
    pub fn message(&self) -> String {
        match self {
            ClientError::Connection(msg) => msg.clone(),
            ClientError::Transport(e) => e.to_string(),
            ClientError::Grpc(s) => s.message().to_string(),
            ClientError::InvalidArgument(msg) => msg.clone(),
            ClientError::InvalidTimestamp(msg) => msg.clone(),
            ClientError::Rejected(r) => r.reason.clone(),
        }
    }

    /// Returns the gRPC status code if this is a gRPC error.
    ///
    /// Returns `None` for `Rejected` because the rejection's
    /// `status_code` is a string (`"FAILED_PRECONDITION"`,
    /// `"INVALID_ARGUMENT"`, `"NOT_FOUND"`), not a `tonic::Code`. Use
    /// the `is_*` predicates to classify a rejection.
    pub fn code(&self) -> Option<Code> {
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
        matches!(self.code(), Some(Code::NotFound))
            || matches!(self, ClientError::Rejected(r) if r.is_not_found())
    }

    /// Returns true if this is a "precondition failed" error.
    pub fn is_precondition_failed(&self) -> bool {
        matches!(self.code(), Some(Code::FailedPrecondition))
            || matches!(self, ClientError::Rejected(r) if r.is_precondition_failed())
    }

    /// Returns true if this is an "invalid argument" error.
    pub fn is_invalid_argument(&self) -> bool {
        matches!(self.code(), Some(Code::InvalidArgument))
            || matches!(self, ClientError::InvalidArgument(_))
            || matches!(self, ClientError::Rejected(r) if r.is_invalid_argument())
    }

    /// Returns true for connection/transport-class errors — including a gRPC
    /// `UNAVAILABLE` status, which is the standard way for gRPC services to
    /// signal a transient transport-level outage. Mirrors the Python client's
    /// classification so retry decisions agree across languages.
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
#[derive(Debug, Clone)]
pub struct CommandRejectedError {
    pub reason: String,
    pub status_code: String,
}

impl CommandRejectedError {
    /// Create a FAILED_PRECONDITION rejection (default for guard failures).
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            status_code: "FAILED_PRECONDITION".to_string(),
        }
    }

    /// Explicit FAILED_PRECONDITION factory. Cross-language alias for
    /// [`CommandRejectedError::new`]; present for name-parity with
    /// Python's `CommandRejectedError.precondition_failed(msg)`.
    pub fn precondition_failed(reason: impl Into<String>) -> Self {
        Self::new(reason)
    }

    /// Create an INVALID_ARGUMENT rejection for input validation failures.
    pub fn invalid_argument(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            status_code: "INVALID_ARGUMENT".to_string(),
        }
    }

    /// Create a NOT_FOUND rejection for missing-aggregate failures.
    ///
    /// Not retryable — refetching events cannot change the outcome.
    pub fn not_found(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            status_code: "NOT_FOUND".to_string(),
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
        write!(f, "Command rejected: {}", self.reason)
    }
}

impl std::error::Error for CommandRejectedError {}

impl From<CommandRejectedError> for Status {
    fn from(err: CommandRejectedError) -> Self {
        match err.status_code.as_str() {
            "INVALID_ARGUMENT" => Status::invalid_argument(err.reason),
            "NOT_FOUND" => Status::not_found(err.reason),
            _ => Status::failed_precondition(err.reason),
        }
    }
}

/// Result type for command/event handlers.
pub type CommandResult<T> = std::result::Result<T, CommandRejectedError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_connection_error_includes_unavailable_grpc() {
        let err = ClientError::from(Status::unavailable("backend down"));
        assert!(
            err.is_connection_error(),
            "Grpc(UNAVAILABLE) must classify as connection error to mirror Python"
        );
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
            assert!(
                !err.is_connection_error(),
                "non-UNAVAILABLE Grpc must not classify as connection error: {:?}",
                err
            );
        }
    }

    #[test]
    fn is_connection_error_includes_connection_and_transport_variants() {
        assert!(ClientError::Connection("x".into()).is_connection_error());
        // Transport variant is constructed via tonic::transport::Error which we can't
        // synthesize directly here; covered by the existing cucumber step that
        // checks ClientError::Connection-as-stand-in-for-transport.
    }

    #[test]
    fn rejected_into_client_error_routes_not_found_predicate() {
        // P2.3: a CommandRejectedError converted via Into<ClientError>
        // answers `is_not_found` polymorphically — same as Python's
        // CommandRejectedError(ClientError) inheritance lets
        // `client_error.is_not_found()` work without casting.
        let rej = CommandRejectedError::not_found("aggregate missing");
        let ce: ClientError = rej.into();
        assert!(ce.is_not_found(), "Rejected(NOT_FOUND) must classify as not_found");
        assert!(!ce.is_precondition_failed());
        assert!(!ce.is_invalid_argument());
        assert!(!ce.is_connection_error());
    }

    #[test]
    fn rejected_into_client_error_routes_invalid_argument_predicate() {
        let rej = CommandRejectedError::invalid_argument("bad input");
        let ce: ClientError = rej.into();
        assert!(ce.is_invalid_argument());
        assert!(!ce.is_not_found());
        assert!(!ce.is_precondition_failed());
    }

    #[test]
    fn rejected_into_client_error_routes_precondition_predicate() {
        let rej = CommandRejectedError::new("guard");
        let ce: ClientError = rej.into();
        assert!(ce.is_precondition_failed());
        assert!(!ce.is_not_found());
        assert!(!ce.is_invalid_argument());
    }

    #[test]
    fn rejected_into_client_error_preserves_message_and_format() {
        let rej = CommandRejectedError::not_found("widget 42");
        let ce: ClientError = rej.into();
        assert_eq!(ce.message(), "widget 42");
        // Display goes through `command rejected: …`, mirroring the
        // standalone CommandRejectedError's Display impl.
        assert_eq!(ce.to_string(), "command rejected: widget 42");
    }
}
