//! Ergonomic Rust client for Angzarr gRPC services.
//!
//! This crate provides typed clients with fluent builder APIs for interacting
//! with Angzarr aggregate coordinator and query services.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use angzarr_client::{DomainClient, CommandBuilderExt, QueryBuilderExt};
//! use uuid::Uuid;
//!
//! async fn example() -> angzarr_client::Result<()> {
//!     // Connect to a domain's coordinator
//!     let client = DomainClient::connect("http://localhost:1310").await?;
//!
//!     // Execute a command
//!     let cart_id = Uuid::new_v4();
//!     let response = client.command_handler
//!         .command("cart", cart_id)
//!         .with_command("/examples.CreateCart", &create_cart)
//!         .execute() // SyncMode::Async; execute_with_mode(mode) picks another
//!         .await?;
//!
//!     // Query events
//!     let events = client.query
//!         .query("cart", cart_id)
//!         .range(0..)
//!         .get_pages()
//!         .await?;
//!     Ok(())
//! }
//! ```
//!
//! # Mocking for Tests
//!
//! The `testing` cargo feature provides recording fakes of the client traits
//! in `angzarr_client::testing::fakes`, along with proto builders and
//! deterministic test UUIDs. Any type implementing `GatewayClient` or
//! `QueryClient` can stand in for the coordinator:
//!
//! ```rust,ignore
//! use angzarr_client::traits::{GatewayClient, QueryClient};
//! use angzarr_client::proto::CommandBook;
//! use async_trait::async_trait;
//!
//! struct MockAggregate;
//!
//! #[async_trait]
//! impl GatewayClient for MockAggregate {
//!     async fn execute(&self, _cmd: CommandBook)
//!         -> angzarr_client::Result<angzarr_client::proto::CommandResponse>
//!     {
//!         // Return mock response
//!         Ok(angzarr_client::proto::CommandResponse::default())
//!     }
//! }
//! ```

/// Version of the angzarr-client crate, injected at build time from VERSION file.
pub const VERSION: &str = env!("ANGZARR_CLIENT_VERSION");

pub mod builder;
pub mod client;
pub mod compensation;
pub mod convert;
pub mod error;
pub mod error_codes;
pub mod handler;
pub mod host;
pub mod identity;
#[path = "proto.rs"]
pub mod proto;
pub mod proto_ext;
pub mod readiness;
pub mod retry;
pub mod router;
pub mod server;
#[cfg(feature = "testing")]
pub mod testing;
pub mod traits;
pub mod transport;
pub mod validation;

// Re-export main types at crate root
pub use client::{CommandHandlerClient, DomainClient, QueryClient, SpeculativeClient};
pub use error::{ClientError, CommandRejectedError, CommandResult, Result};
pub use identity::{compute_root, to_proto_bytes};
pub use retry::{default_retry_policy, ExponentialBackoffRetry, RetryPolicy};
pub use transport::{resolve_ch_endpoint, TransportMode};

// Re-export builder extension traits for fluent API
pub use builder::{CommandBuilder, CommandBuilderExt, QueryBuilder, QueryBuilderExt};

// Re-export helpers
pub use builder::{decode_event, events_from_response};
pub use convert::{
    full_type_name, full_type_url, now, parse_timestamp, proto_to_uuid, try_unpack, type_matches,
    type_name_from_url, type_url, type_url_is, type_url_matches, type_url_matches_exact, unpack,
    uuid_to_proto, DEFAULT_EDITION, META_ANGZARR_DOMAIN, PROJECTION_DOMAIN_PREFIX,
    PROJECTION_TYPE_URL, TYPE_URL_PREFIX, UNKNOWN_DOMAIN, WILDCARD_DOMAIN,
};

// Re-export extension traits
pub use proto_ext::constants::CORRELATION_ID_HEADER;
pub use proto_ext::{
    correlated_request, destination_map, CommandBookExt, CommandPageExt, CoverExt, EditionExt,
    EventBookExt, EventPageExt, ProtoUuidExt, UuidExt,
};

// Component host and transport configuration
pub use host::{ComponentHost, HostAddress, RunningHost};
pub use server::{
    configure_logging, get_transport_config, resolve_bind_address, ServerConfig, DEFAULT_BIND_HOST,
    ENV_BIND_ADDRESS,
};

// Re-export validation helpers
pub use validation::{
    require_exists, require_non_negative, require_not_empty, require_not_empty_str,
    require_not_exists, require_positive, require_status, require_status_not,
};
