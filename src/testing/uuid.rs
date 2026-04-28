//! Deterministic UUID generation for tests.
//!
//! Mirrors Python's `angzarr_client.testing.uuid` byte-for-byte. The same
//! `name`/`namespace` pair always yields the same UUID.

use uuid::Uuid;

/// Default namespace for test UUIDs. Equals Python's `DEFAULT_TEST_NAMESPACE`
/// (`a1b2c3d4-e5f6-7890-abcd-ef1234567890`).
pub const DEFAULT_TEST_NAMESPACE: Uuid =
    Uuid::from_u128(0xa1b2c3d4_e5f6_7890_abcd_ef1234567890u128);

/// Generate a deterministic 16-byte UUID from a name. Returns raw bytes
/// suitable for use as aggregate root IDs.
pub fn uuid_for(name: &str, namespace: Uuid) -> [u8; 16] {
    *Uuid::new_v5(&namespace, name.as_bytes()).as_bytes()
}

/// Generate a deterministic UUID string from a name.
pub fn uuid_str_for(name: &str, namespace: Uuid) -> String {
    Uuid::new_v5(&namespace, name.as_bytes()).to_string()
}

/// Generate a deterministic `Uuid` from a name.
pub fn uuid_obj_for(name: &str, namespace: Uuid) -> Uuid {
    Uuid::new_v5(&namespace, name.as_bytes())
}

/// Generate a deterministic 16-byte UUID using `DEFAULT_TEST_NAMESPACE`.
///
/// Ergonomic wrapper for the common test case. Mirrors Python's
/// `uuid_for(name)` defaulted-namespace form (`testing/uuid.py:14-32`).
/// Audit finding #50.
pub fn uuid_for_default(name: &str) -> [u8; 16] {
    uuid_for(name, DEFAULT_TEST_NAMESPACE)
}

/// Generate a deterministic UUID string using `DEFAULT_TEST_NAMESPACE`.
pub fn uuid_str_for_default(name: &str) -> String {
    uuid_str_for(name, DEFAULT_TEST_NAMESPACE)
}

/// Generate a deterministic `Uuid` using `DEFAULT_TEST_NAMESPACE`.
pub fn uuid_obj_for_default(name: &str) -> Uuid {
    uuid_obj_for(name, DEFAULT_TEST_NAMESPACE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_for_matches_python() {
        // Python:
        //   uuid_for("test-root", DEFAULT_TEST_NAMESPACE).hex()
        //   = "1596515c79c357278c0e26e3ccf145c6"
        let bytes = uuid_for("test-root", DEFAULT_TEST_NAMESPACE);
        assert_eq!(hex::encode(bytes), "1596515c79c357278c0e26e3ccf145c6");
    }

    #[test]
    fn uuid_for_deterministic() {
        assert_eq!(
            uuid_for("player-alice", DEFAULT_TEST_NAMESPACE),
            uuid_for("player-alice", DEFAULT_TEST_NAMESPACE),
        );
    }

    #[test]
    fn uuid_obj_for_bytes_equal_uuid_for() {
        let obj = uuid_obj_for("x", DEFAULT_TEST_NAMESPACE);
        let bytes = uuid_for("x", DEFAULT_TEST_NAMESPACE);
        assert_eq!(obj.as_bytes(), &bytes);
    }

    #[test]
    fn default_test_namespace_matches_python() {
        assert_eq!(
            DEFAULT_TEST_NAMESPACE.to_string(),
            "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
        );
    }

    #[test]
    fn uuid_for_default_equals_explicit_namespace_call() {
        // Audit finding #50: `uuid_for_default(name)` is the ergonomic
        // wrapper matching Python's `uuid_for(name)` defaulted form.
        assert_eq!(
            uuid_for_default("alice"),
            uuid_for("alice", DEFAULT_TEST_NAMESPACE)
        );
    }

    #[test]
    fn uuid_str_for_default_equals_explicit_namespace_call() {
        assert_eq!(
            uuid_str_for_default("alice"),
            uuid_str_for("alice", DEFAULT_TEST_NAMESPACE)
        );
    }
}
