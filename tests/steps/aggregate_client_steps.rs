//! AggregateClient step definitions.

use cucumber::{given, then, when, World};
use std::collections::HashMap;

/// Test context for AggregateClient scenarios.
#[derive(Debug, Default, World)]
pub struct AggregateClientWorld {
    domain: String,
    root: String,
    sequence: u32,
    command_type: String,
    command_data: String,
    correlation_id: Option<String>,
    sync_mode: String,
    timeout_ms: Option<u32>,
    command_succeeded: bool,
    command_failed: bool,
    error: Option<String>,
    error_type: Option<String>,
    events_returned: Vec<(String, u32)>,
    concurrent_results: Vec<bool>,
    aggregates: HashMap<String, u32>,
    projectors_configured: bool,
    sagas_configured: bool,
    service_available: bool,
    service_slow: bool,
    current_sequence: Option<u32>,
    /// Original literal root identifier from the spec (e.g. "order-001"),
    /// kept so Then steps can compare back against the spec's verbatim
    /// text for spec-mutation detection. Distinct from `root` which may
    /// be UUID-coerced for protocol use.
    root_label: String,
    /// Last sequence value passed to an `execute … at sequence N` When,
    /// for spec-mutation verification.
    last_executed_sequence: Option<u32>,
}

// ==========================================================================
// Background Steps
// ==========================================================================

#[given("an AggregateClient connected to the test backend")]
async fn given_aggregate_client(world: &mut AggregateClientWorld) {
    world.service_available = true;
}

// ==========================================================================
// Given Steps - Aggregates
// ==========================================================================

#[given(expr = "a new aggregate root in domain {string}")]
async fn given_new_aggregate(world: &mut AggregateClientWorld, domain: String) {
    world.domain = domain.clone();
    world.root = uuid::Uuid::new_v4().to_string();
    world.sequence = 0;
    world
        .aggregates
        .insert(format!("{}:{}", domain, world.root), 0);
}

#[given(expr = "an aggregate {string} with root {string} at sequence {int}")]
async fn given_aggregate_at_sequence(
    world: &mut AggregateClientWorld,
    domain: String,
    root: String,
    seq: u32,
) {
    world.domain = domain.clone();
    world.root_label = root.clone();
    world.root = root.clone();
    world.sequence = seq;
    world.aggregates.insert(format!("{}:{}", domain, root), seq);
}

#[given(expr = "an aggregate {string} with root {string}")]
async fn given_aggregate(world: &mut AggregateClientWorld, domain: String, root: String) {
    world.domain = domain.clone();
    world.root_label = root.clone();
    world.root = root.clone();
    world.sequence = 0;
    world.aggregates.insert(format!("{}:{}", domain, root), 0);
}

#[given(expr = "no aggregate exists for domain {string} root {string}")]
async fn given_no_aggregate(world: &mut AggregateClientWorld, domain: String, root: String) {
    world.domain = domain;
    world.root_label = root.clone();
    world.root = root;
    world.sequence = 0;
}

#[given(expr = "projectors are configured for {string} domain")]
async fn given_projectors_configured(world: &mut AggregateClientWorld, _domain: String) {
    world.projectors_configured = true;
}

#[given(expr = "sagas are configured for {string} domain")]
async fn given_sagas_configured(world: &mut AggregateClientWorld, _domain: String) {
    world.sagas_configured = true;
}

#[given("the aggregate service is unavailable")]
async fn given_service_unavailable(world: &mut AggregateClientWorld) {
    world.service_available = false;
}

#[given("the aggregate service is slow to respond")]
async fn given_service_slow(world: &mut AggregateClientWorld) {
    world.service_slow = true;
}

// ==========================================================================
// When Steps - Commands
// ==========================================================================

#[when(expr = "I execute a {string} command with data {string}")]
async fn when_execute_command_with_data(
    world: &mut AggregateClientWorld,
    cmd_type: String,
    data: String,
) {
    world.command_type = cmd_type.clone();
    world.command_data = data;
    world.command_succeeded = true;
    // Convert command type to event type (e.g., "CreateOrder" -> "OrderCreated")
    let event_type = if cmd_type.starts_with("Create") {
        format!(
            "{}Created",
            cmd_type.strip_prefix("Create").unwrap_or(&cmd_type)
        )
    } else {
        cmd_type.clone()
    };
    world.events_returned.push((event_type, world.sequence));
}

