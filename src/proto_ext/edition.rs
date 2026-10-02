//! Edition extension trait and constructors.
//!
//! Provides convenience methods for checking timeline status and accessing
//! divergence information.

use crate::proto::Edition;

use super::constants::DEFAULT_EDITION;

/// Extension trait for Edition proto type.
///
/// Provides convenience methods for checking timeline status and accessing
/// divergence information. Constructors remain as associated functions on Edition.
pub trait EditionExt {
    /// Get reference to the edition.
    fn edition_inner(&self) -> &Edition;

    /// Check if this edition has an empty name.
    fn is_empty(&self) -> bool {
        self.edition_inner().name.is_empty()
    }

    /// Check if this is the main timeline (empty or default edition name).
    fn is_main_timeline(&self) -> bool {
        let name = &self.edition_inner().name;
        name.is_empty() || name == DEFAULT_EDITION
    }

    /// Canonical edition name string for cache-key / routing use.
    ///
    /// Returns the edition's name, falling back to [`DEFAULT_EDITION`]
    /// when empty. With `DEFAULT_EDITION = ""`, this is currently a
    /// no-op — but keeping the indirection lets the framework swap in
    /// a non-empty sentinel without touching every call site.
    fn canonical_name(&self) -> &str {
        let edition = self.edition_inner();
        if edition.name.is_empty() {
            DEFAULT_EDITION
        } else {
            &edition.name
        }
    }

    /// Deprecated: use [`canonical_name`](Self::canonical_name).
    /// Kept as an alias to avoid breaking downstream call sites.
    #[deprecated(
        since = "0.6.0",
        note = "use canonical_name() — clearer about what's returned"
    )]
    fn name_or_default(&self) -> &str {
        self.canonical_name()
    }

    /// Get explicit divergence for a specific domain, if any.
    fn divergence_for(&self, domain: &str) -> Option<u32> {
        self.edition_inner()
            .divergences
            .iter()
            .find(|d| d.domain == domain)
            .map(|d| d.sequence)
    }
}

impl EditionExt for Edition {
    fn edition_inner(&self) -> &Edition {
        self
    }
}

/// Constructors for [`Edition`].
pub trait EditionNew: Sized {
    /// The main timeline (empty name).
    fn main_timeline() -> Self;
    /// An edition with implicit divergence (name only).
    fn implicit(name: impl Into<String>) -> Self;
    /// An edition with explicit divergence points.
    fn explicit(name: impl Into<String>, divergences: Vec<crate::proto::DomainDivergence>) -> Self;
}

impl EditionNew for Edition {
    fn main_timeline() -> Self {
        Self {
            name: String::new(),
            divergences: vec![],
        }
    }

    fn implicit(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            divergences: vec![],
        }
    }

    fn explicit(name: impl Into<String>, divergences: Vec<crate::proto::DomainDivergence>) -> Self {
        Self {
            name: name.into(),
            divergences,
        }
    }
}
