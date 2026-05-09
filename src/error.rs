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

use std::collections::{BTreeMap, HashMap};

use prost::Message;
use prost_types::Any;
use tonic::{Code, Status};

/// Logical domain for `google.rpc.ErrorInfo.domain` — identifies the
/// inventory the `reason` (= `code`) is defined in. Any sibling client
/// that reads `grpc-status-details-bin` keys off this value to know
/// it's looking at an angzarr-emitted error.
pub const ERROR_INFO_DOMAIN: &str = "angzarr.io";

/// Subset of `google/rpc/status.proto` matching `google.rpc.Status`
/// wire format. Hand-rolled to avoid a new build-time dep on the
/// `googleapis` proto bundle — the struct is tiny and the wire layout
/// is stable.
#[derive(Clone, PartialEq, Message)]
struct GoogleRpcStatus {
    /// `google.rpc.Code` numeric value — the same int that rides on
    /// `grpc-status`.
    #[prost(int32, tag = "1")]
    code: i32,
    /// Human-readable message. We duplicate `Status::message()` here
    /// per the spec ("should be the same as `Status.message`").
    #[prost(string, tag = "2")]
    message: String,
    /// Heterogeneous structured details. Each entry is a packed proto
    /// (typically `google.rpc.ErrorInfo`, `google.rpc.BadRequest`, …;
    /// we additionally pack our own `Cover` here when present).
    #[prost(message, repeated, tag = "3")]
    details: Vec<Any>,
}

/// Subset of `google/rpc/error_details.proto` matching
/// `google.rpc.ErrorInfo`. Carries our SCREAMING_SNAKE `code` (as
/// `reason`), an inventory `domain` ([`ERROR_INFO_DOMAIN`]), and the
/// detail map verbatim.
#[derive(Clone, PartialEq, Message)]
struct GoogleRpcErrorInfo {
    /// SCREAMING_SNAKE identifier — the `code` from
    /// [`crate::error_codes::codes`].
    #[prost(string, tag = "1")]
    reason: String,
    /// Logical inventory the `reason` belongs to —
    /// [`ERROR_INFO_DOMAIN`].
    #[prost(string, tag = "2")]
    domain: String,
    /// Per-call structured context; matches the
    /// [`ErrorDetail::details`] map verbatim.
    #[prost(map = "string, string", tag = "3")]
    metadata: HashMap<String, String>,
}

/// Type URL for `google.rpc.ErrorInfo` per the canonical `Any` packing.
const ERROR_INFO_TYPE_URL: &str = "type.googleapis.com/google.rpc.ErrorInfo";

/// Type URL for the project's `Cover` proto (declared in
/// `angzarr_client/proto/angzarr/types.proto`).
const COVER_TYPE_URL: &str = "type.googleapis.com/angzarr_client.proto.angzarr.Cover";

/// Build the canonical `grpc-status-details-bin` payload (a serialized
/// `google.rpc.Status` whose `details` is `repeated Any`) for an
/// angzarr error. Returns the raw bytes ready to feed to
/// [`Status::with_details`].
///
/// The payload always carries a `google.rpc.ErrorInfo`; when `cover`
/// is `Some`, the `Cover` proto is appended as a second `Any`. Polyglot
/// siblings read this trailer with whatever google.rpc bindings their
/// language exposes; tonic stamps the binary trailer + base64 wrapping
/// per gRPC spec.
pub fn build_status_details(
    grpc_code: Code,
    message: &str,
    error_code: &str,
    error_details: Option<&BTreeMap<String, String>>,
    cover: Option<&crate::proto::Cover>,
) -> Vec<u8> {
    let info = GoogleRpcErrorInfo {
        reason: error_code.to_string(),
        domain: ERROR_INFO_DOMAIN.to_string(),
        metadata: error_details
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default(),
    };
    let info_bytes = info.encode_to_vec();
    let mut details = vec![Any {
        type_url: ERROR_INFO_TYPE_URL.to_string(),
        value: info_bytes,
    }];
    if let Some(cover) = cover {
        details.push(Any {
            type_url: COVER_TYPE_URL.to_string(),
            value: cover.encode_to_vec(),
        });
    }
    let status_proto = GoogleRpcStatus {
        code: i32::from(grpc_code),
        message: message.to_string(),
        details,
    };
    status_proto.encode_to_vec()
}

