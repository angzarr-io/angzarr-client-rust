// tonic::Status is 176 bytes - acceptable for gRPC error handling
#![allow(clippy::result_large_err)]

//! Unified router over the angzarr-router dispatch engine.
//!
//! Users compose handlers via [`Router::new`]`.with_handler(factory).build()`
//! and match on the returned [`Built`] for the kind-specific runtime router
//! ([`CommandHandlerRouter`], [`SagaRouter`], [`ProcessManagerRouter`],
//! [`ProjectorRouter`], [`UpcasterRouter`]).
//!
//! # Example
//!
//! ```rust,ignore
//! let router = Router::new("agg-player")
//!     .with_handler(|| Player::new(db_pool.clone()))
//!     .build()?;
//! match router {
//!     Built::CommandHandler(ch) => run_command_handler_server("player", 50001, ch).await,
//!     _ => unreachable!(),
//! }
//! ```

pub(crate) mod builder;
#[doc(hidden)]
pub mod component;
mod handler;
pub mod responses;
pub mod routers;

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
