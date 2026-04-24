//! Conversion helpers for protobuf types.

use crate::error::{ClientError, Result};
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

/// Rust-only package prefix produced by prost from the `angzarr_client.proto.*`
/// proto packages. Wire type URLs omit this prefix so that other clients
/// (Go/Python/Java) see the short cross-language names.
pub const INTERNAL_PACKAGE_PREFIX: &str = "angzarr_client.proto.";

/// Strip the Rust-only package prefix to get the wire-format name.
pub fn wire_name(full_name: &str) -> &str {
    full_name
        .strip_prefix(INTERNAL_PACKAGE_PREFIX)
        .unwrap_or(full_name)
}

/// Build a fully-qualified type URL from a message type name.
///
/// # Examples
/// ```
/// use angzarr_client::convert::type_url;
/// assert_eq!(type_url("angzarr_client.proto.examples.AddItemToCart"), "type.googleapis.com/examples.AddItemToCart");
/// ```
pub fn type_url(type_name: &str) -> String {
    format!("{}{}", TYPE_URL_PREFIX, wire_name(type_name))
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
/// # Examples
/// ```
/// use angzarr_client::convert::type_url_matches_exact;
/// assert!(type_url_matches_exact(
///     "type.googleapis.com/examples.PlayerRegistered",
///     "angzarr_client.proto.examples.PlayerRegistered"
/// ));
/// ```
pub fn type_url_matches_exact(type_url: &str, full_type_name: &str) -> bool {
    type_url == format!("{}{}", TYPE_URL_PREFIX, wire_name(full_type_name))
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
        return Err(ClientError::InvalidArgument(format!(
            "type mismatch: expected {}, got {}",
            expected, any.type_url
        )));
    }
    T::decode(any.value.as_slice())
        .map_err(|e| ClientError::InvalidArgument(format!("decode error: {}", e)))
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
    format!("{}{}", TYPE_URL_PREFIX, wire_name(&T::full_name()))
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
    Uuid::from_slice(&proto.value)
        .map_err(|e| ClientError::InvalidArgument(format!("invalid UUID: {}", e)))
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
    let dt: DateTime<Utc> = rfc3339
        .parse()
        .map_err(|e| ClientError::InvalidTimestamp(format!("{}: {}", rfc3339, e)))?;

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
        assert_eq!(
            type_url("angzarr_client.proto.examples.AddItemToCart"),
            "type.googleapis.com/examples.AddItemToCart"
        );
    }

    #[test]
    fn test_type_name_from_url() {
        assert_eq!(
            type_name_from_url("type.googleapis.com/examples.AddItemToCart"),
            "examples.AddItemToCart"
        );
        assert_eq!(type_name_from_url("AddItemToCart"), "AddItemToCart");
    }

    #[test]
    fn test_type_url_matches_exact() {
        assert!(type_url_matches_exact(
            "type.googleapis.com/examples.AddItemToCart",
            "angzarr_client.proto.examples.AddItemToCart"
        ));
        assert!(!type_url_matches_exact(
            "type.googleapis.com/examples.AddItemToCart",
            "angzarr_client.proto.examples.RemoveItem"
        ));
        // Suffix matching should NOT work with exact matching
        assert!(!type_url_matches_exact(
            "type.googleapis.com/examples.AddItemToCart",
            "AddItemToCart"
        ));
    }

    #[test]
    fn type_url_matches_is_alias_for_exact() {
        // Python-canonical name. Must behave identically to type_url_matches_exact.
        assert!(type_url_matches(
            "type.googleapis.com/examples.AddItemToCart",
            "angzarr_client.proto.examples.AddItemToCart"
        ));
        assert!(!type_url_matches(
            "type.googleapis.com/examples.AddItemToCart",
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
