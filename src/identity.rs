//! Aggregate identity computation for Angzarr domains.
//!
//! Derives deterministic aggregate root UUIDs (UUIDv5, decision 9.2) from
//! business keys so every service and language maps the same `(domain, key)` to the same root.
//! Domain-specific wrappers (`order_root`, `cart_root`, …) belong to the
//! application that owns those domains.

use uuid::Uuid;

use crate::error::{ClientError, Result};
use crate::error_codes::{codes, keys, messages};

/// The fixed namespace for framework-derived roots (decision 9.2; the
/// `derived_ids` region of `types.proto`): `d4741b1a-52cd-43cf-add1-3dbb9615510b`.
pub const ANGZARR_ROOT_NAMESPACE: Uuid = Uuid::from_u128(0xd4741b1a_52cd_43cf_add1_3dbb9615510b);

/// Compute a deterministic root UUID from domain and business key.
///
/// `UUIDv5(ANGZARR_ROOT_NAMESPACE, "{domain}:{business_key}")` (RFC 9562),
/// the name being the UTF-8 bytes of that text with no normalisation (case,
/// Unicode form and whitespace are hashed as given). A domain never contains
/// `':'`, so the first `':'` splits the name unambiguously whatever the key
/// holds; a domain containing one is refused with
/// [`codes::DOMAIN_CONTAINS_COLON`].
///
/// `compute_root("player", "alice@example.com")` =
/// `cfe936c0-2912-5ca9-96e6-f37c0b441759`.
pub fn compute_root(domain: &str, business_key: &str) -> Result<Uuid> {
    if domain.contains(':') {
        return Err(ClientError::invalid_argument(
            codes::DOMAIN_CONTAINS_COLON,
            messages::DOMAIN_CONTAINS_COLON,
            [(keys::DOMAIN, domain)],
        ));
    }
    let name = format!("{domain}:{business_key}");
    Ok(Uuid::new_v5(&ANGZARR_ROOT_NAMESPACE, name.as_bytes()))
}

/// Convert a UUID to its 16-byte proto representation.
pub fn to_proto_bytes(id: Uuid) -> [u8; 16] {
    *id.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_namespace_is_the_spec_constant() {
        assert_eq!(
            ANGZARR_ROOT_NAMESPACE.to_string(),
            "d4741b1a-52cd-43cf-add1-3dbb9615510b"
        );
    }

    /// The worked vector of decision 9.2 / the `derived_ids` region of
    /// `types.proto`.
    #[test]
    fn compute_root_matches_the_spec_worked_vector() {
        assert_eq!(
            compute_root("player", "alice@example.com")
                .unwrap()
                .to_string(),
            "cfe936c0-2912-5ca9-96e6-f37c0b441759"
        );
    }

    #[test]
    fn compute_root_is_uuid5_of_domain_colon_key_under_the_root_namespace() {
        assert_eq!(
            compute_root("cart", "alice").unwrap(),
            Uuid::new_v5(&ANGZARR_ROOT_NAMESPACE, b"cart:alice")
        );
    }

    #[test]
    fn compute_root_refuses_a_domain_containing_a_colon() {
        let err = compute_root("bad:domain", "k").unwrap_err();
        assert!(err.is_invalid_argument());
        assert_eq!(err.code(), codes::DOMAIN_CONTAINS_COLON);
        match err {
            ClientError::InvalidArgument(d) => {
                assert_eq!(
                    d.details.get(keys::DOMAIN).map(String::as_str),
                    Some("bad:domain")
                );
            }
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    #[test]
    fn compute_root_accepts_a_colon_in_the_key() {
        assert_eq!(
            compute_root("cart", "a:b").unwrap(),
            Uuid::new_v5(&ANGZARR_ROOT_NAMESPACE, b"cart:a:b")
        );
    }

    #[test]
    fn compute_root_does_not_normalise_case() {
        assert_ne!(
            compute_root("player", "Alice@example.com").unwrap(),
            compute_root("player", "alice@example.com").unwrap()
        );
    }

    #[test]
    fn compute_root_does_not_normalise_unicode_or_whitespace() {
        // U+00E9 (NFC) vs "e" + U+0301 (NFD): hashed as their own UTF-8 bytes.
        let nfc = "caf\u{e9}";
        let nfd = "cafe\u{301}";
        assert_ne!(
            compute_root("shop", nfc).unwrap(),
            compute_root("shop", nfd).unwrap()
        );
        assert_eq!(
            compute_root("shop", nfd).unwrap(),
            Uuid::new_v5(&ANGZARR_ROOT_NAMESPACE, "shop:cafe\u{301}".as_bytes())
        );
        assert_ne!(
            compute_root("shop", " x").unwrap(),
            compute_root("shop", "x").unwrap()
        );
    }

    #[test]
    fn compute_root_keeps_the_domain_key_boundary() {
        assert_ne!(
            compute_root("ab", "c").unwrap(),
            compute_root("a", "bc").unwrap()
        );
    }

    #[test]
    fn compute_root_deterministic() {
        let a = compute_root("order", "o-1").unwrap();
        let b = compute_root("order", "o-1").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn compute_root_varies_by_domain() {
        let a = compute_root("customer", "x").unwrap();
        let b = compute_root("product", "x").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn to_proto_bytes_is_the_uuid_bytes() {
        let id = compute_root("x", "y").unwrap();
        assert_eq!(to_proto_bytes(id), *id.as_bytes());
    }
}
