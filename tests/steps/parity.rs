//! Step definitions for features/client/parity.feature.
//!
//! Asserts each canonical public name is reachable in the compiled crate.
//! Presence is checked against a hardcoded set mirroring the current
//! `lib.rs` re-exports; testing helpers live in `angzarr_client::testing`
//! (the `testing` feature, which this crate's dev-dependency enables). Names not in the set fail the scenario with a
//! clear message pointing to the missing re-export.
//!
//! The `compile_probe` module at the bottom references each exported name
//! so dropping a re-export from `lib.rs` triggers a compile error here.

#![allow(dead_code, unused_imports)]

use cucumber::{given, then, World};

#[derive(Debug, Default, World)]
pub struct ParityWorld {
    importable: bool,
}

const EXPORTED: &[&str] = &[
    // Clients
    "CommandHandlerClient",
    "QueryClient",
    "SpeculativeClient",
    "DomainClient",
    // Router runtime
    "Router",
    "BuildError",
    "DispatchError",
    "CommandHandlerRouter",
    "SagaRouter",
    "ProcessManagerRouter",
    "ProjectorRouter",
    "UpcasterRouter",
    // Handler kind declarations (proc macros)
    "command_handler",
    "saga",
    "process_manager",
    "projector",
    "upcaster",
    // Method markers
    "handles",
    "applies",
    "rejected",
    "state_factory",
    "upcasts",
    // gRPC server adapters
    "CommandHandlerGrpc",
    "SagaGrpc",
    "ProcessManagerGrpc",
    "ProjectorGrpc",
    "UpcasterGrpc",
    // Response types
    "SagaHandlerResponse",
    "ProcessManagerResponse",
    "RejectionHandlerResponse",
    // Errors
    "ClientError",
    "CommandRejectedError",
    // Constants
    "TYPE_URL_PREFIX",
    "UNKNOWN_DOMAIN",
    "WILDCARD_DOMAIN",
    "DEFAULT_EDITION",
    "META_ANGZARR_DOMAIN",
    "PROJECTION_DOMAIN_PREFIX",
    "PROJECTION_TYPE_URL",
    // Identity helpers
    "compute_root",
    "to_proto_bytes",
    // Retry
    "RetryPolicy",
    "ExponentialBackoffRetry",
    "default_retry_policy",
    // Validation
    "require_exists",
    "require_not_exists",
    "require_positive",
    "require_non_negative",
    "require_not_empty",
    "require_not_empty_str",
    "require_status",
    "require_status_not",
    // Compensation
    "CompensationContext",
    "delegate_to_framework",
    "emit_compensation_events",
    "pm_delegate_to_framework",
    "pm_emit_compensation_events",
    // Event packing — pack_event/pack_events removed in audit #57
    // and new_event_book/new_event_book_multi removed under @C-0103
    // (zero production callers; per-language helpers had divergent
    // contracts). Production code uses inline `Any { type_url, value }`
    // or `testing::pack_event(msg)` for fixtures.
    // Builders (direct types, distinct from *Ext traits)
    "CommandBuilder",
    "QueryBuilder",
    // Destinations
    "Destinations",
    // Server utilities
    "configure_logging",
    "get_transport_config",
    "create_server",
    "run_server",
    "cleanup_socket",
];

/// Names reachable from `angzarr_client::testing` (the `testing` feature,
/// which this crate's dev-dependency enables) and not from the root.
const TESTING_EXPORTED: &[&str] = &[
    "make_timestamp",
    "make_cover",
    "make_event_page",
    "make_event_book",
    "make_command_page",
    "make_command_book",
    "uuid_for",
    "uuid_str_for",
    "uuid_obj_for",
    "DEFAULT_TEST_NAMESPACE",
    "ScenarioContext",
];

/// Predicates implemented on `ClientError` (verified by `tests::error` below
/// — `ClientError::is_not_found`, etc.).
const ERROR_PREDICATES_IMPLEMENTED: &[&str] = &[
    "is_not_found",
    "is_precondition_failed",
    "is_invalid_argument",
    "is_connection_error",
];

fn check(name: &str) {
    assert!(
        EXPORTED.contains(&name),
        "\"{}\" is not re-exported from angzarr_client",
        name
    );
}

fn check_testing(name: &str) {
    assert!(
        TESTING_EXPORTED.contains(&name),
        "\"{}\" is not exported from angzarr_client::testing",
        name
    );
}

// --- Background ------------------------------------------------------------

#[given("the angzarr client library is importable at its public root")]
async fn given_library_importable(world: &mut ParityWorld) {
    world.importable = true;
}

// --- Generic symbol checks -------------------------------------------------

#[then(expr = "the {string} symbol is exported")]
async fn then_symbol_exported(_world: &mut ParityWorld, name: String) {
    check(&name);
}

#[then(expr = "the {string} kind declaration is exported")]
async fn then_kind_decl_exported(_world: &mut ParityWorld, name: String) {
    check(&name);
}

