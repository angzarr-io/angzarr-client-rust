//! Shared test-spec harness for the client-rust gherkin step defs.
//!
//! The in-process coordinator backend and the fixture messages the feature
//! files name. Proto builders and recording client fakes come from
//! `angzarr_client::testing`.

#![allow(dead_code)]

pub mod backend;
pub mod fixtures;
