//! Compile-fail coverage for kind-macro stacking detection.
//!
//! Each fixture in `tests/router/ui/` applies more than one kind attribute
//! (or the same kind twice) to a single impl, and must fail at macro parse
//! time with a precise `compile_error!` — not by falling through to a later
//! E0119 trait-conflict against the generated `impl Handler`.

#[test]
fn stacking_kind_attributes_is_rejected_at_parse_time() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/stack_command_handler_saga.rs");
    t.compile_fail("tests/router/ui/stack_command_handler_projector.rs");
    t.compile_fail("tests/router/ui/stack_same_kind.rs");
}

#[test]
fn with_handler_rejects_types_not_implementing_handler() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/with_handler_rejects_non_handler.rs");
}
