//! Shared test-spec harness for the client-rust gherkin step defs.
//!
//! Port of `client-python/main/tests/{fixtures.py, client/conftest.py,
//! client/steps/_helpers.py, client/steps/_fakes.py}`. See
//! `client-rust/main/.../plans/...` for the design.

#![allow(dead_code)]

pub mod fakes;
pub mod fixtures;
pub mod helpers;
pub mod world;
