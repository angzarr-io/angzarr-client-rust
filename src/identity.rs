//! Aggregate identity computation for Angzarr domains.
//!
//! Provides deterministic UUID generation from business keys, ensuring
//! consistent aggregate identification across services. Matches the Python
//! `angzarr_client.identity` module byte-for-byte.

use uuid::Uuid;

/// Namespace UUID for generating deterministic inventory product UUIDs.
///
/// Equals `uuid::Uuid::NAMESPACE_DNS` (`6ba7b810-9dad-11d1-80b4-00c04fd430c8`).
/// Matches Python's `INVENTORY_PRODUCT_NAMESPACE`.
pub const INVENTORY_PRODUCT_NAMESPACE: Uuid = Uuid::NAMESPACE_DNS;

/// Compute a deterministic root UUID from domain and business key.
///
/// Mirrors Python's `compute_root`:
/// `uuid5(NAMESPACE_OID, "angzarr" + domain + business_key)`.
pub fn compute_root(domain: &str, business_key: &str) -> Uuid {
    let seed = format!("angzarr{}{}", domain, business_key);
    Uuid::new_v5(&Uuid::NAMESPACE_OID, seed.as_bytes())
}

/// Deterministic UUID for an inventory product aggregate.
pub fn inventory_product_root(product_id: &str) -> Uuid {
    Uuid::new_v5(&INVENTORY_PRODUCT_NAMESPACE, product_id.as_bytes())
}

/// Deterministic root UUID for a customer aggregate.
pub fn customer_root(email: &str) -> Uuid {
    compute_root("customer", email)
}

/// Deterministic root UUID for a product aggregate.
pub fn product_root(sku: &str) -> Uuid {
    compute_root("product", sku)
}

/// Deterministic root UUID for an order aggregate.
pub fn order_root(order_id: &str) -> Uuid {
    compute_root("order", order_id)
}

/// Deterministic root UUID for an inventory aggregate.
pub fn inventory_root(product_id: &str) -> Uuid {
    compute_root("inventory", product_id)
}

/// Deterministic root UUID for a cart aggregate.
pub fn cart_root(customer_id: &str) -> Uuid {
    compute_root("cart", customer_id)
}

/// Deterministic root UUID for a fulfillment aggregate.
pub fn fulfillment_root(order_id: &str) -> Uuid {
    compute_root("fulfillment", order_id)
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
    fn inventory_product_namespace_is_dns() {
        assert_eq!(INVENTORY_PRODUCT_NAMESPACE, Uuid::NAMESPACE_DNS);
    }

    #[test]
    fn domain_helpers_delegate_to_compute_root() {
        assert_eq!(customer_root("e"), compute_root("customer", "e"));
        assert_eq!(product_root("s"), compute_root("product", "s"));
        assert_eq!(order_root("o"), compute_root("order", "o"));
        assert_eq!(inventory_root("p"), compute_root("inventory", "p"));
        assert_eq!(cart_root("c"), compute_root("cart", "c"));
        assert_eq!(fulfillment_root("o"), compute_root("fulfillment", "o"));
    }

    #[test]
    fn to_proto_bytes_returns_16() {
        let id = compute_root("x", "y");
        assert_eq!(to_proto_bytes(id).len(), 16);
    }
}
