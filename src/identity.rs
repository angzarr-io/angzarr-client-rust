//! Aggregate identity computation for Angzarr domains.
//!
//! Derives deterministic aggregate root UUIDs from business keys so every
//! service and language maps the same `(domain, key)` to the same root.
//! Domain-specific wrappers (`order_root`, `cart_root`, …) belong to the
//! application that owns those domains.

use uuid::Uuid;

/// Compute a deterministic root UUID from domain and business key.
///
/// `uuid5(NAMESPACE_OID, domain + ":" + business_key)`. Domain names never
/// contain `':'`, so every `(domain, key)` pair hashes a distinct name.
pub fn compute_root(domain: &str, business_key: &str) -> Uuid {
    let name = format!("{domain}:{business_key}");
    Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes())
}

/// Convert a UUID to its 16-byte proto representation.
pub fn to_proto_bytes(id: Uuid) -> [u8; 16] {
    *id.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_root_is_uuid5_of_domain_colon_key() {
        assert_eq!(
            compute_root("cart", "alice"),
            Uuid::new_v5(&Uuid::NAMESPACE_OID, b"cart:alice")
        );
        assert_eq!(
            compute_root("cart", "alice").to_string(),
            "92ab9191-4f7d-5ff2-adcf-93a1e494c055"
        );
    }

    #[test]
    fn compute_root_keeps_the_domain_key_boundary() {
        assert_ne!(compute_root("ab", "c"), compute_root("a", "bc"));
    }

    #[test]
    fn compute_root_deterministic() {
        let a = compute_root("order", "o-1");
        let b = compute_root("order", "o-1");
        assert_eq!(a, b);
    }

    #[test]
    fn compute_root_varies_by_domain() {
        let a = compute_root("customer", "x");
        let b = compute_root("product", "x");
        assert_ne!(a, b);
    }

    #[test]
    fn to_proto_bytes_is_the_uuid_bytes() {
        let id = compute_root("x", "y");
        assert_eq!(to_proto_bytes(id), *id.as_bytes());
    }
}
