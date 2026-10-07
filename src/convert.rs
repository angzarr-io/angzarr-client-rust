//! Conversion helpers for protobuf types.

use crate::error::{ClientError, Result};
use crate::error_codes::{codes, keys, messages};
use crate::proto::Uuid as ProtoUuid;
use chrono::{DateTime, Utc};
use prost::Name;
use prost_types::{Any, Timestamp};
use uuid::Uuid;

// Canonical constants live in `proto_ext::constants`; re-exported here
// for callers that import from the `convert::` namespace.
pub use crate::proto_ext::constants::{
    DEFAULT_EDITION, META_ANGZARR_DOMAIN, PROJECTION_DOMAIN_PREFIX, PROJECTION_TYPE_URL,
    TYPE_URL_PREFIX, UNKNOWN_DOMAIN, WILDCARD_DOMAIN,
};

/// The type URL emitted for a fully-qualified proto type name:
/// [`TYPE_URL_PREFIX`] (`/`) + the name.
///
/// # Examples
/// ```
/// use angzarr_client::convert::type_url;
/// assert_eq!(type_url("orders.OrderCreated"), "/orders.OrderCreated");
/// ```
pub fn type_url(type_name: &str) -> String {
    format!("{}{}", TYPE_URL_PREFIX, type_name)
}

/// Extract the wire-format type name from a type URL.
///
/// Returns the part after the last `/` (e.g., "examples.PlayerRegistered").
pub fn type_name_from_url(type_url: &str) -> &str {
    type_url.rsplit('/').next().unwrap_or(type_url)
}

