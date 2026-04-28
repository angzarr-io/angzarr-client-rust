//! Cross-language inventory of stable error codes, static messages, and
//! detail-map keys.
//!
//! Audit finding #59 (structural error model). Three sub-modules:
//!
//! - [`codes`] — SCREAMING_SNAKE event-type identifiers. Carried on the
//!   `code` field of every error.
//! - [`messages`] — static human-readable messages. Carried on the
//!   `message` field. Same constant name as the corresponding code.
//! - [`keys`] — detail-map key strings. Used at insertion sites and in
//!   `details["..."]` lookups.
//!
//! Adding a new error: add the constant in all three sub-modules (with
//! the same name in `codes` and `messages`) AND add the parallel
//! constants in `client-python/main/angzarr_client/error_codes.py`,
//! then reference them at the call site.

/// SCREAMING_SNAKE event-type identifiers — the value of `code` on
/// every error this client constructs.
pub mod codes {
    // Validation
    pub const VALUE_NOT_POSITIVE: &str = "VALUE_NOT_POSITIVE";
    pub const VALUE_NOT_NON_NEGATIVE: &str = "VALUE_NOT_NON_NEGATIVE";
    pub const VALUE_EMPTY: &str = "VALUE_EMPTY";
    pub const COLLECTION_EMPTY: &str = "COLLECTION_EMPTY";
    pub const ENTITY_NOT_FOUND: &str = "ENTITY_NOT_FOUND";
    pub const ENTITY_ALREADY_EXISTS: &str = "ENTITY_ALREADY_EXISTS";
    pub const STATUS_MISMATCH: &str = "STATUS_MISMATCH";
    pub const STATUS_FORBIDDEN: &str = "STATUS_FORBIDDEN";

    // Builder
    pub const COMMAND_TYPE_URL_MISSING: &str = "COMMAND_TYPE_URL_MISSING";
    pub const COMMAND_PAYLOAD_MISSING: &str = "COMMAND_PAYLOAD_MISSING";
    pub const COMMAND_SEQUENCE_MISSING: &str = "COMMAND_SEQUENCE_MISSING";

    // Conversion
    pub const ANY_TYPE_MISMATCH: &str = "ANY_TYPE_MISMATCH";
    pub const ANY_DECODE_FAILED: &str = "ANY_DECODE_FAILED";
    pub const PROTO_UUID_INVALID: &str = "PROTO_UUID_INVALID";
    pub const TIMESTAMP_PARSE_FAILED: &str = "TIMESTAMP_PARSE_FAILED";

    // Transport / connection
    pub const ENDPOINT_PARSE_FAILED: &str = "ENDPOINT_PARSE_FAILED";
    pub const ENDPOINT_INVALID_URI: &str = "ENDPOINT_INVALID_URI";
    pub const CONNECTION_FAILED: &str = "CONNECTION_FAILED";
    pub const CONNECTION_FAILED_MAX_RETRIES: &str = "CONNECTION_FAILED_MAX_RETRIES";
    pub const TRANSPORT_ERROR: &str = "TRANSPORT_ERROR";
    pub const GRPC_ERROR: &str = "GRPC_ERROR";

    // Dispatch — common
    pub const HANDLER_WRONG_RESPONSE_KIND: &str = "HANDLER_WRONG_RESPONSE_KIND";
    pub const HANDLER_WRONG_REQUEST_KIND: &str = "HANDLER_WRONG_REQUEST_KIND";
    pub const NO_HANDLER_REGISTERED: &str = "NO_HANDLER_REGISTERED";
    pub const MISSING_COMMAND_BOOK: &str = "MISSING_COMMAND_BOOK";
    pub const MISSING_COMMAND_PAGE: &str = "MISSING_COMMAND_PAGE";
    pub const MISSING_COMMAND_PAYLOAD: &str = "MISSING_COMMAND_PAYLOAD";
    pub const NOTIFICATION_DECODE_FAILED: &str = "NOTIFICATION_DECODE_FAILED";
    pub const REJECTION_NOTIFICATION_DECODE_FAILED: &str = "REJECTION_NOTIFICATION_DECODE_FAILED";

    // Dispatch — saga
    pub const MISSING_SAGA_SOURCE: &str = "MISSING_SAGA_SOURCE";
    pub const EMPTY_SAGA_SOURCE: &str = "EMPTY_SAGA_SOURCE";
    pub const MISSING_SAGA_EVENT_PAYLOAD: &str = "MISSING_SAGA_EVENT_PAYLOAD";
    pub const SAGA_INVALID_TYPE_URL: &str = "SAGA_INVALID_TYPE_URL";
    pub const SAGA_HANDLER_UNSUPPORTED_RETURN_TYPE: &str = "SAGA_HANDLER_UNSUPPORTED_RETURN_TYPE";

    // Dispatch — process manager
    pub const MISSING_PM_TRIGGER: &str = "MISSING_PM_TRIGGER";
    pub const EMPTY_PM_TRIGGER: &str = "EMPTY_PM_TRIGGER";
    pub const MISSING_PM_EVENT_PAYLOAD: &str = "MISSING_PM_EVENT_PAYLOAD";
    pub const PM_INVALID_TYPE_URL: &str = "PM_INVALID_TYPE_URL";
    pub const PM_HANDLER_WRONG_RETURN_TYPE: &str = "PM_HANDLER_WRONG_RETURN_TYPE";

    // Dispatch — upcaster
    pub const UPCASTER_WRONG_RESPONSE_KIND: &str = "UPCASTER_WRONG_RESPONSE_KIND";