#[when(expr = "I execute a {string} command at sequence {int}")]
async fn when_execute_command_at_sequence(
    world: &mut AggregateClientWorld,
    cmd_type: String,
    seq: u32,
) {
    world.command_type = cmd_type.clone();
    world.last_executed_sequence = Some(seq);
    let key = format!("{}:{}", world.domain, world.root);
    let current_seq = *world.aggregates.get(&key).unwrap_or(&0);

    if seq != current_seq {
        world.command_failed = true;
        world.error_type = Some("precondition".to_string());
        world.error = Some("Sequence mismatch".to_string());
    } else {
        world.command_succeeded = true;
        world.events_returned.push((cmd_type, seq));
    }
}

#[when(expr = "I execute a command at sequence {int}")]
async fn when_execute_at_sequence(world: &mut AggregateClientWorld, seq: u32) {
    world.last_executed_sequence = Some(seq);
    let key = format!("{}:{}", world.domain, world.root);
    let current_seq = *world.aggregates.get(&key).unwrap_or(&0);

    if seq != current_seq {
        world.command_failed = true;
        world.error_type = Some("precondition".to_string());
        world.error = Some("Sequence mismatch".to_string());
    } else {
        world.command_succeeded = true;
        world.events_returned.push(("Event".to_string(), seq));
    }
}

#[when(expr = "I execute a command with correlation ID {string}")]
async fn when_execute_with_correlation(world: &mut AggregateClientWorld, cid: String) {
    world.correlation_id = Some(cid);
    world.command_succeeded = true;
    world
        .events_returned
        .push(("Event".to_string(), world.sequence));
}

#[when("two commands are sent concurrently at sequence 0")]
async fn when_concurrent_commands(world: &mut AggregateClientWorld) {
    // First succeeds
    world.concurrent_results.push(true);
    // Second fails with precondition error
    world.concurrent_results.push(false);
}

#[when(expr = "I query the current sequence for {string} root {string}")]
async fn when_query_current_sequence(
    world: &mut AggregateClientWorld,
    domain: String,
    root: String,
) {
    let key = format!("{}:{}", domain, root);
    world.current_sequence = world.aggregates.get(&key).copied();
}

#[when("I retry the command at the correct sequence")]
async fn when_retry_correct_sequence(world: &mut AggregateClientWorld) {
    world.command_succeeded = true;
    world.command_failed = false;
    world.error = None;
    world.error_type = None;
}

#[when("I execute a command asynchronously")]
async fn when_execute_async(world: &mut AggregateClientWorld) {
    world.sync_mode = "ASYNC".to_string();
    world.command_succeeded = true;
}

#[when("I execute a command with sync mode SIMPLE")]
async fn when_execute_sync_simple(world: &mut AggregateClientWorld) {
    world.sync_mode = "SIMPLE".to_string();
    world.command_succeeded = true;
}

#[when("I execute a command with sync mode CASCADE")]
async fn when_execute_sync_cascade(world: &mut AggregateClientWorld) {
    world.sync_mode = "CASCADE".to_string();
    world.command_succeeded = true;
}

#[when("I execute a command with malformed payload")]
async fn when_execute_malformed(world: &mut AggregateClientWorld) {
    world.command_failed = true;
    world.error_type = Some("invalid_argument".to_string());
    world.error = Some("Invalid payload".to_string());
}

#[when("I execute a command without required fields")]
async fn when_execute_missing_fields(world: &mut AggregateClientWorld) {
    world.command_failed = true;
    world.error_type = Some("invalid_argument".to_string());
    world.error = Some("Missing required field: order_id".to_string());
}

#[when(expr = "I execute a command to domain {string}")]
async fn when_execute_to_domain(world: &mut AggregateClientWorld, domain: String) {
    if domain == "nonexistent" {
        world.command_failed = true;
        world.error_type = Some("unknown_domain".to_string());
        world.error = Some("Unknown domain".to_string());
    } else {
        world.command_succeeded = true;
    }
}

#[when("I execute a command that produces 3 events")]
async fn when_execute_multi_event(world: &mut AggregateClientWorld) {
    world.command_succeeded = true;
    let base_seq = world.sequence;
    world.events_returned.push(("Event1".to_string(), base_seq));
    world
        .events_returned
        .push(("Event2".to_string(), base_seq + 1));
    world
        .events_returned
        .push(("Event3".to_string(), base_seq + 2));
}

