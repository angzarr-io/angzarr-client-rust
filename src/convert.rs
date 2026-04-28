//! Conversion helpers for protobuf types.

use crate::error::{ClientError, Result};
use crate::error_codes::{codes, keys, messages};
use crate::proto::Uuid as ProtoUuid;
use chrono::{DateTime, Utc};
use prost::Name;
use prost_types::{Any, Timestamp};
use uuid::Uuid;

/// Default type URL prefix for protocol buffer messages.
pub const TYPE_URL_PREFIX: &str = "type.googleapis.com/";

/// Canonical domain identifiers — match Python's `angzarr_client.helpers`.
pub const UNKNOWN_DOMAIN: &str = "unknown";
pub const WILDCARD_DOMAIN: &str = "*";
pub const DEFAULT_EDITION: &str = "";
pub const META_ANGZARR_DOMAIN: &str = "_angzarr";
pub const PROJECTION_DOMAIN_PREFIX: &str = "_projection";
pub const PROJECTION_TYPE_URL: &str = "angzarr_client.proto.angzarr.Projection";

/// Proto package prefix used by the angzarr_client.proto.* proto package
/// declarations. **NOT** stripped from proto-Any `type_url`s — per the
/// `google.protobuf.Any` spec the URL must contain the *fully qualified*
/// type name (i.e., the value of `Message::DESCRIPTOR.full_name()`),
/// which includes the package prefix verbatim. Audit finding #58.
///
/// Retained because the angzarr-internal `type.angzarr.io/` routing
/// scheme (`proto_ext::type_url`) intentionally uses a short form for
/// readability — that's a different URL contract from Any's.
pub const INTERNAL_PACKAGE_PREFIX: &str = "angzarr_client.proto.";

/// Strip the `angzarr_client.proto.` package prefix from a full proto
/// type name. Used **only** by the angzarr-internal
/// `type.angzarr.io/` URL scheme (`proto_ext::type_url::for_type`),
/// where short forms are deliberate. **Do not call from any code that
/// builds a `type.googleapis.com/...` proto-Any URL** — Any URLs must
/// carry the fully qualified type name per spec (audit finding #58).
pub fn wire_name(full_name: &str) -> &str {
    full_name
        .strip_prefix(INTERNAL_PACKAGE_PREFIX)
        .unwrap_or(full_name)
}

/// Build a fully-qualified `type.googleapis.com/...` URL from a message's
/// fully-qualified proto type name.
///
/// Per `google.protobuf.Any` spec, the URL's last path segment **must**
/// be the message's fully qualified name (`<package>.<MessageName>`).
/// Audit finding #58: this used to incorrectly strip the
/// `angzarr_client.proto.` prefix via `wire_name`, producing
/// non-spec-compliant URLs that diverged from Python's emission. The
/// strip has been removed.
///
/// # Examples
/// ```
/// use angzarr_client::convert::type_url;
/// assert_eq!(
///     type_url("angzarr_client.proto.examples.AddItemToCart"),
///     "type.googleapis.com/angzarr_client.proto.examples.AddItemToCart"
/// );
/// ```
pub fn type_url(type_name: &str) -> String {
    format!("{}{}", TYPE_URL_PREFIX, type_name)
}

/// Extract the wire-format type name from a type URL.
///
/// Returns the part after the last `/` (e.g., "examples.PlayerRegistered").
/// Callers that need the Rust-internal name can prepend
/// [`INTERNAL_PACKAGE_PREFIX`] or use [`wire_name`]'s inverse.
pub fn type_name_from_url(type_url: &str) -> &str {
    type_url.rsplit('/').next().unwrap_or(type_url)
}

