//! Integration tests for the Tier 5 unified `Router`.
//!
//! Runtime behavior (builder, dispatch, state rebuild, multi-handler, sequence,
//! rejection) is covered by the gherkin acceptance tier in `tests/features.rs`
//! against `angzarr-project/features/client/*.feature`. The modules below cover
//! what gherkin cannot:
//!
//! - `decorators` — proc-macro expansion metadata (declaration order, config
//!   shape). Language-specific; not a cross-language contract.
//! - `stacking` — compile-fail assertions for kind-macro stacking and
//!   non-`Handler` `with_handler` arguments. trybuild-driven.
//! - `validation` — compile-fail assertions for macros missing required
//!   attributes. trybuild-driven.

#[path = "router/decorators.rs"]
mod decorators;

#[path = "router/stacking.rs"]
mod stacking;

#[path = "router/validation.rs"]
mod validation;