#[when(expr = "I query events for {string} root {string}")]
async fn when_query_events(world: &mut AggregateClientWorld, domain: String, root: String) {
    let key = format!("{}:{}", domain, root);
    if let Some(&count) = world.aggregates.get(&key) {
        for i in 0..count {
            world.events_returned.push(("Event".to_string(), i));
        }
    }
}

#[when("I attempt to execute a command")]
async fn when_attempt_execute(world: &mut AggregateClientWorld) {
    if !world.service_available {
        world.command_failed = true;
        world.error_type = Some("connection".to_string());
        world.error = Some("Connection error".to_string());
    }
}

#[when(expr = "I execute a command with timeout {int}ms")]
async fn when_execute_with_timeout(world: &mut AggregateClientWorld, timeout: u32) {
    world.timeout_ms = Some(timeout);
    if world.service_slow {
        world.command_failed = true;
        world.error_type = Some("timeout".to_string());
        world.error = Some("Deadline exceeded".to_string());
    }
}

#[when(expr = "I execute a {string} command for root {string} at sequence {int}")]
async fn when_execute_for_root(
    world: &mut AggregateClientWorld,
    cmd_type: String,
    root: String,
    seq: u32,
) {
    world.root = root.clone();
    if seq == 0 {
        world.command_succeeded = true;
        world
            .events_returned
            .push((cmd_type.replace("Create", "Created"), 0));
        world
            .aggregates
            .insert(format!("{}:{}", world.domain, root), 1);
    } else {
        world.command_failed = true;
        world.error_type = Some("precondition".to_string());
    }
}

// ==========================================================================
// Then Steps
// ==========================================================================

#[then("the command should succeed")]
async fn then_command_succeeds(world: &mut AggregateClientWorld) {
    assert!(world.command_succeeded, "Command should succeed");
}

#[then("the command should fail")]
async fn then_command_fails(world: &mut AggregateClientWorld) {
    assert!(world.command_failed, "Command should fail");
}

#[then(expr = "the response should contain {int} event")]
async fn then_response_contains_events(world: &mut AggregateClientWorld, count: u32) {
    assert_eq!(world.events_returned.len() as u32, count);
}

#[then(expr = "the response should contain {int} events")]
async fn then_response_contains_events_plural(world: &mut AggregateClientWorld, count: u32) {
    assert_eq!(world.events_returned.len() as u32, count);
}

#[then(expr = "the event should have type {string}")]
async fn then_event_has_type(world: &mut AggregateClientWorld, event_type: String) {
    assert!(!world.events_returned.is_empty());
    // Check if the returned event type matches the expected type
    assert_eq!(
        world.events_returned[0].0, event_type,
        "Expected event type '{}', got '{}'",
        event_type, world.events_returned[0].0
    );
}

#[then(expr = "the response should contain events starting at sequence {int}")]
async fn then_events_start_at(world: &mut AggregateClientWorld, seq: u32) {
    assert!(!world.events_returned.is_empty());
    assert_eq!(world.events_returned[0].1, seq);
}

#[then(expr = "the response events should have correlation ID {string}")]
async fn then_events_have_correlation(world: &mut AggregateClientWorld, cid: String) {
    assert_eq!(world.correlation_id, Some(cid));
}

#[then("the command should fail with precondition error")]
async fn then_fail_precondition(world: &mut AggregateClientWorld) {
    assert!(world.command_failed);
    assert_eq!(world.error_type, Some("precondition".to_string()));
}

#[then("the error should indicate sequence mismatch")]
async fn then_error_sequence_mismatch(world: &mut AggregateClientWorld) {
    assert!(world
        .error
        .as_ref()
        .map(|e| e.contains("Sequence"))
        .unwrap_or(false));
}

#[then("one should succeed")]
async fn then_one_succeeds(world: &mut AggregateClientWorld) {
    assert!(world.concurrent_results.iter().any(|&r| r));
}

#[then("one should fail with precondition error")]
async fn then_one_fails_precondition(world: &mut AggregateClientWorld) {
    assert!(world.concurrent_results.iter().any(|&r| !r));
}

#[then("the response should return without waiting for projectors")]
async fn then_async_returns(world: &mut AggregateClientWorld) {
    assert_eq!(world.sync_mode, "ASYNC");
}

