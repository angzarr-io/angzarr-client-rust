//! Cover extension trait and implementations.
//!
//! Provides convenient accessors for domain, correlation_id, and root_id
//! from Cover-bearing types.

use crate::proto::{CommandBook, Cover, EventBook, Query};

use super::constants::{DEFAULT_EDITION, UNKNOWN_DOMAIN};

/// Extension trait for types with an optional Cover.
///
/// Provides convenient accessors for domain, correlation_id, and root_id
/// without verbose `.cover.as_ref().map(...)` chains.
pub trait CoverExt {
    /// Get the cover, if present.
    fn cover(&self) -> Option<&Cover>;

    /// Get the domain from the cover, or [`UNKNOWN_DOMAIN`] if missing
    /// or empty.
    ///
    /// Audit finding #54: empty-domain covers (partially-constructed
    /// Cover during testing, malformed wire input, etc.) are treated as
    /// missing. Mirrors Python's `helpers.py::domain` which falls back
    /// for either `c is None` OR `not c.domain`. Postel's Law:
    /// normalize ambiguous-shaped data.
    fn domain(&self) -> &str {
        self.cover()
            .map(|c| c.domain.as_str())
            .filter(|d| !d.is_empty())
            .unwrap_or(UNKNOWN_DOMAIN)
    }

    /// Get the correlation_id from the cover, or empty string if missing.
    fn correlation_id(&self) -> &str {
        self.cover()
            .map(|c| c.correlation_id.as_str())
            .unwrap_or("")
    }

    /// Get the root UUID as a hex-encoded string, if present.
    fn root_id_hex(&self) -> Option<String> {
        self.cover()
            .and_then(|c| c.root.as_ref())
            .map(|u| hex::encode(&u.value))
    }

    /// Get the root UUID, if present.
    fn root_uuid(&self) -> Option<uuid::Uuid> {
        self.cover()
            .and_then(|c| c.root.as_ref())
            .and_then(|u| uuid::Uuid::from_slice(&u.value).ok())
    }

    /// Check if correlation_id is present and non-empty.
    fn has_correlation_id(&self) -> bool {
        !self.correlation_id().is_empty()
    }

    /// Get the edition name from the cover.
    ///
    /// Returns the explicit edition name if set and non-empty, otherwise
    /// `DEFAULT_EDITION` (currently `""` — the canonical empty marker
    /// used in cache keys; matches Python `helpers.cache_key`'s
    /// `edition or ''` formula).
    fn edition(&self) -> &str {
        self.cover()
            .and_then(|c| c.edition.as_ref())
            .map(|e| e.name.as_str())
            .filter(|e| !e.is_empty())
            .unwrap_or(DEFAULT_EDITION)
    }

    /// Compute the bus routing key: `"{domain}"`.
    ///
    /// The routing key is a transport concern used for bus subscription matching.
    /// Edition filtering is handled at the handler level, not the bus level.
    fn routing_key(&self) -> String {
        self.domain().to_string()
    }

    /// Generate a cache key for this entity based on edition + domain + root.
    ///
    /// Used for caching aggregate state during saga retry to avoid redundant fetches.
    /// Includes edition to prevent collision between aggregates in different timelines.
    fn cache_key(&self) -> String {
        let edition = self.edition();
        let domain = self.domain();
        let root = self.root_id_hex().unwrap_or_default();
        format!("{edition}:{domain}:{root}")
    }
}

impl CoverExt for EventBook {
    fn cover(&self) -> Option<&Cover> {
        self.cover.as_ref()
    }
}

impl CoverExt for CommandBook {
    fn cover(&self) -> Option<&Cover> {
        self.cover.as_ref()
    }
}

impl CoverExt for Query {
    fn cover(&self) -> Option<&Cover> {
        self.cover.as_ref()
    }
}

impl CoverExt for Cover {
    fn cover(&self) -> Option<&Cover> {
        Some(self)
    }
}

impl Cover {
    /// Audit #86: copy the `source` cover's edition (full struct,
    /// including divergences) onto this cover, overwriting whatever
    /// was here. **Always-override semantics:** the framework
    /// guarantees timeline consistency on saga / PM cross-domain
    /// emissions; handlers cannot escape into a different edition by
    /// setting their own outgoing cover. Cross-timeline emission
    /// would need a separate fork-to-timeline mechanism.
    pub fn propagate_edition_from(&mut self, source: &Cover) {
        self.edition = source.edition.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::EventBook;

    // Audit #54: domain() falls back to UNKNOWN_DOMAIN for both
    // cover-missing and cover-with-empty-domain cases.

    #[test]
    fn domain_falls_back_when_cover_missing() {
        let book = EventBook {
            cover: None,
            ..Default::default()
        };
        assert_eq!(book.domain(), UNKNOWN_DOMAIN);
    }

    #[test]
    fn domain_falls_back_when_domain_is_empty() {
        let book = EventBook {
            cover: Some(Cover {
                domain: String::new(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(book.domain(), UNKNOWN_DOMAIN);
    }

    #[test]
    fn domain_returns_set_value() {
        let book = EventBook {
            cover: Some(Cover {
                domain: "order".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(book.domain(), "order");
    }

    // Audit #86: `propagate_edition_from` always overrides outgoing
    // edition with the source's full Edition struct (name + divergences).

    use crate::proto::{DomainDivergence, Edition};

    #[test]
    fn propagate_edition_copies_name_when_outgoing_unset() {
        let source = Cover {
            edition: Some(Edition {
                name: "speculative".to_string(),
                divergences: vec![],
            }),
            ..Default::default()
        };
        let mut outgoing = Cover {
            edition: None,
            ..Default::default()
        };
        outgoing.propagate_edition_from(&source);
        assert_eq!(
            outgoing.edition.as_ref().map(|e| e.name.as_str()),
            Some("speculative"),
        );
    }

    #[test]
    fn propagate_edition_overrides_handler_set_edition() {
        let source = Cover {
            edition: Some(Edition {
                name: "alpha".to_string(),
                divergences: vec![],
            }),
            ..Default::default()
        };
        let mut outgoing = Cover {
            edition: Some(Edition {
                name: "beta".to_string(),
                divergences: vec![],
            }),
            ..Default::default()
        };
        outgoing.propagate_edition_from(&source);
        assert_eq!(
            outgoing.edition.as_ref().map(|e| e.name.as_str()),
            Some("alpha"),
            "always-override semantics: source wins",
        );
    }

    #[test]
    fn propagate_edition_clears_when_source_unset() {
        let source = Cover {
            edition: None,
            ..Default::default()
        };
        let mut outgoing = Cover {
            edition: Some(Edition {
                name: "leftover".to_string(),
                divergences: vec![],
            }),
            ..Default::default()
        };
        outgoing.propagate_edition_from(&source);
        assert!(
            outgoing.edition.is_none(),
            "source had no edition → outgoing must match (cleared)",
        );
    }

    #[test]
    fn propagate_edition_preserves_divergences() {
        let source = Cover {
            edition: Some(Edition {
                name: "speculative".to_string(),
                divergences: vec![DomainDivergence {
                    domain: "order".to_string(),
                    sequence: 5,
                }],
            }),
            ..Default::default()
        };
        let mut outgoing = Cover {
            edition: None,
            ..Default::default()
        };
        outgoing.propagate_edition_from(&source);
        let edition = outgoing.edition.as_ref().expect("edition stamped");
        assert_eq!(edition.name, "speculative");
        assert_eq!(edition.divergences.len(), 1);
        assert_eq!(edition.divergences[0].domain, "order");
        assert_eq!(edition.divergences[0].sequence, 5);
    }
}
