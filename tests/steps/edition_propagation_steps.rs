//! Edition propagation step definitions (coordinator-contract simulation).
//!
//! Covers the cross-domain edition-propagation contract: sagas and process
//! managers must inherit the source / trigger cover's edition on every
//! emitted command, event, or process_events book. See
//! `angzarr-project/features/coordinator-contract/edition_propagation.feature`.

use cucumber::{given, then, when, World};

/// Test context for edition-propagation scenarios.
///
/// Fields are intentionally minimal; the harness is currently stubbed and
/// will be fleshed out as the coordinator-contract simulation is wired up.
#[derive(Debug, Default, World)]
pub struct EditionPropagationWorld {
    #[allow(dead_code)]
    saga_name: Option<String>,
    #[allow(dead_code)]
    pm_name: Option<String>,
    #[allow(dead_code)]
    source_edition: Option<String>,
    #[allow(dead_code)]
    trigger_edition: Option<String>,
    #[allow(dead_code)]
    handler_outgoing_edition: Option<String>,
}

// ==========================================================================
// Given steps
// ==========================================================================

// TODO (WIP): Implement this step matcher properly.
#[given(expr = "a saga {string} translating from {string} to {string}")]
async fn given_saga_translating(
    _world: &mut EditionPropagationWorld,
    _saga: String,
    _from: String,
    _to: String,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(expr = "a process manager {string} with sources {string} and targets {string}")]
async fn given_process_manager(
    _world: &mut EditionPropagationWorld,
    _pm: String,
    _sources: String,
    _targets: String,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given("the saga handles OrderCreated by emitting a ReserveStock command")]
async fn given_saga_handles_emit_reserve_stock(_world: &mut EditionPropagationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given("the saga handles OrderCreated by emitting an OrderObserved event")]
async fn given_saga_handles_emit_order_observed(_world: &mut EditionPropagationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given("the PM also emits an OrderTracked process_event on OrderCreated")]
async fn given_pm_emits_order_tracked(_world: &mut EditionPropagationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(expr = "the source event has edition {string}")]
async fn given_source_event_has_edition(_world: &mut EditionPropagationWorld, _edition: String) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(expr = "the source event has edition {string} with divergence at {string}={int}")]
async fn given_source_event_edition_with_divergence(
    _world: &mut EditionPropagationWorld,
    _edition: String,
    _domain: String,
    _seq: u32,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given("the source event has no edition set")]
async fn given_source_event_no_edition(_world: &mut EditionPropagationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(expr = "the saga handler sets outgoing edition {string}")]
async fn given_saga_handler_sets_outgoing_edition(
    _world: &mut EditionPropagationWorld,
    _edition: String,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(expr = "the trigger event has edition {string}")]
async fn given_trigger_event_has_edition(_world: &mut EditionPropagationWorld, _edition: String) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[given(expr = "the PM handler sets outgoing edition {string}")]
async fn given_pm_handler_sets_outgoing_edition(
    _world: &mut EditionPropagationWorld,
    _edition: String,
) {
    panic!("WIP: step needs implementation");
}

// ==========================================================================
// When steps
// ==========================================================================

// TODO (WIP): Implement this step matcher properly.
#[when("an OrderCreated event is dispatched to the saga")]
async fn when_order_created_dispatched_to_saga(_world: &mut EditionPropagationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[when("an OrderCreated trigger is dispatched to the PM")]
async fn when_order_created_dispatched_to_pm(_world: &mut EditionPropagationWorld) {
    panic!("WIP: step needs implementation");
}

// ==========================================================================
// Then steps
// ==========================================================================

// TODO (WIP): Implement this step matcher properly.
#[then(expr = "the emitted command's cover has edition {string}")]
async fn then_emitted_command_cover_edition(
    _world: &mut EditionPropagationWorld,
    _edition: String,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(expr = "the emitted command's cover has edition {string} with divergence at {string}={int}")]
async fn then_emitted_command_cover_edition_with_divergence(
    _world: &mut EditionPropagationWorld,
    _edition: String,
    _domain: String,
    _seq: u32,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then("the emitted command's cover has no edition set")]
async fn then_emitted_command_cover_no_edition(_world: &mut EditionPropagationWorld) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(expr = "the emitted event's cover has edition {string}")]
async fn then_emitted_event_cover_edition(_world: &mut EditionPropagationWorld, _edition: String) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(expr = "the persisted command's cover has edition {string}")]
async fn then_persisted_command_cover_edition(
    _world: &mut EditionPropagationWorld,
    _edition: String,
) {
    panic!("WIP: step needs implementation");
}

// TODO (WIP): Implement this step matcher properly.
#[then(expr = "every emitted process_events book's cover has edition {string}")]
async fn then_every_process_events_book_cover_edition(
    _world: &mut EditionPropagationWorld,
    _edition: String,
) {
    panic!("WIP: step needs implementation");
}
