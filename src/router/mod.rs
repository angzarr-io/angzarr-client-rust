// tonic::Status is 176 bytes - acceptable for gRPC error handling
#![allow(clippy::result_large_err)]

//! Tier 5 unified router.
//!
//! Users compose handlers via [`Router::new`]`.with_handler(factory).build()`
//! and match on the returned [`Built`] for the kind-specific runtime router
//! ([`runtime::CommandHandlerRouter`], [`runtime::SagaRouter`],
//! [`runtime::ProcessManagerRouter`], [`runtime::ProjectorRouter`]).
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
mod handler;
mod helpers;
pub mod responses;
pub mod runtime;
mod state;
pub mod upcaster;

// Public types
pub use builder::Router;
pub use handler::{
    BuildError, Built, DispatchError, Handler, HandlerConfig, HandlerKind, HandlerRequest,
    HandlerResponse, Kind,
};
pub use helpers::{event_book_from, event_page, new_event_book, new_event_book_multi};
pub use responses::{ProcessManagerResponse, RejectionHandlerResponse, SagaHandlerResponse};
pub use runtime::{CommandHandlerRouter, ProcessManagerRouter, ProjectorRouter, SagaRouter};
pub use state::Destinations;
pub use upcaster::UpcasterRouter;

pub use crate::error::{CommandRejectedError, CommandResult};
