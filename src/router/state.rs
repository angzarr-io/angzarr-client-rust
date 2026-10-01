//! Output-domain bookkeeping for sagas and process managers.
//!
//! A saga or process manager declares the domains it issues commands to.
//! [`Destinations`] holds that declaration and turns an emitted command into
//! a deferred one: every page header becomes `angzarr_deferred` carrying the
//! triggering event's provenance. Deferred commands have no expected version;
//! the destination appends them at its head.

use crate::error::{ClientError, Result};
use crate::error_codes::{codes, keys, messages};
use crate::proto::{
    page_header::SequenceType, AngzarrDeferredSequence, CommandBook, Cover, PageHeader,
};

/// The output domains a saga / process manager declares, in declaration
/// order.
///
/// # Example
///
/// ```rust,ignore
/// let destinations = Destinations::new(["inventory", "shipping"]);
/// destinations.stamp_command(&mut cmd, "inventory", &source_cover, source_seq, 0)?;
/// ```
#[derive(Debug, Default, Clone)]
pub struct Destinations {
    domains: Vec<String>,
}

impl Destinations {
    /// Destinations for the given declared output domains. Duplicates are
    /// kept once, at their first position.
    pub fn new<I, S>(domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut out: Vec<String> = Vec::new();
        for d in domains {
            let d = d.into();
            if !out.contains(&d) {
                out.push(d);
            }
        }
        Self { domains: out }
    }

    /// A `PageHeader` carrying `angzarr_deferred` provenance: the
    /// triggering event's cover and sequence and the command's position in
    /// the invocation's output. `source_component` is left for the
    /// coordinator to stamp.
    pub fn deferred_header(source_cover: Cover, source_seq: u32, command_index: u32) -> PageHeader {
        PageHeader {
            sequence_type: Some(SequenceType::AngzarrDeferred(AngzarrDeferredSequence {
                source: Some(source_cover),
                source_seq,
                command_index,
                ..Default::default()
            })),
            sync_mode: None,
        }
    }

    /// Make `cmd` a deferred command for `domain`: every page header becomes
    /// [`Self::deferred_header`] (any explicit sequence is replaced; a
    /// page's `sync_mode` is kept).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` with code `UNDECLARED_OUTPUT_DOMAIN` and
    /// `details["domain"]` when `domain` is not a declared output domain.
    pub fn stamp_command(
        &self,
        cmd: &mut CommandBook,
        domain: &str,
        source_cover: &Cover,
        source_seq: u32,
        command_index: u32,
    ) -> Result<()> {
        if !self.has_domain(domain) {
            return Err(ClientError::invalid_argument(
                codes::UNDECLARED_OUTPUT_DOMAIN,
                messages::UNDECLARED_OUTPUT_DOMAIN,
                [(keys::DOMAIN, domain.to_string())],
            ));
        }
        for page in &mut cmd.pages {
            let sync_mode = page.header.as_ref().and_then(|h| h.sync_mode);
            let mut header = Self::deferred_header(source_cover.clone(), source_seq, command_index);
            header.sync_mode = sync_mode;
            page.header = Some(header);
        }
        Ok(())
    }

    /// True when `domain` is a declared output domain.
    pub fn has_domain(&self, domain: &str) -> bool {
        self.domains.iter().any(|d| d == domain)
    }

    /// The declared output domains, in declaration order.
    pub fn domains(&self) -> impl Iterator<Item = &str> {
        self.domains.iter().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{command_page::Payload as CmdPayload, CommandPage, Uuid as ProtoUuid};
    use prost::Message;

    fn source() -> Cover {
        Cover {
            domain: "order".into(),
            root: Some(ProtoUuid {
                value: (0u8..16).collect(),
            }),
            correlation_id: "corr-1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn domains_keep_declaration_order_without_duplicates() {
        let d = Destinations::new(["zulu", "alpha", "zulu", "mike"]);
        assert_eq!(
            d.domains().collect::<Vec<_>>(),
            vec!["zulu", "alpha", "mike"]
        );
        assert!(d.has_domain("alpha"));
        assert!(!d.has_domain("shipping"));
        assert!(!d.has_domain(""));
    }

    #[test]
    fn stamp_command_makes_every_page_deferred() {
        let d = Destinations::new(["inventory"]);
        let mut cmd = CommandBook {
            pages: vec![
                CommandPage {
                    header: Some(PageHeader {
                        sequence_type: Some(SequenceType::Sequence(9)),
                        sync_mode: Some(crate::proto::SyncMode::Simple as i32),
                    }),
                    ..Default::default()
                },
                CommandPage::default(),
            ],
            ..Default::default()
        };
        d.stamp_command(&mut cmd, "inventory", &source(), 3, 1)
            .unwrap();
        for (i, page) in cmd.pages.iter().enumerate() {
            let header = page.header.as_ref().unwrap();
            let Some(SequenceType::AngzarrDeferred(def)) = &header.sequence_type else {
                panic!("page {i} not deferred: {header:?}");
            };
            assert_eq!(def.source.as_ref(), Some(&source()));
            assert_eq!(def.source_seq, 3);
            assert_eq!(def.command_index, 1);
            assert!(def.source_component.is_empty());
        }
        assert_eq!(
            cmd.pages[0].header.as_ref().unwrap().sync_mode,
            Some(crate::proto::SyncMode::Simple as i32)
        );
        assert_eq!(cmd.pages[1].header.as_ref().unwrap().sync_mode, None);
    }

    #[test]
    fn stamp_command_rejects_an_undeclared_domain() {
        let d = Destinations::new(["inventory"]);
        let mut cmd = CommandBook {
            pages: vec![CommandPage::default()],
            ..Default::default()
        };
        let err = d
            .stamp_command(&mut cmd, "shipping", &source(), 0, 0)
            .expect_err("undeclared domain");
        assert_eq!(err.code(), codes::UNDECLARED_OUTPUT_DOMAIN);
        let ClientError::InvalidArgument(detail) = &err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert_eq!(detail.details[keys::DOMAIN], "shipping");
        assert_eq!(cmd.pages[0].header, None, "nothing stamped on error");
    }

    /// Same fixture and golden as parity/client/wire_parity.feature C-0182.
    #[test]
    fn stamp_command_wire_parity() {
        use sha2::{Digest, Sha256};
        let mut book = CommandBook {
            cover: Some(Cover {
                domain: "inventory".into(),
                root: Some(ProtoUuid {
                    value: (0x10u8..=0x1f).collect(),
                }),
                correlation_id: "corr-1".into(),
                ..Default::default()
            }),
            pages: vec![CommandPage {
                payload: Some(CmdPayload::Command(prost_types::Any {
                    type_url: "/example.Foo".into(),
                    value: vec![1, 2, 3, 4],
                })),
                ..Default::default()
            }],
        };
        Destinations::new(["inventory"])
            .stamp_command(&mut book, "inventory", &source(), 3, 0)
            .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(book.encode_to_vec())),
            "10b1ce23a470f107662591a7da41830c724fdc0c9562824130f09b0a12f011f5"
        );
    }
}
