// tonic::Status is 176 bytes - acceptable for gRPC error handling
#![allow(clippy::result_large_err)]

//! Component declarations over the angzarr-router dispatch engine.
//!
//! The kind attributes ([`command_handler`], [`saga`], [`process_manager`],
//! [`projector`], [`upcaster`]) and their method markers turn a handler type
//! into registrations on [`binding`], the angzarr-router engine. Serve
//! components with [`crate::ComponentHost`]; [`Router`] builds a
//! kind-specific runtime router ([`CommandHandlerRouter`], [`SagaRouter`],
//! [`ProcessManagerRouter`], [`ProjectorRouter`], [`UpcasterRouter`]) for
//! direct dispatch.
//!
//! # Example
//!
//! ```rust,ignore
//! use angzarr_client::ComponentHost;
//!
//! ComponentHost::new()
//!     .with_handler(|| Ledger::new(pool.clone()))
//!     .serve()
//!     .await?;
//! ```

pub(crate) mod builder;
#[doc(hidden)]
pub mod component;
mod handler;
pub mod responses;
pub mod routers;

/// The angzarr-router dispatch engine the component macros register
/// handlers with.
pub use angzarr_router as binding;

// Component declarations: kind attributes and method markers.
pub use angzarr_macros::{
    applies, command_handler, handles, handles_fact, process_manager, projector, rejected, saga,
    state_factory, upcaster, upcasts,
};

// Public types
pub use angzarr_router::destinations::Destinations;
pub use builder::Router;
pub use handler::{BuildError, Built, DispatchError, Handler, HandlerConfig, HandlerKind, Kind};
pub use responses::{
    FactRecord, IntoFactRecord, ProcessManagerResponse, RejectionHandlerResponse,
    SagaHandlerResponse,
};
pub use routers::{
    CommandHandlerRouter, ProcessManagerRouter, ProjectorRouter, SagaRouter, UpcasterRouter,
};

pub use crate::error::{CommandRejectedError, CommandResult};
