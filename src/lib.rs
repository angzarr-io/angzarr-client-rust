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
//!         .with_command("type.googleapis.com/examples.CreateCart", &create_cart)
//!         .execute()
//!         .await?;
//!
//!     // Query events
//!     let events = client.query
//!         .query("cart", cart_id)
//!         .range(0)
//!         .get_pages()
//!         .await?;
//!     Ok(())
//! }
//! ```
//!
//! # Mocking for Tests
//!
//! Implement the `GatewayClient` and `QueryClient` traits to create mock clients:
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
pub mod handler;
pub mod identity;
#[path = "proto.rs"]
pub mod proto;
pub mod proto_ext;
pub mod retry;
pub mod router;
pub mod server;
pub mod testing;
pub mod traits;
pub mod transport;
pub mod validation;

// Re-export main types at crate root
pub use client::{CommandHandlerClient, DomainClient, QueryClient, SpeculativeClient};
pub use error::{ClientError, CommandRejectedError, CommandResult, Result};
pub use identity::{
    cart_root, compute_root, customer_root, fulfillment_root, inventory_product_root,
    inventory_root, order_root, product_root, to_proto_bytes, INVENTORY_PRODUCT_NAMESPACE,
};
pub use testing::{
    make_command_book, make_command_page, make_cover, make_event_book, make_event_page,
    make_timestamp, pack_event as testing_pack_event, uuid_for, uuid_obj_for, uuid_str_for,
    ScenarioContext, DEFAULT_TEST_NAMESPACE,
};
pub use retry::{default_retry_policy, RetryPolicy};
pub use transport::{resolve_ch_endpoint, TransportMode};

// Re-export builder extension traits for fluent API
pub use builder::{CommandBuilder, CommandBuilderExt, QueryBuilder, QueryBuilderExt};

// Re-export compensation helpers
pub use compensation::{
    delegate_to_framework, delegate_to_framework_with_options, emit_compensation_events,
    is_notification, pm_delegate_to_framework, pm_emit_compensation_events, CompensationContext,
    PMRevocationResponse,
};

// Re-export helpers
pub use builder::{decode_event, events_from_response, root_from_cover};
pub use convert::{
    full_type_name, full_type_url, now, parse_timestamp, proto_to_uuid, try_unpack, type_matches,
    type_name_from_url, type_url, type_url_matches_exact, unpack, uuid_to_proto, DEFAULT_EDITION,
    META_ANGZARR_DOMAIN, PROJECTION_DOMAIN_PREFIX, PROJECTION_TYPE_URL, TYPE_URL_PREFIX,
    UNKNOWN_DOMAIN, WILDCARD_DOMAIN,
};

// Re-export extension traits
pub use proto_ext::{
    CommandBookExt, CommandPageExt, CoverExt, EditionExt, EventBookExt, EventPageExt, ProtoUuidExt,
    UuidExt,
};

// Re-export Tier 5 unified router surface
pub use router::{
    // Helper functions
    event_book_from,
    event_page,
    new_event_book,
    new_event_book_multi,
    pack_event,
    pack_events,
    // Upcaster types (separate system, retained)
    BoxedUpcasterHandler,
    // Tier 5 unified Handler contract
    BuildError,
    Built,
    // Typed runtime routers returned by Router::build()
    CommandHandlerRouter,
    // Destination-sequence stamping for saga/PM outbound commands
    Destinations,
    DispatchError,
    Handler,
    HandlerConfig,
    HandlerKind,
    HandlerRequest,
    HandlerResponse,
    Kind,
    ProcessManagerResponse,
    ProcessManagerRouter,
    ProjectorRouter,
    RejectionHandlerResponse,
    // Builder
    Router,
    SagaHandlerResponse,
    SagaRouter,
    UpcasterHandler,
    UpcasterHandlerHOF,
    UpcasterMode,
    UpcasterRouter,
};

// Re-export handler types
pub use handler::{
    CommandHandlerGrpc, ProcessManagerGrpcHandler, ProjectorHandler, SagaHandler, StatePacker,
    UpcasterGrpcHandler, UpcasterHandleClosureFn, UpcasterHandleFn,
};

// Re-export server utilities
pub use server::{
    cleanup_socket, configure_logging, create_server, get_transport_config,
    run_command_handler_server, run_process_manager_server, run_projector_server, run_saga_server,
    run_server, run_upcaster_server, ServerConfig,
};

// Re-export validation helpers
pub use validation::{
    require_exists, require_non_negative, require_not_empty, require_not_empty_str,
    require_not_exists, require_positive, require_status, require_status_not,
};

// Re-export proc macros for Tier 5 OO-style component definitions
pub use angzarr_macros::{aggregate, applies, handles, process_manager, projector, rejected, saga};
