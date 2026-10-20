//! Client traits for gateway and query operations.
//!
//! These traits define the client API for interacting with angzarr services.
//! Both standalone (in-process) and distributed (gRPC) modes implement
//! the same traits, enabling deploy-anywhere client code.

use async_trait::async_trait;

use crate::error::Result;
use crate::proto::{CommandBook, CommandResponse, EventBook, Query};

/// Trait for gateway client operations (command execution): one method
/// per `CommandHandlerCoordinatorService` RPC (command_handler.proto).
///
/// Implement this trait to create mock clients for testing or
/// alternative transport implementations.
#[async_trait]
pub trait GatewayClient: Send + Sync {
    /// ASYNC (`HandleAsync`): the command is accepted for processing; no
    /// decision has been made, so nothing comes back.
    async fn handle_async(&self, command: CommandBook) -> Result<()>;

    /// DECISION (`Handle`): the decision, its events and delivery outcome.
    async fn handle(&self, command: CommandBook) -> Result<CommandResponse>;

    /// SIMPLE (`HandleSync`): the decision, after the domain's synchronous
    /// projectors ran; their results are in `outcome.projections`.
    async fn handle_sync(&self, command: CommandBook) -> Result<CommandResponse>;
}

/// Trait for event query client operations.
///
/// Implement this trait to create mock clients for testing or
/// alternative transport implementations.
#[async_trait]
pub trait QueryClient: Send + Sync {
    /// Fetch a single EventBook for the query (unary RPC).
    async fn get_event_book(&self, query: Query) -> Result<EventBook>;

    /// Fetch all matching EventBooks for the query (streaming RPC).
    ///
    /// Default implementation wraps `get_event_book` as a single-element
    /// vector — safe for mock impls that don't differentiate the unary
    /// and streaming RPCs. The real gRPC client overrides this to drain
    /// the streaming response. Mirrors Python's
    /// `QueryClient.get_events` (`client.py:187`).
    async fn get_events(&self, query: Query) -> Result<Vec<EventBook>> {
        Ok(vec![self.get_event_book(query).await?])
    }

    /// Every event of one correlation across every domain it touched
    /// (`GetCorrelatedEvents`), one EventBook per stream; `edition` picks
    /// the timeline (`None` = main).
    async fn get_correlated_events(
        &self,
        correlation_id: uuid::Uuid,
        edition: Option<String>,
    ) -> Result<Vec<EventBook>>;
}