/// True when `type_url` names `full_type_name`: whatever its prefix, the
/// text after the last `/` equals the fully-qualified name exactly (no
/// suffix matching).
///
/// # Examples
/// ```
/// use angzarr_client::convert::type_url_matches_exact;
/// assert!(type_url_matches_exact("/orders.OrderCreated", "orders.OrderCreated"));
/// assert!(type_url_matches_exact(
///     "type.googleapis.com/orders.OrderCreated",
///     "orders.OrderCreated"
/// ));
/// assert!(!type_url_matches_exact("/orders.OrderCreated", "OrderCreated"));
/// ```
pub fn type_url_matches_exact(type_url: &str, full_type_name: &str) -> bool {
    type_name_from_url(type_url) == full_type_name
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
    type_url_is::<T>(&any.type_url)
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
    if !type_url_is::<T>(&any.type_url) {
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
/// assert_eq!(full_type_url::<PlayerRegistered>(), "/examples.PlayerRegistered");
/// ```
pub fn full_type_url<T: Name>() -> String {
    format!("{}{}", TYPE_URL_PREFIX, T::full_name())
}

/// True when `type_url` names message type `T` (any prefix; the full name
/// after the last `/` must equal `T::full_name()`).
pub fn type_url_is<T: Name>(type_url: &str) -> bool {
    type_name_from_url(type_url) == T::full_name()
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
///
/// Validates byte-shape only (`Uuid::from_slice` rejects `len != 16`);
/// does **not** verify RFC 4122 variant/version bits. Matches Python's
/// permissive `uuid.UUID(bytes=...)` for cross-language parity — a
/// payload of 16 bytes that violates RFC 4122 produces a `Uuid` whose
/// methods may surprise (e.g., `get_version()` returns `None`). Callers
/// that need semantic validation should check `uuid.get_version()`
/// after this returns.
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
/// Per `google.protobuf.Timestamp`, `nanos` must be in `[0, 999_999_999]`
/// and `seconds` may be negative for instants before the Unix epoch. This
/// function preserves both invariants — `chrono::DateTime::timestamp()`
/// returns the right `seconds` for negative values, and `timestamp_subsec_nanos()`
/// is documented to be in the valid range; the assertion here is a guard
/// against a future chrono regression rather than a runtime check the
/// caller should rely on.
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

    let nanos = dt.timestamp_subsec_nanos();
    debug_assert!(
        nanos < 1_000_000_000,
        "chrono nanos out of range: {}",
        nanos
    );
    Ok(Timestamp {
        seconds: dt.timestamp(),
        nanos: nanos as i32,
    })
}

/// Get the current time as a protobuf Timestamp.
///
/// Saturates to the Unix epoch (`Timestamp { seconds: 0, nanos: 0 }`)
/// rather than panicking when the system clock is before 1970. This is
/// theoretically impossible on a sane system, but a stuck/skewed clock
/// during test runs shouldn't kill the process — the caller's clock
/// expectations decide whether the saturated value is acceptable.
pub fn now() -> Timestamp {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => Timestamp {
            seconds: d.as_secs() as i64,
            nanos: d.subsec_nanos() as i32,
        },
        Err(_) => Timestamp {
            seconds: 0,
            nanos: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_urls_are_emitted_with_the_bare_slash_prefix() {
        assert_eq!(TYPE_URL_PREFIX, "/");
        assert_eq!(type_url("orders.OrderCreated"), "/orders.OrderCreated");
        assert_eq!(
            full_type_url::<crate::proto::Cover>(),
            "/io.angzarr.v1.Cover"
        );
    }

    #[test]
    fn test_type_name_from_url() {
        assert_eq!(
            type_name_from_url("type.googleapis.com/orders.OrderCreated"),
            "orders.OrderCreated"
        );
        assert_eq!(
            type_name_from_url("/orders.OrderCreated"),
            "orders.OrderCreated"
        );
        assert_eq!(
            type_name_from_url("orders.OrderCreated"),
            "orders.OrderCreated"
        );
    }

    /// Any prefix is accepted; the full name after the last "/" is compared
    /// exactly.
    #[test]
    fn type_urls_match_by_full_name_whatever_the_prefix() {
        for url in [
            "/myapp.events.v1.OrderCreated",
            "type.googleapis.com/myapp.events.v1.OrderCreated",
            "example.com/types/myapp.events.v1.OrderCreated",
            "myapp.events.v1.OrderCreated",
        ] {
            assert!(
                type_url_matches(url, "myapp.events.v1.OrderCreated"),
                "{url}"
            );
            assert!(
                type_url_matches_exact(url, "myapp.events.v1.OrderCreated"),
                "{url}"
            );
        }
        assert!(!type_url_matches(
            "/myapp.events.v2.OrderCreated",
            "myapp.events.v1.OrderCreated"
        ));
        assert!(!type_url_matches(
            "/myapp.events.v1.OrderCreated",
            "OrderCreated"
        ));
        assert!(!type_url_matches(
            "/myapp.events.v1.OrderCreated",
            "v1.OrderCreated"
        ));
    }

    #[test]
    fn typed_matching_accepts_any_prefix() {
        let bytes = prost::Message::encode_to_vec(&crate::proto::Cover {
            domain: "d".into(),
            ..Default::default()
        });
        for url in [
            "/io.angzarr.v1.Cover",
            "type.googleapis.com/io.angzarr.v1.Cover",
        ] {
            assert!(type_url_is::<crate::proto::Cover>(url));
            let any = Any {
                type_url: url.into(),
                value: bytes.clone(),
            };
            assert!(type_matches::<crate::proto::Cover>(&any));
            assert_eq!(try_unpack::<crate::proto::Cover>(&any).unwrap().domain, "d");
            assert_eq!(unpack::<crate::proto::Cover>(&any).unwrap().domain, "d");
        }
        assert!(!type_url_is::<crate::proto::Cover>("/io.angzarr.v1.Covers"));
        let other = Any {
            type_url: "/io.angzarr.v2.Cover".into(),
            value: bytes,
        };
        assert!(try_unpack::<crate::proto::Cover>(&other).is_none());
        assert_eq!(
            unpack::<crate::proto::Cover>(&other).unwrap_err().code(),
            codes::ANY_TYPE_MISMATCH
        );
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

    #[test]
    fn test_parse_timestamp_pre_unix_epoch() {
        // protobuf Timestamp permits negative `seconds`; chrono returns
        // the right value for instants before 1970 and `nanos` stays
        // in [0, 1e9). Pin both invariants here so a future chrono
        // change can't silently break wire compat.
        let ts = parse_timestamp("1969-12-31T23:59:59.500Z").unwrap();
        assert_eq!(ts.seconds, -1);
        assert_eq!(ts.nanos, 500_000_000);
        assert!(ts.nanos >= 0 && ts.nanos < 1_000_000_000);
    }

    #[test]
    fn test_now_returns_valid_protobuf_timestamp() {
        // Saturation path is only taken when the system clock is
        // before the Unix epoch — a failure mode we'd rather log than
        // panic on. Document the steady-state shape: positive seconds,
        // nanos in [0, 1e9).
        let ts = now();
        assert!(ts.seconds >= 0);
        assert!(ts.nanos >= 0 && ts.nanos < 1_000_000_000);
    }
}
