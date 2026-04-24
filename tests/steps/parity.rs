//! Step definitions for features/client/parity.feature.
//!
//! Asserts each canonical public name is reachable in the compiled crate.
//! Presence is checked against a hardcoded set mirroring the current
//! `lib.rs` re-exports. Names not in the set fail the scenario with a
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
    "CommandHandlerClient", "QueryClient", "SpeculativeClient", "DomainClient",
    // Router runtime
    "Router", "BuildError", "UpcasterRouter",
    // Handler kind declarations (proc macros)
    "saga", "process_manager", "projector",
    // Method markers
    "handles", "applies", "rejected",
    // gRPC server adapters
    "CommandHandlerGrpc",
    // Errors
    "ClientError", "CommandRejectedError",
    // Constants
    "INVENTORY_PRODUCT_NAMESPACE", "TYPE_URL_PREFIX",
    // Identity helpers
    "compute_root", "customer_root", "product_root", "order_root", "inventory_root",
    "inventory_product_root", "cart_root", "fulfillment_root", "to_proto_bytes",
    // Testing helpers
    "make_timestamp", "make_cover", "make_event_page", "make_event_book",
    "make_command_page", "make_command_book", "uuid_for", "uuid_str_for",
    "uuid_obj_for", "DEFAULT_TEST_NAMESPACE", "ScenarioContext",
    // Retry
    "RetryPolicy", "default_retry_policy",
    // Validation
    "require_exists", "require_not_exists", "require_positive", "require_non_negative",
    "require_not_empty", "require_not_empty_str", "require_status", "require_status_not",
    // Compensation
    "CompensationContext", "delegate_to_framework", "emit_compensation_events",
    "pm_delegate_to_framework", "pm_emit_compensation_events",
    // Event packing
    "pack_event", "new_event_book", "new_event_book_multi",
    // Destinations
    "Destinations",
];

const ERROR_PREDICATES_IMPLEMENTED: &[&str] = &[];

fn check(name: &str) {
    assert!(
        EXPORTED.contains(&name),
        "\"{}\" is not re-exported from angzarr_client",
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

#[then(expr = "the {string} constant is exported")]
async fn then_constant_exported(_world: &mut ParityWorld, name: String) {
    check(&name);
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
    use angzarr_client::{
        applies, cart_root, compute_root, customer_root, default_retry_policy,
        delegate_to_framework, emit_compensation_events, fulfillment_root, handles,
        inventory_product_root, inventory_root, make_command_book, make_command_page,
        make_cover, make_event_book, make_event_page, make_timestamp, new_event_book,
        new_event_book_multi, order_root, pm_delegate_to_framework, pm_emit_compensation_events,
        process_manager, product_root, projector, rejected, require_exists, require_non_negative,
        require_not_empty, require_not_empty_str, require_not_exists, require_positive,
        require_status, require_status_not, saga, to_proto_bytes, uuid_for, uuid_obj_for,
        uuid_str_for, BuildError, ClientError, CommandHandlerClient, CommandHandlerGrpc,
        CommandRejectedError, CompensationContext, Destinations, DomainClient, QueryClient,
        RetryPolicy, Router, ScenarioContext, SpeculativeClient, UpcasterRouter,
        DEFAULT_TEST_NAMESPACE, INVENTORY_PRODUCT_NAMESPACE, TYPE_URL_PREFIX,
    };
    // `pack_event` lives at crate root via router::pack_event
    use angzarr_client::pack_event;
}