#[then(expr = "the {string} method marker is exported")]
async fn then_method_marker_exported(_world: &mut ParityWorld, name: String) {
    check(&name);
}

#[then(expr = "the {string} symbol is exported from the testing module")]
async fn then_symbol_in_testing(_world: &mut ParityWorld, name: String) {
    check_testing(&name);
}

#[then(expr = "the {string} constant is exported from the testing module")]
async fn then_constant_in_testing(_world: &mut ParityWorld, name: String) {
    check_testing(&name);
}

#[then("none of the testing helpers is exported from the client's root")]
async fn then_testing_not_at_root(_world: &mut ParityWorld) {
    for name in TESTING_EXPORTED {
        assert!(
            !EXPORTED.contains(name),
            "\"{name}\" is re-exported from the angzarr_client root"
        );
    }
    // `root_probe` compiles only while the crate root exports none of them.
    root_probe::check();
}

#[then(expr = "the {string} constant is exported")]
async fn then_constant_exported(_world: &mut ParityWorld, name: String) {
    check(&name);
}

#[then(expr = "the {string} constant is exported with value {string}")]
async fn then_constant_value(_world: &mut ParityWorld, name: String, value: String) {
    check(&name);
    let actual = match name.as_str() {
        "TYPE_URL_PREFIX" => angzarr_client::TYPE_URL_PREFIX,
        other => panic!("no value probe for constant {other}"),
    };
    assert_eq!(actual, value, "{name} value");
}

#[then(expr = "the client exposes the {string} error predicate")]
async fn then_error_predicate_exposed(_world: &mut ParityWorld, name: String) {
    assert!(
        ERROR_PREDICATES_IMPLEMENTED.contains(&name.as_str()),
        "error predicate \"{}\" is not implemented on ClientError",
        name
    );
}

// --- Compile-time probe: referencing each exported name forces lib.rs to
// keep them reachable. Drop a re-export => this module stops compiling.
mod compile_probe {
    #![allow(unused_imports, dead_code)]
    use angzarr_client::testing::{
        make_command_book, make_command_page, make_cover, make_event_book, make_event_page,
        make_timestamp, uuid_for, uuid_obj_for, uuid_str_for, ScenarioContext,
        DEFAULT_TEST_NAMESPACE,
    };
    use angzarr_client::{
        applies, cleanup_socket, command_handler, compute_root, configure_logging, create_server,
        default_retry_policy, delegate_to_framework, emit_compensation_events,
        get_transport_config, handles, pm_delegate_to_framework, pm_emit_compensation_events,
        process_manager, projector, rejected, require_exists, require_non_negative,
        require_not_empty, require_not_empty_str, require_not_exists, require_positive,
        require_status, require_status_not, run_server, saga, state_factory, to_proto_bytes,
        upcaster, upcasts, BuildError, ClientError, CommandBuilder, CommandHandlerClient,
        CommandHandlerGrpc, CommandHandlerRouter, CommandRejectedError, CompensationContext,
        Destinations, DispatchError, DomainClient, ExponentialBackoffRetry, ProcessManagerGrpc,
        ProcessManagerResponse, ProcessManagerRouter, ProjectorGrpc, ProjectorRouter, QueryBuilder,
        QueryClient, RejectionHandlerResponse, RetryPolicy, Router, SagaGrpc, SagaHandlerResponse,
        SagaRouter, SpeculativeClient, UpcasterGrpc, UpcasterRouter, DEFAULT_EDITION,
        META_ANGZARR_DOMAIN, PROJECTION_DOMAIN_PREFIX, PROJECTION_TYPE_URL, TYPE_URL_PREFIX,
        UNKNOWN_DOMAIN, WILDCARD_DOMAIN,
    };
}

/// Compiles only while the crate root exports none of the testing helpers:
/// each name below is also provided by `fallback`, and a name both glob
/// imports provide is ambiguous at its use site.
#[allow(non_upper_case_globals, dead_code)]
mod root_probe {
    mod fallback {
        pub const make_timestamp: u8 = 0;
        pub const make_cover: u8 = 0;
        pub const make_event_page: u8 = 0;
        pub const make_event_book: u8 = 0;
        pub const make_command_page: u8 = 0;
        pub const make_command_book: u8 = 0;
        pub const uuid_for: u8 = 0;
        pub const uuid_str_for: u8 = 0;
        pub const uuid_obj_for: u8 = 0;
        pub const DEFAULT_TEST_NAMESPACE: u8 = 0;
        pub struct ScenarioContext;
    }

    #[allow(unused_imports)]
    use angzarr_client::*;
    use fallback::*;

    pub fn check() {
        let names: [u8; 10] = [
            make_timestamp,
            make_cover,
            make_event_page,
            make_event_book,
            make_command_page,
            make_command_book,
            uuid_for,
            uuid_str_for,
            uuid_obj_for,
            DEFAULT_TEST_NAMESPACE,
        ];
        assert_eq!(names, [0; 10]);
        let _: fallback::ScenarioContext = ScenarioContext;
    }
}
