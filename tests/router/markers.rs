//! Method-marker validation: malformed or conflicting markers are compile
//! errors, never silently ignored; marker types may be paths.

#[test]
fn malformed_and_conflicting_markers_fail_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/method_stack_handles_applies.rs");
    t.compile_fail("tests/router/ui/malformed_handles_marker.rs");
    t.compile_fail("tests/router/ui/malformed_rejected_marker.rs");
    t.compile_fail("tests/router/ui/unknown_upcasts_argument.rs");
}

#[test]
fn marker_types_may_be_paths() {
    let t = trybuild::TestCases::new();
    t.pass("tests/router/ui/handles_path_marker.rs");
    t.pass("tests/router/ui/upcaster_hygiene.rs");
}
