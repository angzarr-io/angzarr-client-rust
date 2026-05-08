//! Constants used across the proto extensions.

/// gRPC metadata key for correlation ID propagation.
pub const CORRELATION_ID_HEADER: &str = "x-correlation-id";

/// Fallback domain when cover is missing or has no domain set.
pub const UNKNOWN_DOMAIN: &str = "unknown";

/// Domain prefix for synthetic projection event books.
///
/// Projector output is published as `_projection.{projector_name}.{domain}`.
pub const PROJECTION_DOMAIN_PREFIX: &str = "_projection";

/// Protobuf type URL for serialized Projection messages in synthetic event books.
pub const PROJECTION_TYPE_URL: &str = "angzarr_client.proto.angzarr.Projection";

/// Wildcard domain for catch-all routing (matches any domain).
pub const WILDCARD_DOMAIN: &str = "*";

/// The meta domain for angzarr infrastructure.
pub const META_ANGZARR_DOMAIN: &str = "_angzarr";

/// Default edition name for the main timeline.
///
/// The main timeline is represented by an empty string — no "angzarr"
/// sentinel taking space in every row. Named editions use their own
/// non-empty identifier.
pub const DEFAULT_EDITION: &str = "";

/// Type URL prefix for googleapis.com protobuf Any messages.
///
/// Used by `decode_typed` to match type URLs in Event/Command payloads.
pub const TYPE_URL_PREFIX: &str = "type.googleapis.com/";

/// Type URL prefix for angzarr-internal framework messages
/// (Notification, Revocation, Confirmation, Compensate, NoOp, …).
///
/// Pinned by the `type_url_constants_share_prefix` test in
/// `proto_ext::type_url` so a typo in any one of the
/// `proto_ext::type_url::*` constants fails compilation tests.
pub const ANGZARR_TYPE_URL_PREFIX: &str = "type.angzarr.io/";
