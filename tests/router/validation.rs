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
fn saga_without_target_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/saga_without_target.rs");
}

#[test]
fn process_manager_without_targets_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/process_manager_without_targets.rs");
}

#[test]
fn projector_without_domains_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/projector_without_domains.rs");
}

#[test]
fn process_manager_without_pm_domain_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/process_manager_without_pm_domain.rs");
}

#[test]
fn process_manager_without_sources_fails_to_compile() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/router/ui/process_manager_without_sources.rs");
}