/// Check if a type URL matches the given fully-qualified type name exactly.
///
/// Audit finding #58: comparison uses the spec-compliant fully qualified
/// name verbatim — no `wire_name` strip — so it matches Python's
/// emission and the `google.protobuf.Any` contract.
///
/// # Examples
/// ```
/// use angzarr_client::convert::type_url_matches_exact;
/// assert!(type_url_matches_exact(
///     "type.googleapis.com/angzarr_client.proto.examples.PlayerRegistered",
///     "angzarr_client.proto.examples.PlayerRegistered"
/// ));
/// ```
pub fn type_url_matches_exact(type_url: &str, full_type_name: &str) -> bool {
    type_url == format!("{}{}", TYPE_URL_PREFIX, full_type_name)
}

/// Python-canonical name for [`type_url_matches_exact`]. Python exposes
/// `type_url_matches` as the primary function and `type_url_matches_exact`
/// as a Rust-compat alias; Rust reciprocates so either call shape works in
/// either language.
pub fn type_url_matches(type_url: &str, full_type_name: &str) -> bool {
    type_url_matches_exact(type_url, full_type_name)
}

// Type-safe reflection helpers using prost::Name

/// Check if an Any contains a message of type T using prost::Name reflection.
///
/// This is preferred over string-based suffix matching.
///
/// # Examples
/// ```ignore
/// use angzarr_client::convert::type_matches;
/// use examples::PlayerRegistered;
///
/// let any: prost_types::Any = /* ... */;
/// if type_matches::<PlayerRegistered>(&any) {
///     let msg = try_unpack::<PlayerRegistered>(&any).unwrap();
/// }
/// ```
pub fn type_matches<T: prost::Message + Name>(any: &Any) -> bool {
    any.type_url == full_type_url::<T>()
}

/// Unpack an Any to type T if the type matches, returning None otherwise.
///
/// This is type-safe: it only unpacks if the type URL matches exactly.
pub fn try_unpack<T: prost::Message + Default + Name>(any: &Any) -> Option<T> {
    if type_matches::<T>(any) {
        T::decode(any.value.as_slice()).ok()
    } else {
        None
    }
}