/// Decode the inverse of [`build_status_details`] for tests and
/// inspection. Returns `(error_code, metadata, optional_cover)`.
///
/// Robust to siblings that pack extra `Any` entries we don't recognize
/// — those are silently skipped.
#[cfg(test)]
pub fn unpack_status_details(
    bytes: &[u8],
) -> Option<(String, BTreeMap<String, String>, Option<crate::proto::Cover>)> {
    let status = GoogleRpcStatus::decode(bytes).ok()?;
    let mut error_code = String::new();
    let mut metadata = BTreeMap::new();
    let mut cover = None;
    for any in &status.details {
        if any.type_url == ERROR_INFO_TYPE_URL {
            if let Ok(info) = GoogleRpcErrorInfo::decode(any.value.as_slice()) {
                error_code = info.reason;
                metadata = info.metadata.into_iter().collect();
            }
        } else if any.type_url == COVER_TYPE_URL {
            cover = crate::proto::Cover::decode(any.value.as_slice()).ok();
        }
    }
    Some((error_code, metadata, cover))
}

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
    ///
    /// Audit #76: Display emits the static inventory message; the dynamic
    /// tonic error survives via `source()` on the `#[from]` chain.
    #[error("{}", crate::error_codes::messages::TRANSPORT_ERROR)]
    Transport(#[from] tonic::transport::Error),

    /// gRPC error from the server.
    ///
    /// Audit #76: Display emits the static inventory message; the
    /// server-side message survives via the wrapped `Status`.
    #[error("{}", crate::error_codes::messages::GRPC_ERROR)]
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
    pub fn invalid_argument<I, K, V>(code: &'static str, message: &'static str, details: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self::InvalidArgument(ErrorDetail::new(code, message, details))
    }

    /// Build an `InvalidTimestamp` variant with structured details.
    pub fn invalid_timestamp<I, K, V>(code: &'static str, message: &'static str, details: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self::InvalidTimestamp(ErrorDetail::new(code, message, details))
    }

    /// Build a `Connection` variant with structured details.
    pub fn connection<I, K, V>(code: &'static str, message: &'static str, details: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self::Connection(ErrorDetail::new(code, message, details))
    }

    /// Returns the (static) error message.
    ///
    /// Audit #76: `Transport` and `Grpc` variants now return inventory
    /// constants instead of the underlying tonic/Status message; the
    /// dynamic message survives via `Display` on the wrapped cause.
    pub fn message(&self) -> String {
        match self {
            ClientError::Connection(d) => d.message.to_string(),
            ClientError::Transport(_) => crate::error_codes::messages::TRANSPORT_ERROR.to_string(),
            ClientError::Grpc(_) => crate::error_codes::messages::GRPC_ERROR.to_string(),
            ClientError::InvalidArgument(d) => d.message.to_string(),
            ClientError::InvalidTimestamp(d) => d.message.to_string(),
            ClientError::Rejected(r) => r.message.to_string(),
        }
    }

    /// Returns the SCREAMING_SNAKE error code from the inventory.
    pub fn code(&self) -> &'static str {
        match self {
            ClientError::Connection(d) => d.code,
            ClientError::Transport(_) => crate::error_codes::codes::TRANSPORT_ERROR,
            ClientError::Grpc(_) => crate::error_codes::codes::GRPC_ERROR,
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
///
/// `cover` is the addressing envelope (`domain`, `root`, `correlation_id`,
/// `edition`) of the command that produced this rejection. Handlers do
/// not populate it; the router stamps it from the incoming
/// `ContextualCommand` at the dispatch boundary so every rejection is
/// traceable to its originating workflow without each call site having
/// to thread the context.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRejectedError {
    pub code: &'static str,
    pub message: &'static str,
    pub status_code: &'static str,
    pub details: BTreeMap<String, String>,
    pub cover: Option<crate::proto::Cover>,
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
            cover: None,
        }
    }

    /// Create an INVALID_ARGUMENT rejection for input validation failures.
    pub fn invalid_argument<I, K, V>(code: &'static str, message: &'static str, details: I) -> Self
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
            cover: None,
        }
    }

    /// Create a NOT_FOUND rejection for missing-aggregate failures.
    ///
    /// Not retryable — refetching events cannot change the outcome.
    pub fn not_found<I, K, V>(code: &'static str, message: &'static str, details: I) -> Self
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
            cover: None,
        }
    }

    /// Stamp the addressing envelope. Builder-style for the dispatch
    /// boundary to attach the request's cover to a propagating rejection.
    pub fn with_cover(mut self, cover: crate::proto::Cover) -> Self {
        self.cover = Some(cover);
        self
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
        // Static message rides in `Status::message()` (greppable across
        // languages); the structured `code`, `details` map, and `cover`
        // ride in `grpc-status-details-bin` as a `google.rpc.Status`
        // whose `details: repeated Any` carries
        // `google.rpc.ErrorInfo` + (when present) the angzarr `Cover`
        // proto. Any sibling client with google.rpc bindings can
        // unpack this directly — no custom trailer scheme.
        let grpc_code = match err.status_code {
            "INVALID_ARGUMENT" => Code::InvalidArgument,
            "NOT_FOUND" => Code::NotFound,
            _ => Code::FailedPrecondition,
        };
        let payload = build_status_details(
            grpc_code,
            err.message,
            err.code,
            Some(&err.details),
            err.cover.as_ref(),
        );
        Status::with_details(grpc_code, err.message, bytes::Bytes::from(payload))
    }
}

