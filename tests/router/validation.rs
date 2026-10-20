//! Compile-fail validation: each kind macro must reject impls missing their
//! required attributes at macro-parse time.
//!
//! Runtime build-time invariants (e.g. duplicate `(domain, type_url)` allowed
//! across factories) are covered by the gherkin scenarios in
//! `features/client/{builder,validation}.feature`; this module only holds the
//! compile-fail harness that gherkin cannot express.

#[test]
fn command_handler_without_state_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/command_handler_without_state.rs");
}

#[test]
fn saga_without_output_domain_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/saga_without_output_domain.rs");
}

/// A saga has exactly one destination (decision 14.24).
#[test]
fn a_saga_with_several_destinations_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/saga_with_several_destinations.rs");
}

#[test]
fn projector_without_domains_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/projector_without_domains.rs");
}

#[test]
fn process_manager_without_domain_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/process_manager_without_domain.rs");
}

#[test]
fn process_manager_input_domains_are_per_event() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/process_manager_with_input_domain.rs");
    t.compile_fail("tests/router/ui/process_manager_handler_without_domain.rs");
    t.compile_fail("tests/router/ui/handles_unknown_argument.rs");
}