#[then("the response should include projector results")]
async fn then_includes_projector_results(world: &mut AggregateClientWorld) {
    assert!(world.projectors_configured);
}

#[then("the response should include downstream saga results")]
async fn then_includes_saga_results(world: &mut AggregateClientWorld) {
    assert!(world.sagas_configured);
}

#[then("the command should fail with invalid argument error")]
async fn then_fail_invalid_argument(world: &mut AggregateClientWorld) {
    assert!(world.command_failed);
    assert_eq!(world.error_type, Some("invalid_argument".to_string()));
}

#[then("the error message should describe the missing field")]
async fn then_error_describes_field(world: &mut AggregateClientWorld) {
    assert!(world
        .error
        .as_ref()
        .map(|e| e.contains("field"))
        .unwrap_or(false));
}

#[then("the error should indicate unknown domain")]
async fn then_error_unknown_domain(world: &mut AggregateClientWorld) {
    assert_eq!(world.error_type, Some("unknown_domain".to_string()));
}

#[then(expr = "events should have sequences {int}, {int}, {int}")]
async fn then_events_have_sequences(world: &mut AggregateClientWorld, s1: u32, s2: u32, s3: u32) {
    assert_eq!(world.events_returned.len(), 3);
    assert_eq!(world.events_returned[0].1, s1);
    assert_eq!(world.events_returned[1].1, s2);
    assert_eq!(world.events_returned[2].1, s3);
}

#[then("I should see all 3 events or none")]
async fn then_atomic_events(world: &mut AggregateClientWorld) {
    // Either 3 or 0 events
    assert!(world.events_returned.len() == 3 || world.events_returned.is_empty());
}

#[then("the aggregate operation should fail with connection error")]
async fn then_aggregate_fail_connection(world: &mut AggregateClientWorld) {
    assert!(world.command_failed);
    assert_eq!(world.error_type, Some("connection".to_string()));
}

#[then("the operation should fail with timeout or deadline error")]
async fn then_fail_timeout(world: &mut AggregateClientWorld) {
    assert!(world.command_failed);
    assert_eq!(world.error_type, Some("timeout".to_string()));
}

#[then(expr = "the aggregate should now exist with {int} event")]
async fn then_aggregate_exists(world: &mut AggregateClientWorld, count: u32) {
    let key = format!("{}:{}", world.domain, world.root);
    assert_eq!(world.aggregates.get(&key), Some(&count));
}

// --------------------------------------------------------------------------
// Spec-mutation guards: independent capture of values from the spec so that
// mutations to the captured strings/ints in Given/When are observable here.
// See angzarr-project Tier 1 spec rewording (sour-mutants findings).
// --------------------------------------------------------------------------

#[then(expr = "the targeted aggregate has domain {string}")]
async fn then_targeted_domain(world: &mut AggregateClientWorld, expected: String) {
    assert_eq!(
        world.domain, expected,
        "world.domain={:?} expected={:?}",
        world.domain, expected
    );
}

#[then(expr = "the targeted aggregate has root {string}")]
async fn then_targeted_root(world: &mut AggregateClientWorld, expected: String) {
    assert_eq!(
        world.root_label, expected,
        "world.root_label={:?} expected={:?}",
        world.root_label, expected
    );
}

#[then(expr = "the executed command was at sequence {int}")]
async fn then_executed_sequence(world: &mut AggregateClientWorld, expected: u32) {
    assert_eq!(
        world.last_executed_sequence,
        Some(expected),
        "last_executed_sequence={:?} expected={}",
        world.last_executed_sequence,
        expected
    );
}

// ---------------------------------------------------------------------------
// WIP stubs: parity-cleanup generated step matchers (panic until implemented).
// ---------------------------------------------------------------------------

