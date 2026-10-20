//! RED design tests for the sender and registration surface proposed in
//! `docs/design/sender-and-registration.md`. Built only with the
//! `proposal-tests` feature, and expected NOT to compile until the owner
//! signs the proposal off and it is implemented:
//!
//! ```bash
//! cargo test --features proposal-tests --test proposal
//! ```
//!
//! Spec: angzarr-project c0a00b9 — features/client/aggregate_client.feature
//! (C-0194..C-0198), features/client/query_client.feature (C-0241),
//! features/handler-contract/handling.feature (C-0533..C-0535), types.proto
//! (Handling, ComponentDescriptor) and command_handler.proto (the
//! DECISION / ASYNC / SIMPLE coordinator RPCs).

#[allow(dead_code)]
mod common;

use std::time::Duration;

use angzarr_client::proto::{
    CommandHandlerResponse, ComponentKind, ConflictResolution, ContextualCommand, EventBook,
};
#[allow(unused_imports)]
use angzarr_client::router::{applies, command_handler, handles, Built, Router};
use angzarr_client::{
    CommandBuilderExt, CommandHandlerClient, CommandResult, ComponentHost, QueryClient,
};

use common::backend::{GenericCommand, TestBackend, TYPE_PREFIX};

async fn connected() -> (TestBackend, CommandHandlerClient) {
    let backend = TestBackend::start_tcp().await;
    backend.add_projector("orders");
    let client = CommandHandlerClient::connect(backend.endpoint())
        .await
        .expect("connect");
    (backend, client)
}

fn add_item() -> GenericCommand {
    GenericCommand {
        data: "apple".into(),
        count: 1,
    }
}

// ---------------------------------------------------------------------------
// Delivery modes (C-0197, C-0198; command_handler.proto).
// ---------------------------------------------------------------------------

/// C-0197: fire-and-forget sends HandleAsync and returns before any
/// decision or projector ran; it answers nothing but acceptance.
#[tokio::test]
async fn enqueue_is_fire_and_forget_over_handle_async() {
    let (backend, client) = connected().await;
    let accepted: () = client
        .command_new("orders")
        .with_command(format!("{TYPE_PREFIX}AddItem"), &add_item())
        .enqueue()
        .await
        .expect("accepted for processing");
    let _ = accepted;
    assert_eq!(backend.rpcs(), vec!["HandleAsync"]);
    assert!(backend.projector_runs().is_empty());
}

/// C-0198: waiting for projectors sends HandleSync and returns their
/// projections in the outcome.
#[tokio::test]
async fn execute_projected_waits_for_the_synchronous_projectors() {
    let (backend, client) = connected().await;
    let response = client
        .command_new("orders")
        .with_command(format!("{TYPE_PREFIX}AddItem"), &add_item())
        .execute_projected()
        .await
        .expect("decided and projected");
    assert_eq!(backend.rpcs(), vec!["HandleSync"]);
    let outcome = response.outcome.expect("outcome");
    assert_eq!(outcome.projections.len(), 1);
    assert_eq!(outcome.projections[0].projector, "orders-projector");
}

/// The default stays DECISION (Handle): the decision, no projections.
#[tokio::test]
async fn execute_stays_the_decision() {
    let (backend, client) = connected().await;
    client
        .command_new("orders")
        .with_command(format!("{TYPE_PREFIX}AddItem"), &add_item())
        .execute()
        .await
        .expect("decided");
    assert_eq!(backend.rpcs(), vec!["Handle"]);
}

/// The trait carries all three modes, so fakes and alternative transports
/// implement them too.
#[tokio::test]
async fn the_gateway_trait_carries_every_mode() {
    #[allow(unused_imports)]
    use angzarr_client::traits::GatewayClient;
    let fake = angzarr_client::testing::RecordingGatewayClient::new();
    let book = fake
        .command_new("orders")
        .with_command(format!("{TYPE_PREFIX}AddItem"), &add_item())
        .build()
        .unwrap();
    fake.enqueue(book.clone()).await.unwrap();
    fake.execute_projected(book).await.unwrap();
    assert_eq!(fake.call_count("enqueue"), 1);
    assert_eq!(fake.call_count("execute_projected"), 1);
}

// ---------------------------------------------------------------------------
// Handling (types.proto, Handling; C-0194..C-0196).
// ---------------------------------------------------------------------------

/// A sender that does not know the head asks the coordinator to stamp it:
/// no expected_sequence on the wire.
#[test]
fn at_head_sends_no_expected_sequence() {
    let fake = angzarr_client::testing::RecordingGatewayClient::new();
    let book = fake
        .command_new("orders")
        .at_head()
        .with_conflict_resolution(ConflictResolution::Strict)
        .with_command(format!("{TYPE_PREFIX}AddItem"), &add_item())
        .build()
        .unwrap();
    let handling = book.handling.expect("handling");
    assert_eq!(handling.expected_sequence, None);
    assert_eq!(handling.conflict_resolution(), ConflictResolution::Strict);
}

