//! Aggregate identity computation for Angzarr domains.
//!
//! Derives deterministic aggregate root UUIDs from business keys so every
//! service and language maps the same `(domain, key)` to the same root.
//! Domain-specific wrappers (`order_root`, `cart_root`, …) belong to the
//! application that owns those domains.

use uuid::Uuid;

/// Compute a deterministic root UUID from domain and business key.
///
/// `uuid5(NAMESPACE_OID, "angzarr" + domain + business_key)`.
pub fn compute_root(domain: &str, business_key: &str) -> Uuid {
    let seed = format!("angzarr{}{}", domain, business_key);
    Uuid::new_v5(&Uuid::NAMESPACE_OID, seed.as_bytes())
}

/// Convert a UUID to its 16-byte proto representation.
pub fn to_proto_bytes(id: Uuid) -> [u8; 16] {
    *id.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_root_matches_python() {
        // Verified byte-equal with Python's:
        //   compute_root("player", "alice@x.com") = 8cf1fb5d-45ce-58c2-a7e4-34359eb42d7c
        assert_eq!(
            compute_root("player", "alice@x.com").to_string(),
            "8cf1fb5d-45ce-58c2-a7e4-34359eb42d7c"
        );
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