    // Build-time validation
    pub const HANDLER_FIELD_EMPTY_STRING: &str = "HANDLER_FIELD_EMPTY_STRING";
    pub const HANDLER_FIELD_EMPTY_LIST: &str = "HANDLER_FIELD_EMPTY_LIST";
    pub const HANDLER_STATE_NOT_TYPE: &str = "HANDLER_STATE_NOT_TYPE";
    pub const HANDLER_UNKNOWN_KIND: &str = "HANDLER_UNKNOWN_KIND";
    pub const ROUTER_NO_HANDLERS: &str = "ROUTER_NO_HANDLERS";
}

/// Static human-readable messages — the value of `message` on every
/// error this client constructs. Constant names match the corresponding
/// `codes::` constant.
pub mod messages {
    // Validation
    pub const VALUE_NOT_POSITIVE: &str = "value must be positive";
    pub const VALUE_NOT_NON_NEGATIVE: &str = "value must be non-negative";
    pub const VALUE_EMPTY: &str = "value must not be empty";
    pub const COLLECTION_EMPTY: &str = "collection must not be empty";
    pub const ENTITY_NOT_FOUND: &str = "entity not found";
    pub const ENTITY_ALREADY_EXISTS: &str = "entity already exists";
    pub const STATUS_MISMATCH: &str = "status does not match expected";
    pub const STATUS_FORBIDDEN: &str = "status is the forbidden value";

    // Builder
    pub const COMMAND_TYPE_URL_MISSING: &str = "command type_url not set";
    pub const COMMAND_PAYLOAD_MISSING: &str = "command payload not set";
    pub const COMMAND_SEQUENCE_MISSING: &str = "sequence not set (call with_sequence)";

    // Conversion
    pub const ANY_TYPE_MISMATCH: &str = "Any type_url does not match expected message type";
    pub const ANY_DECODE_FAILED: &str = "failed to decode Any payload into expected message type";
    pub const PROTO_UUID_INVALID: &str = "proto UUID bytes are not a valid 16-byte UUID";
    pub const TIMESTAMP_PARSE_FAILED: &str = "failed to parse RFC3339 timestamp";

    // Transport / connection
    pub const ENDPOINT_PARSE_FAILED: &str = "failed to parse fallback endpoint";
    pub const ENDPOINT_INVALID_URI: &str = "endpoint URI is invalid";
    pub const CONNECTION_FAILED: &str = "connection failed";
    pub const CONNECTION_FAILED_MAX_RETRIES: &str = "connection failed after max retries";

    // Dispatch — common
    pub const HANDLER_WRONG_RESPONSE_KIND: &str = "handler returned wrong response kind";
    pub const HANDLER_WRONG_REQUEST_KIND: &str = "handler dispatched with wrong request kind";
    pub const NO_HANDLER_REGISTERED: &str =
        "no handler registered for the given (domain, type_url)";
    pub const MISSING_COMMAND_BOOK: &str = "missing command book";
    pub const MISSING_COMMAND_PAGE: &str = "missing command page";
    pub const MISSING_COMMAND_PAYLOAD: &str = "missing command payload";
    pub const NOTIFICATION_DECODE_FAILED: &str = "failed to decode Notification payload";
    pub const REJECTION_NOTIFICATION_DECODE_FAILED: &str =
        "failed to decode RejectionNotification payload";

    // Dispatch — saga
    pub const MISSING_SAGA_SOURCE: &str = "missing saga source";
    pub const EMPTY_SAGA_SOURCE: &str = "empty saga source";
    pub const MISSING_SAGA_EVENT_PAYLOAD: &str = "missing event payload";
    pub const SAGA_INVALID_TYPE_URL: &str = "saga trigger has invalid type_url";
    pub const SAGA_HANDLER_UNSUPPORTED_RETURN_TYPE: &str =
        "saga handler returned unsupported type";

    // Dispatch — process manager
    pub const MISSING_PM_TRIGGER: &str = "missing PM trigger";
    pub const EMPTY_PM_TRIGGER: &str = "empty PM trigger";
    pub const MISSING_PM_EVENT_PAYLOAD: &str = "missing event payload on PM trigger";
    pub const PM_INVALID_TYPE_URL: &str = "PM trigger has invalid type_url";
    pub const PM_HANDLER_WRONG_RETURN_TYPE: &str =
        "PM handler must return ProcessManagerResponse";

    // Dispatch — upcaster
    pub const UPCASTER_WRONG_RESPONSE_KIND: &str = "upcaster handler returned non-Upcaster response";

    // Build-time validation
    pub const HANDLER_FIELD_EMPTY_STRING: &str = "handler field must be a non-empty string";
    pub const HANDLER_FIELD_EMPTY_LIST: &str = "handler field must be a non-empty list";
    pub const HANDLER_STATE_NOT_TYPE: &str = "handler 'state' must be a type";
    pub const HANDLER_UNKNOWN_KIND: &str = "unknown handler kind";
    pub const ROUTER_NO_HANDLERS: &str = "no handlers registered on Router";
}

/// Detail-map key constants — the keys used in the `details` mapping on
/// every error this client constructs. Use these at insertion sites
/// AND when reading `details[key]` in tests.
pub mod keys {
    pub const FIELD: &str = "field";
    pub const CONTEXT: &str = "context";
    pub const CAUSE: &str = "cause";
    pub const ENDPOINT: &str = "endpoint";
    pub const INPUT: &str = "input";
    pub const EXPECTED: &str = "expected";
    pub const ACTUAL: &str = "actual";
    pub const EXPECTED_KIND: &str = "expected_kind";
    pub const DOMAIN: &str = "domain";
    pub const TYPE_URL: &str = "type_url";
    pub const ROUTER_NAME: &str = "router_name";
    pub const HANDLER_CLASS: &str = "handler_class";
    pub const HANDLER_KIND: &str = "handler_kind";
    pub const ACTUAL_RETURN_TYPE: &str = "actual_return_type";
}
