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
        }
    }

    /// Returns the gRPC status code if this is a gRPC error.
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
    }

    /// Returns true if this is a "precondition failed" error.
    pub fn is_precondition_failed(&self) -> bool {
        matches!(self.code(), Some(Code::FailedPrecondition))
    }

    /// Returns true if this is an "invalid argument" error.
    pub fn is_invalid_argument(&self) -> bool {
        matches!(self.code(), Some(Code::InvalidArgument))
            || matches!(self, ClientError::InvalidArgument(_))
    }

    /// Returns true if this is a connection or transport error.
    pub fn is_connection_error(&self) -> bool {
        matches!(self, ClientError::Connection(_) | ClientError::Transport(_))
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
