//! UUID conversion traits.
//!
//! Provides bidirectional conversion between proto UUID and standard UUID types.

use crate::proto::Uuid as ProtoUuid;

/// Extension trait for ProtoUuid proto type.
///
/// Provides conversion methods to standard Uuid types.
pub trait ProtoUuidExt {
    /// Convert to a standard UUID.
    fn to_uuid(&self) -> Result<uuid::Uuid, uuid::Error>;

    /// Convert to a hex-encoded string (dashless, 32 chars for a v4/v5 UUID).
    fn to_hex(&self) -> String;

    /// Convert to standard UUID text format (8-4-4-4-12 dashed, e.g.
    /// `"6ba7b810-9dad-11d1-80b4-00c04fd430c8"`) if the bytes are exactly
    /// 16 bytes; otherwise falls back to dashless hex.
    ///
    /// Mirrors Python's `proto_uuid_to_text(u)` (`helpers.py:134-138`).
    /// Audit finding #48.
    fn to_uuid_text(&self) -> String;
}

impl ProtoUuidExt for ProtoUuid {
    fn to_uuid(&self) -> Result<uuid::Uuid, uuid::Error> {
        uuid::Uuid::from_slice(&self.value)
    }

    fn to_hex(&self) -> String {
        hex::encode(&self.value)
    }

    fn to_uuid_text(&self) -> String {
        match self.to_uuid() {
            Ok(u) => u.to_string(),
            Err(_) => self.to_hex(),
        }
    }
}

/// Extension trait for uuid::Uuid to convert to proto types.
pub trait UuidExt {
    /// Convert to a ProtoUuid.
    fn to_proto_uuid(&self) -> ProtoUuid;
}

impl UuidExt for uuid::Uuid {
    fn to_proto_uuid(&self) -> ProtoUuid {
        ProtoUuid {
            value: self.as_bytes().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_uuid_text_emits_dashed_form_for_16_bytes() {
        // Audit finding #48: cross-language convention is dashed 8-4-4-4-12.
        // Equivalent to Python's `bytes_to_uuid_text(b)` for 16-byte input.
        let bytes = [
            0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4,
            0x30, 0xc8,
        ];
        let proto = ProtoUuid {
            value: bytes.to_vec(),
        };
        assert_eq!(
            proto.to_uuid_text(),
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8"
        );
    }

    #[test]
    fn to_uuid_text_falls_back_to_hex_for_non_16_bytes() {
        // Mirrors Python's `bytes_to_uuid_text(b)` non-16-byte branch
        // (`helpers.py:129-131`): non-canonical inputs return dashless hex.
        let proto = ProtoUuid {
            value: vec![0xAB, 0xCD, 0xEF],
        };
        assert_eq!(proto.to_uuid_text(), "abcdef");
    }

    #[test]
    fn to_hex_remains_dashless() {
        // Existing API contract: `to_hex` is the dashless form. Audit
        // finding #48 added `to_uuid_text` for the dashed form; pinning
        // here to ensure they don't collapse into one.
        let proto = ProtoUuid {
            value: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        assert_eq!(proto.to_hex(), "deadbeef");
    }
}
