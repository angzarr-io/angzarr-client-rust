//! Shared test proto types.
//!
//! Port of `client-python/main/tests/fixtures.py`. Each type's
//! `prost::Name` PACKAGE/NAME matches the Python `DESCRIPTOR.full_name`
//! byte-for-byte so the resulting `type.googleapis.com/<full_name>` URLs
//! are identical across languages.

// ---------------------------------------------------------------------------
// Order domain — Commands
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct CreateOrder {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub customer_id: ::prost::alloc::string::String,
    #[prost(string, repeated, tag = "3")]
    pub items: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
}
impl ::prost::Name for CreateOrder {
    const PACKAGE: &'static str = "order";
    const NAME: &'static str = "CreateOrder";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CompleteOrder {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
}
impl ::prost::Name for CompleteOrder {
    const PACKAGE: &'static str = "order";
    const NAME: &'static str = "CompleteOrder";
}

// ---------------------------------------------------------------------------
// Order domain — Events
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct OrderCreatedV1 {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub customer_id: ::prost::alloc::string::String,
}
impl ::prost::Name for OrderCreatedV1 {
    const PACKAGE: &'static str = "order";
    const NAME: &'static str = "OrderCreatedV1";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct OrderCreated {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub customer_id: ::prost::alloc::string::String,
    #[prost(int64, tag = "3")]
    pub total: i64,
}
impl ::prost::Name for OrderCreated {
    const PACKAGE: &'static str = "order";
    const NAME: &'static str = "OrderCreated";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct OrderCompleted {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub shipped_at: ::prost::alloc::string::String,
}
impl ::prost::Name for OrderCompleted {
    const PACKAGE: &'static str = "order";
    const NAME: &'static str = "OrderCompleted";
}

// ---------------------------------------------------------------------------
// Inventory domain
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct ReserveStock {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub sku: ::prost::alloc::string::String,
    #[prost(int64, tag = "3")]
    pub quantity: i64,
}
impl ::prost::Name for ReserveStock {
    const PACKAGE: &'static str = "inventory";
    const NAME: &'static str = "ReserveStock";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct StockReserved {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub sku: ::prost::alloc::string::String,
    #[prost(int64, tag = "3")]
    pub quantity: i64,
}
impl ::prost::Name for StockReserved {
    const PACKAGE: &'static str = "inventory";
    const NAME: &'static str = "StockReserved";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct StockUpdated {
    #[prost(string, tag = "1")]
    pub sku: ::prost::alloc::string::String,
    #[prost(int64, tag = "2")]
    pub quantity: i64,
}
impl ::prost::Name for StockUpdated {
    const PACKAGE: &'static str = "inventory";
    const NAME: &'static str = "StockUpdated";
}

// ---------------------------------------------------------------------------
// Fulfillment domain
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct CreateShipment {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub address: ::prost::alloc::string::String,
}
impl ::prost::Name for CreateShipment {
    const PACKAGE: &'static str = "fulfillment";
    const NAME: &'static str = "CreateShipment";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ShipmentCreated {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub tracking_number: ::prost::alloc::string::String,
}
impl ::prost::Name for ShipmentCreated {
    const PACKAGE: &'static str = "fulfillment";
    const NAME: &'static str = "ShipmentCreated";
}

// ---------------------------------------------------------------------------
// Player domain — Events (projector tests)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct PlayerRegisteredV1 {
    #[prost(string, tag = "1")]
    pub player_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub display_name: ::prost::alloc::string::String,
}
impl ::prost::Name for PlayerRegisteredV1 {
    const PACKAGE: &'static str = "player";
    const NAME: &'static str = "PlayerRegisteredV1";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PlayerRegistered {
    #[prost(string, tag = "1")]
    pub player_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub display_name: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub registered_at: ::prost::alloc::string::String,
}
impl ::prost::Name for PlayerRegistered {
    const PACKAGE: &'static str = "player";
    const NAME: &'static str = "PlayerRegistered";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ScoreUpdated {
    #[prost(string, tag = "1")]
    pub player_id: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub game_id: ::prost::alloc::string::String,
    #[prost(int64, tag = "3")]
    pub score_delta: i64,
    #[prost(int64, tag = "4")]
    pub new_total: i64,
}
impl ::prost::Name for ScoreUpdated {
    const PACKAGE: &'static str = "player";
    const NAME: &'static str = "ScoreUpdated";
}

// ---------------------------------------------------------------------------
// Player domain — Commands (aggregate tests)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct RegisterPlayer {
    #[prost(string, tag = "1")]
    pub email: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub display_name: ::prost::alloc::string::String,
}
impl ::prost::Name for RegisterPlayer {
    const PACKAGE: &'static str = "player";
    const NAME: &'static str = "RegisterPlayer";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct DepositFunds {
    #[prost(int64, tag = "1")]
    pub amount: i64,
}
impl ::prost::Name for DepositFunds {
    const PACKAGE: &'static str = "player";
    const NAME: &'static str = "DepositFunds";
}

// ---------------------------------------------------------------------------
// Player domain — Additional Events (aggregate tests)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct FundsDeposited {
    #[prost(int64, tag = "1")]
    pub new_bankroll: i64,
}
impl ::prost::Name for FundsDeposited {
    const PACKAGE: &'static str = "player";
    const NAME: &'static str = "FundsDeposited";
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct FundsReleased {
    #[prost(int64, tag = "1")]
    pub amount: i64,
    #[prost(string, tag = "2")]
    pub reason: ::prost::alloc::string::String,
}
impl ::prost::Name for FundsReleased {
    const PACKAGE: &'static str = "player";
    const NAME: &'static str = "FundsReleased";
}

// ---------------------------------------------------------------------------
// Payment domain — Commands (rejection tests)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct ProcessPayment {
    #[prost(string, tag = "1")]
    pub order_id: ::prost::alloc::string::String,
    #[prost(int64, tag = "2")]
    pub amount: i64,
}
impl ::prost::Name for ProcessPayment {
    const PACKAGE: &'static str = "payment";
    const NAME: &'static str = "ProcessPayment";
}

// ---------------------------------------------------------------------------
// Workflow domain — Events (PM rejection tests)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct WorkflowFailed {
    #[prost(string, tag = "1")]
    pub reason: ::prost::alloc::string::String,
    #[prost(string, tag = "2")]
    pub failed_domain: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub failed_command: ::prost::alloc::string::String,
}
impl ::prost::Name for WorkflowFailed {
    const PACKAGE: &'static str = "workflow";
    const NAME: &'static str = "WorkflowFailed";
}
