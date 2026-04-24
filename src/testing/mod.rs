//! Testing utilities for angzarr applications.
//!
//! Mirrors Python's `angzarr_client.testing` subpackage. Provides deterministic
//! UUID generation, proto-message builders, and a BDD-style `ScenarioContext`.

pub mod builders;
pub mod context;
pub mod uuid;

pub use builders::{
    make_command_book, make_command_page, make_cover, make_event_book, make_event_page,
    make_timestamp, pack_event,
};
pub use context::ScenarioContext;
pub use uuid::{uuid_for, uuid_obj_for, uuid_str_for, DEFAULT_TEST_NAMESPACE};
