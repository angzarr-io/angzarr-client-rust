//! Testing utilities for angzarr applications.
//!
//! Mirrors Python's `angzarr_client.testing` subpackage: deterministic UUID
//! generation, proto-message builders, a BDD-style `ScenarioContext`, and
//! recording fakes of the client traits. Compiled with the `testing` cargo
//! feature; enable it from a consumer's `[dev-dependencies]`:
//!
//! ```toml
//! [dev-dependencies]
//! angzarr-client = { version = "0.5", features = ["testing"] }
//! ```

pub mod builders;
pub mod context;
pub mod fakes;
pub mod uuid;

pub use builders::{
    make_command_book, make_command_page, make_cover, make_event_book, make_event_page,
    make_event_page_at, make_timestamp, pack_event,
};
pub use context::ScenarioContext;
pub use fakes::{
    stub_rpc_error, RecordingGatewayClient, RecordingQueryClient, RecordingSpeculativeClient,
};
pub use uuid::{
    uuid_for, uuid_for_default, uuid_obj_for, uuid_obj_for_default, uuid_str_for,
    uuid_str_for_default, DEFAULT_TEST_NAMESPACE,
};
