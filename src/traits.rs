//! Client traits for gateway and query operations.
//!
//! These traits define the client API for interacting with angzarr services.
//! Both standalone (in-process) and distributed (gRPC) modes implement
//! the same traits, enabling deploy-anywhere client code.

use async_trait::async_trait;

use crate::error::Result;
use crate::proto::{CommandBook, CommandResponse, EventBook, Query};

/// Trait for gateway client operations (command execution).
///
/// Implement this trait to create mock clients for testing or
/// alternative transport implementations.
#[async_trait]
pub trait GatewayClient: Send + Sync {
    /// Execute a command and return the coordinator's decision
    /// (`CommandHandlerCoordinatorService.Handle`).
    async fn execute(&self, command: CommandBook) -> Result<CommandResponse>;
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
}
