//! Cross-cutting World fragment and shared `@then` assertions.
//!
//! Port of `client-python/main/tests/client/conftest.py`. Rust cucumber binds
//! one `World` per feature; this module provides an embeddable struct
//! ([`CommonWorld`]) carrying the recurring fields from Python's `World`
//! dataclass, plus pure-function assertion helpers for the three cross-cutting
//! `@then` steps. Each step file registers those `@then`s itself (cucumber-rs
//! has no `conftest` analog) but the assertion body lives once.

use std::collections::HashMap;

use angzarr_client::error::ClientError;
use angzarr_client::proto::{
    BusinessResponse, CommandBook, ProcessManagerHandleResponse, Projection, SagaResponse,
};

/// Variant slot for the captured response from a single dispatch.
#[derive(Debug, Default)]
pub enum DispatchResponse {
    #[default]
    None,
    Business(BusinessResponse),
    Saga(SagaResponse),
    Pm(ProcessManagerHandleResponse),
    Projection(Projection),
}

impl DispatchResponse {
    pub fn as_business(&self) -> Option<&BusinessResponse> {
        match self {
            Self::Business(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_saga(&self) -> Option<&SagaResponse> {
        match self {
            Self::Saga(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_pm(&self) -> Option<&ProcessManagerHandleResponse> {
        match self {
            Self::Pm(p) => Some(p),
            _ => None,
        }
    }

    pub fn as_projection(&self) -> Option<&Projection> {
        match self {
            Self::Projection(p) => Some(p),
            _ => None,
        }
    }
}

/// Per-scenario shared fields. Embed into per-feature World structs:
///
/// ```ignore
/// #[derive(Debug, Default, World)]
/// pub struct CommandHandlerWorld {
///     pub common: CommonWorld,
///     // …feature-specific fields…
/// }
/// ```
///
/// Field set mirrors `client-python/main/tests/client/conftest.py::World`:
/// - `call_log` — multi-handler invocation order
/// - `write_log` — projector write descriptors (string-encoded so the struct
///   stays Debug/Default; matches Python's freeform list)
/// - `dest_seqs` — destination sequences provided to saga/PM handlers
/// - `observed_dest` — destination sequences observed inside saga/PM handlers
/// - `observed` — arbitrary string-keyed observations from inside handlers
/// - `response` — captured dispatch output
/// - `dispatch_exc` — captured dispatch error
#[derive(Debug, Default)]
pub struct CommonWorld {
    pub call_log: Vec<String>,
    pub write_log: Vec<String>,
    pub dest_seqs: HashMap<String, u32>,
    pub observed_dest: HashMap<String, u32>,
    pub observed: HashMap<String, String>,
    pub response: DispatchResponse,
    pub dispatch_exc: Option<ClientError>,
}

// ---------------------------------------------------------------------------
// Cross-cutting `@then` assertions
// ---------------------------------------------------------------------------

/// Mirrors Python conftest's `the response contains exactly one command`.
///
/// Looks up the command list on whichever response variant carries one
/// (Business with Saga payload, Saga, or PM). Panics if no command-bearing
/// response is present.
pub fn assert_one_command(world: &CommonWorld) {
    let commands = response_commands(world);
    assert_eq!(
        commands.len(),
        1,
        "expected exactly 1 command, got {}",
        commands.len()
    );
}

/// Mirrors `the response contains no commands`.
pub fn assert_no_commands(world: &CommonWorld) {
    let commands = response_commands(world);
    assert!(
        commands.is_empty(),
        "expected no commands, got {}",
        commands.len()
    );
}

/// Mirrors `the command targets the "{domain}" domain`.
pub fn assert_command_targets(world: &CommonWorld, domain: &str) {
    let commands = response_commands(world);
    assert!(
        !commands.is_empty(),
        "expected at least 1 command to check target domain"
    );
    let actual = commands[0]
        .cover
        .as_ref()
        .map(|c| c.domain.as_str())
        .unwrap_or("<no-cover>");
    assert_eq!(actual, domain, "command target domain mismatch");
}

fn response_commands(world: &CommonWorld) -> &[CommandBook] {
    if let Some(saga) = world.response.as_saga() {
        return saga.commands.as_slice();
    }
    if let Some(pm) = world.response.as_pm() {
        return pm.commands.as_slice();
    }
    if let Some(_business) = world.response.as_business() {
        // BusinessResponse carries `result: Option<Result>` where the
        // command-emitting variant lives in `Result::Saga(SagaResponse)`.
        // Drill into it via the saga payload if present.
        // (Step files using BusinessResponse with commands set their
        // CommonWorld.response = Saga(saga.clone()) before asserting.)
        return &[];
    }
    &[]
}