/// Result type for command/event handlers.
pub type CommandResult<T> = std::result::Result<T, CommandRejectedError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_cover_stamps_addressing_envelope() {
        use crate::proto::{Cover, Uuid};
        let rej = CommandRejectedError::precondition_failed(
            "TEST_CODE",
            "test message",
            std::iter::empty::<(&str, &str)>(),
        );
        assert!(rej.cover.is_none(), "default cover is None");

        let stamped = rej.with_cover(Cover {
            domain: "player".into(),
            root: Some(Uuid {
                value: vec![0xab, 0xcd],
            }),
            correlation_id: "corr-123".into(),
            edition: None,
        });
        let cover = stamped.cover.expect("cover stamped");
        assert_eq!(cover.domain, "player");
        assert_eq!(cover.correlation_id, "corr-123");
        assert_eq!(cover.root.unwrap().value, vec![0xab, 0xcd]);
    }

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

    #[test]
    fn rejected_into_status_packs_canonical_error_info() {
        // From<CommandRejectedError> for Status packs the structured
        // payload into `grpc-status-details-bin` as a google.rpc.Status
        // carrying a google.rpc.ErrorInfo. Polyglot siblings read the
        // canonical trailer via their google.rpc bindings — no custom
        // trailer scheme.
        let rej = CommandRejectedError::precondition_failed(
            "ALREADY_OPEN",
            "registration already open",
            [("field", "status")],
        );
        let status: Status = rej.into();
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert_eq!(status.message(), "registration already open");

        let details = status.details();
        assert!(!details.is_empty(), "binary details trailer must be set");
        let (code, metadata, cover) = unpack_status_details(&details).expect("decode");
        assert_eq!(code, "ALREADY_OPEN");
        assert_eq!(metadata.get("field").map(String::as_str), Some("status"));
        assert!(cover.is_none());
    }

    #[test]
    fn rejected_into_status_packs_cover_when_present() {
        use crate::proto::{Cover, Uuid as ProtoUuid};
        let rej = CommandRejectedError::not_found(
            "ENTITY_NOT_FOUND",
            "entity not found",
            std::iter::empty::<(String, String)>(),
        )
        .with_cover(Cover {
            domain: "player".into(),
            root: Some(ProtoUuid {
                value: vec![0xab, 0xcd, 0xef],
            }),
            correlation_id: "corr-42".into(),
            edition: None,
        });
        let status: Status = rej.into();
        let (code, _metadata, unpacked) =
            unpack_status_details(&status.details()).expect("decode");
        assert_eq!(code, "ENTITY_NOT_FOUND");
        let cover = unpacked.expect("cover roundtripped");
        assert_eq!(cover.domain, "player");
        assert_eq!(cover.correlation_id, "corr-42");
        assert_eq!(cover.root.unwrap().value, vec![0xab, 0xcd, 0xef]);
    }

    #[test]
    fn rejected_into_status_skips_cover_when_absent() {
        // No cover stamped on the rejection → no Cover Any in
        // `details`. ErrorInfo is always packed.
        let rej = CommandRejectedError::precondition_failed(
            "X",
            "x",
            std::iter::empty::<(String, String)>(),
        );
        let status: Status = rej.into();
        let (code, _metadata, cover) =
            unpack_status_details(&status.details()).expect("decode");
        assert_eq!(code, "X");
        assert!(cover.is_none());
    }

    #[test]
    fn error_info_domain_is_stable() {
        // Pin the inventory domain. Any sibling that keys on this
        // value to recognize an angzarr-emitted error fails fast if
        // we ever rename it.
        assert_eq!(ERROR_INFO_DOMAIN, "angzarr.io");
    }

    // Audit #76 + #78: Transport / Grpc variants emit static inventory
    // messages (no leak of dynamic tonic / Status text) and surface the
    // SCREAMING_SNAKE codes from the inventory.

    #[test]
    fn grpc_variant_message_and_code_are_static_inventory() {
        let err = ClientError::from(Status::not_found("the actual server detail leaks"));
        assert_eq!(err.message(), crate::error_codes::messages::GRPC_ERROR);
        assert_eq!(err.code(), crate::error_codes::codes::GRPC_ERROR);
        assert_eq!(err.to_string(), crate::error_codes::messages::GRPC_ERROR);
        // The dynamic detail is still reachable via the wrapped Status.
        assert_eq!(
            err.status().map(|s| s.message()),
            Some("the actual server detail leaks"),
        );
    }
}