/// C-0194: a stale expected sequence is ABORTED, classified by a
/// predicate like the other status classes. The setter is named after
/// `Handling.expected_sequence`.
#[tokio::test]
async fn a_stale_sequence_is_classified_as_aborted() {
    let (_backend, client) = connected().await;
    let err = client
        .command_new("orders")
        .with_expected_sequence(3)
        .with_command(format!("{TYPE_PREFIX}AddItem"), &add_item())
        .execute()
        .await
        .expect_err("stale");
    assert!(err.is_aborted(), "got {err:?}");
    assert!(!err.is_precondition_failed());
}

// ---------------------------------------------------------------------------
// Correlated reads across domains (C-0241; query.proto GetCorrelatedEvents).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn correlated_events_cross_domains() {
    let backend = TestBackend::start_tcp().await;
    let corr = common::backend::correlation_for("workflow-123");
    let proto = angzarr_client::uuid_to_proto(corr);
    for domain in ["orders", "inventory"] {
        backend.seed_correlated(
            &common::backend::cover(domain, "r", None),
            "Done",
            1,
            Some(&proto),
        );
    }
    let client = QueryClient::connect(backend.endpoint()).await.unwrap();
    let books: Vec<EventBook> = client.get_correlated_events(corr, None).await.unwrap();
    assert_eq!(books.len(), 2);
}

// ---------------------------------------------------------------------------
// Rules version and the registration hello (C-0533..C-0535, RV-2).
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderState {
    #[prost(uint32, tag = "1")]
    items: u32,
}
impl ::prost::Name for OrderState {
    const NAME: &'static str = "OrderState";
    const PACKAGE: &'static str = "orders.v1";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct AddItem {}
impl ::prost::Name for AddItem {
    const NAME: &'static str = "AddItem";
    const PACKAGE: &'static str = "orders.v1";
}

struct Order;

#[command_handler(domain = "order", state = OrderState)]
impl Order {
    #[handles(AddItem)]
    fn add(&self, _cmd: AddItem, _state: &OrderState, _seq: u64) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

fn order_router(version: &str) -> angzarr_client::router::CommandHandlerRouter {
    let Ok(Built::CommandHandler(router)) = Router::new("order")
        .with_rules_version(version)
        .with_handler(|| Order)
        .build()
    else {
        panic!("command-handler router");
    };
    router
}

/// C-0533: every answer reports the build version.
#[test]
fn every_answer_reports_the_rules_version() {
    let router = order_router("1.4.2+build.7");
    let command = angzarr_client::testing::make_command_book(
        angzarr_client::testing::make_cover("order", [1; 16]),
        angzarr_client::testing::pack_event(&AddItem {}),
        None,
    );
    let response: CommandHandlerResponse = router
        .dispatch(ContextualCommand {
            command: Some(command),
            ..Default::default()
        })
        .expect("handled");
    assert_eq!(response.rules_version, "1.4.2+build.7");
}

/// C-0535: the registration hello carries the version and the state type
/// generated from the state message.
#[test]
fn the_registration_hello_carries_version_and_state_type() {
    let router = order_router("1.4.2+build.7");
    let hellos = router.component_descriptors().expect("hello");
    assert_eq!(hellos.len(), 1);
    assert_eq!(hellos[0].kind(), ComponentKind::Aggregate);
    assert_eq!(hellos[0].rules_version, "1.4.2+build.7");
    assert_eq!(
        hellos[0].state.as_ref().map(|s| s.type_url.as_str()),
        Some("/orders.v1.OrderState")
    );
}

/// The version is baked at build time from the binary's build version
/// (versionator writes VERSION; the client's build helper exports it).
#[test]
fn the_build_version_is_baked_in() {
    let version: &'static str = angzarr_client::rules_version!();
    assert!(!version.is_empty());
}

/// RV-2: a binary without a version cannot start (MISSING_RULES_VERSION).
#[tokio::test]
async fn a_host_without_a_rules_version_refuses_to_start() {
    let err = ComponentHost::new()
        .with_handler(|| Order)
        .with_drain_period(Duration::ZERO)
        .start()
        .await
        .err()
        .expect("refused");
    assert!(err.is_invalid_argument(), "got {err:?}");
}

/// The hello is sent to the coordinator when the host starts; the host
/// exposes what it announced.
#[tokio::test]
async fn a_running_host_announces_its_components() {
    let running = ComponentHost::new()
        .with_router(Built::CommandHandler(order_router("2.0.1")))
        .start()
        .await
        .expect("starts");
    let hellos = running.component_descriptors();
    assert_eq!(hellos[0].name, "order");
    running.shutdown().await.unwrap();
}
