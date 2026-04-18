//! Integration tests for the Tier 5 unified `Router`.

#[path = "router/builder.rs"]
mod builder;

#[path = "router/decorators.rs"]
mod decorators;

#[path = "router/dispatch_command_handler.rs"]
mod dispatch_command_handler;

#[path = "router/state_rebuild.rs"]
mod state_rebuild;

#[path = "router/multi_handler.rs"]
mod multi_handler;

#[path = "router/sequence.rs"]
mod sequence;

#[path = "router/rejection.rs"]
mod rejection;

#[path = "router/dispatch_saga.rs"]
mod dispatch_saga;

#[path = "router/dispatch_pm.rs"]
mod dispatch_pm;

#[path = "router/dispatch_projector.rs"]
mod dispatch_projector;

#[path = "router/stacking.rs"]
mod stacking;

#[path = "router/validation.rs"]
mod validation;