/// Unpack an Any to type T, returning an error if type doesn't match or decode fails.
pub fn unpack<T: prost::Message + Default + Name>(any: &Any) -> Result<T> {
    let expected = full_type_url::<T>();
    if any.type_url != expected {
        return Err(ClientError::invalid_argument(
            codes::ANY_TYPE_MISMATCH,
            messages::ANY_TYPE_MISMATCH,
            [
                (keys::EXPECTED, expected),
                (keys::ACTUAL, any.type_url.clone()),
            ],
        ));
    }
    T::decode(any.value.as_slice()).map_err(|e| {
        ClientError::invalid_argument(
            codes::ANY_DECODE_FAILED,
            messages::ANY_DECODE_FAILED,
            [
                (keys::EXPECTED, expected.clone()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })
}

/// Get the full type URL for message type T.
///
/// # Examples
/// ```ignore
/// use angzarr_client::convert::full_type_url;
/// use examples::PlayerRegistered;
///
/// assert_eq!(
///     full_type_url::<PlayerRegistered>(),
///     "type.googleapis.com/examples.PlayerRegistered"
/// );
/// ```
pub fn full_type_url<T: Name>() -> String {
    // Audit finding #58: emit the fully qualified name verbatim per
    // `google.protobuf.Any` spec — no `wire_name` strip.
    format!("{}{}", TYPE_URL_PREFIX, T::full_name())
}

/// Get the fully-qualified type name for message type T (without URL prefix).
pub fn full_type_name<T: Name>() -> String {
    T::full_name()
}

/// Convert a UUID to its protobuf representation.
pub fn uuid_to_proto(uuid: Uuid) -> ProtoUuid {
    ProtoUuid {
        value: uuid.as_bytes().to_vec(),
    }
}

/// Convert a protobuf UUID to a standard UUID.
pub fn proto_to_uuid(proto: &ProtoUuid) -> Result<Uuid> {
    Uuid::from_slice(&proto.value).map_err(|e| {
        ClientError::invalid_argument(
            codes::PROTO_UUID_INVALID,
            messages::PROTO_UUID_INVALID,
            [(keys::CAUSE, e.to_string())],
        )
    })
}

/// Parse an RFC3339 timestamp string into a protobuf Timestamp.
///
/// # Examples
/// ```
/// use angzarr_client::convert::parse_timestamp;
/// let ts = parse_timestamp("2024-01-15T10:30:00Z").unwrap();
/// assert_eq!(ts.seconds, 1705314600);
/// ```
pub fn parse_timestamp(rfc3339: &str) -> Result<Timestamp> {
    let dt: DateTime<Utc> = rfc3339.parse().map_err(|e: chrono::ParseError| {
        ClientError::invalid_timestamp(
            codes::TIMESTAMP_PARSE_FAILED,
            messages::TIMESTAMP_PARSE_FAILED,
            [
                (keys::INPUT, rfc3339.to_string()),
                (keys::CAUSE, e.to_string()),
            ],
        )
    })?;

    Ok(Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    })
}

/// Get the current time as a protobuf Timestamp.
pub fn now() -> Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before unix epoch");

    Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_url() {
        // Audit finding #58: per `google.protobuf.Any` spec the URL
        // carries the fully qualified type name verbatim (package
        // prefix retained).
        assert_eq!(
            type_url("angzarr_client.proto.examples.AddItemToCart"),
            "type.googleapis.com/angzarr_client.proto.examples.AddItemToCart"
        );
    }

    #[test]
    fn test_type_name_from_url() {
        assert_eq!(
            type_name_from_url("type.googleapis.com/angzarr_client.proto.examples.AddItemToCart"),
            "angzarr_client.proto.examples.AddItemToCart"
        );
        assert_eq!(type_name_from_url("AddItemToCart"), "AddItemToCart");
    }

    #[test]
    fn test_type_url_matches_exact() {
        // Audit finding #58: match on the fully qualified name.
        assert!(type_url_matches_exact(
            "type.googleapis.com/angzarr_client.proto.examples.AddItemToCart",
            "angzarr_client.proto.examples.AddItemToCart"
        ));
        assert!(!type_url_matches_exact(
            "type.googleapis.com/angzarr_client.proto.examples.AddItemToCart",
            "angzarr_client.proto.examples.RemoveItem"
        ));
        // Short-form URLs no longer match (was the pre-#58 incorrect behavior).
        assert!(!type_url_matches_exact(
            "type.googleapis.com/examples.AddItemToCart",
            "angzarr_client.proto.examples.AddItemToCart"
        ));
        // Suffix matching never worked.
        assert!(!type_url_matches_exact(
            "type.googleapis.com/angzarr_client.proto.examples.AddItemToCart",
            "AddItemToCart"
        ));
    }

    #[test]
    fn type_url_matches_is_alias_for_exact() {
        // Python-canonical name. Must behave identically to type_url_matches_exact.
        assert!(type_url_matches(
            "type.googleapis.com/angzarr_client.proto.examples.AddItemToCart",
            "angzarr_client.proto.examples.AddItemToCart"
        ));
        assert!(!type_url_matches(
            "type.googleapis.com/angzarr_client.proto.examples.AddItemToCart",
            "AddItemToCart"
        ));
    }

    #[test]
    fn test_uuid_conversion() {
        let uuid = Uuid::new_v4();
        let proto = uuid_to_proto(uuid);
        let back = proto_to_uuid(&proto).unwrap();
        assert_eq!(uuid, back);
    }

    #[test]
    fn test_parse_timestamp() {
        let ts = parse_timestamp("2024-01-15T10:30:00Z").unwrap();
        assert_eq!(ts.seconds, 1705314600);
        assert_eq!(ts.nanos, 0);
    }

    #[test]
    fn test_parse_timestamp_with_nanos() {
        let ts = parse_timestamp("2024-01-15T10:30:00.123456789Z").unwrap();
        assert_eq!(ts.seconds, 1705314600);
        assert_eq!(ts.nanos, 123456789);
    }

    #[test]
    fn test_parse_timestamp_invalid() {
        assert!(parse_timestamp("not a timestamp").is_err());
    }
}