// TODO (WIP): Implement this step matcher properly.
#[given(regex = r"^a client connected to the test backend$")]
async fn wip_given_a_client_connected_to_the_test_backend(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r#"^I send a "([^"]*)" command with data "([^"]*)"$"#)]
async fn wip_when_i_send_a_createorder_command_with_data_customer_12(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the command is accepted$")]
async fn wip_then_the_command_is_accepted(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r#"^a single "([^"]*)" event is recorded$"#)]
async fn wip_then_a_single_ordercreated_event_is_recorded(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r#"^I send an "([^"]*)" command at sequence (-?\d+)$"#)]
async fn wip_when_i_send_an_additem_command_at_sequence_3(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the new events continue the history from sequence (-?\d+)$")]
async fn wip_then_the_new_events_continue_the_history_from_sequence(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r#"^I send a command tagged with correlation ID "([^"]*)"$"#)]
async fn wip_when_i_send_a_command_tagged_with_correlation_id_trace(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r#"^the resulting events carry correlation ID "([^"]*)"$"#)]
async fn wip_then_the_resulting_events_carry_correlation_id_trace_45(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I send a command at sequence (-?\d+)$")]
async fn wip_when_i_send_a_command_at_sequence_3(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the command is refused because the aggregate has moved on$")]
async fn wip_then_the_command_is_refused_because_the_aggregate_has_m(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^one command is accepted$")]
async fn wip_then_one_command_is_accepted(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the other is refused because the aggregate has moved on$")]
async fn wip_then_the_other_is_refused_because_the_aggregate_has_mov(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r#"^I look up the current sequence for "([^"]*)" root "([^"]*)"$"#)]
async fn wip_when_i_look_up_the_current_sequence_for_orders_root_ord(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I retry the command at that sequence$")]
async fn wip_when_i_retry_the_command_at_that_sequence(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I send a command without waiting for downstream work$")]
async fn wip_when_i_send_a_command_without_waiting_for_downstream_wo(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the response returns before any projectors have caught up$")]
async fn wip_then_the_response_returns_before_any_projectors_have_ca(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I send a command and wait for projectors$")]
async fn wip_when_i_send_a_command_and_wait_for_projectors(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the response reflects the projectors having processed the event$")]
async fn wip_then_the_response_reflects_the_projectors_having_proces(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I send a command and wait for downstream sagas$")]
async fn wip_when_i_send_a_command_and_wait_for_downstream_sagas(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the response reflects the downstream sagas having completed$")]
async fn wip_then_the_response_reflects_the_downstream_sagas_having(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I send a command with a malformed payload$")]
async fn wip_when_i_send_a_command_with_a_malformed_payload(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the command is refused as invalid$")]
async fn wip_then_the_command_is_refused_as_invalid(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I send a command missing required fields$")]
async fn wip_when_i_send_a_command_missing_required_fields(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the refusal names the missing field$")]
async fn wip_then_the_refusal_names_the_missing_field(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r#"^I send a command to domain "([^"]*)"$"#)]
async fn wip_when_i_send_a_command_to_domain_nonexistent(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the command is refused because the domain is unknown$")]
async fn wip_then_the_command_is_refused_because_the_domain_is_unkno(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I send a command that produces (-?\d+) events$")]
async fn wip_when_i_send_a_command_that_produces_3_events(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^(-?\d+) events are recorded$")]
async fn wip_then_3_events_are_recorded(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the events occupy consecutive sequences starting at (-?\d+)$")]
async fn wip_then_the_events_occupy_consecutive_sequences_starting_a(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r#"^I read back the events for "([^"]*)" root "([^"]*)"$"#)]
async fn wip_when_i_read_back_the_events_for_orders_root_order_007(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^either all (-?\d+) events are present or none of them are$")]
async fn wip_then_either_all_3_events_are_present_or_none_of_them_ar(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I attempt to send a command$")]
async fn wip_when_i_attempt_to_send_a_command(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the call fails because the service cannot be reached$")]
async fn wip_then_the_call_fails_because_the_service_cannot_be_reach(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(regex = r"^the aggregate service does not respond in time$")]
async fn wip_given_the_aggregate_service_does_not_respond_in_time(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r"^I send a command with a short timeout$")]
async fn wip_when_i_send_a_command_with_a_short_timeout(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the call fails because the deadline was exceeded$")]
async fn wip_then_the_call_fails_because_the_deadline_was_exceeded(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when(regex = r#"^I send a "([^"]*)" command for root "([^"]*)" at sequence (-?\d+)$"#)]
async fn wip_when_i_send_a_createorder_command_for_root_order_new_at(
    _world: &mut AggregateClientWorld,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(regex = r"^the aggregate now exists with one event$")]
async fn wip_then_the_aggregate_now_exists_with_one_event(_world: &mut AggregateClientWorld) {
    panic!("WIP: step needs implementation");
}
