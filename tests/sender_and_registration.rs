//! Sender delivery modes, Handling and the component description, as
//! signed off in `docs/design/sender-and-registration.md`.
//!
//! Spec: angzarr-project — features/client/aggregate_client.feature
//! (C-0194..C-0198), features/client/query_client.feature (C-0241),
//! features/handler-contract/handling.feature (C-0533, C-0535, C-0565),
//! types.proto (Handling, ComponentDescriptor), command_handler.proto (the
//! DECISION / ASYNC / SIMPLE coordinator RPCs) and component.proto
//! (ComponentService.Describe).

mod common;

#[allow(unused_imports)]
use common::backend::{GenericCommand, TestBackend, TYPE_PREFIX};

#[allow(dead_code)]
async fn connected() -> (TestBackend, angzarr_client::CommandHandlerClient) {
    let backend = TestBackend::start_tcp().await;
    backend.add_projector("orders");
    let client = angzarr_client::CommandHandlerClient::connect(backend.endpoint())
        .await
        .expect("connect");
    (backend, client)
}

#[allow(dead_code)]
fn add_item_url() -> String {
    format!("{TYPE_PREFIX}AddItem")
}

#[allow(dead_code)]
fn add_item() -> GenericCommand {
    GenericCommand {
        data: "apple".into(),
        count: 1,
    }
}

// ---------------------------------------------------------------------------
// Delivery modes (C-0197, C-0198; command_handler.proto).
// ---------------------------------------------------------------------------

/// C-0197: ASYNC sends HandleAsync and answers nothing but acceptance,
/// before any decision or projector ran.
#[tokio::test]
async fn execute_async_is_fire_and_forget_over_handle_async() {
    use angzarr_client::CommandBuilderExt;
    let (backend, client) = connected().await;
    let accepted: angzarr_client::Result<()> = client
        .command_new("orders")
        .with_command(add_item_url(), &add_item())
        .execute_async()
        .await;
    accepted.expect("accepted for processing");
    assert_eq!(backend.rpcs(), vec!["HandleAsync"]);
    assert!(backend.projector_runs().is_empty());
}

/// C-0198: SIMPLE sends HandleSync and answers the events plus the
/// synchronous projectors' projections.
#[tokio::test]
async fn execute_sync_waits_for_the_synchronous_projectors() {
    use angzarr_client::CommandBuilderExt;
    let (backend, client) = connected().await;
    let response = client
        .command_new("orders")
        .with_command(add_item_url(), &add_item())
        .execute_sync()
        .await
        .expect("decided and projected");
    assert_eq!(backend.rpcs(), vec!["HandleSync"]);
    assert_eq!(response.events.map(|e| e.pages.len()), Some(1));
    let outcome = response.outcome.expect("outcome");
    assert_eq!(outcome.projections.len(), 1);
    assert_eq!(outcome.projections[0].projector, "orders-projector");
}

/// DECISION (Handle) answers the decision's events and no projections.
#[tokio::test]
async fn execute_is_the_decision() {
    use angzarr_client::CommandBuilderExt;
    let (backend, client) = connected().await;
    let response = client
        .command_new("orders")
        .with_command(add_item_url(), &add_item())
        .execute()
        .await
        .expect("decided");
    assert_eq!(backend.rpcs(), vec!["Handle"]);
    assert_eq!(response.events.map(|e| e.pages.len()), Some(1));
    assert!(response.outcome.is_none_or(|o| o.projections.is_empty()));
}

/// The client and the GatewayClient trait carry the three RPCs by name.
#[tokio::test]
async fn the_gateway_trait_names_the_three_rpcs() {
    use angzarr_client::traits::GatewayClient;
    use angzarr_client::CommandBuilderExt;
    let fake = angzarr_client::testing::RecordingGatewayClient::new();
    let book = fake
        .command_new("orders")
        .with_command(add_item_url(), &add_item())
        .build()
        .unwrap();
    fake.handle_async(book.clone()).await.unwrap();
    fake.handle(book.clone()).await.unwrap();
    fake.handle_sync(book).await.unwrap();
    assert_eq!(fake.call_count("handle_async"), 1);
    assert_eq!(fake.call_count("handle"), 1);
    assert_eq!(fake.call_count("handle_sync"), 1);
}

// ---------------------------------------------------------------------------
// Handling (types.proto, Handling; C-0194..C-0196).
// ---------------------------------------------------------------------------

/// No default: an unset expected sequence sends no Handling, so the
/// coordinator stamps the head.
#[test]
fn an_unset_expected_sequence_is_not_sent() {
    use angzarr_client::CommandBuilderExt;
    let fake = angzarr_client::testing::RecordingGatewayClient::new();
    let book = fake
        .command_new("orders")
        .with_command(add_item_url(), &add_item())
        .build()
        .unwrap();
    assert_eq!(book.handling, None);
}

/// `with_expected_sequence` and `with_conflict_resolution` set the
/// Handling fields of the same names.
#[test]
fn handling_setters_are_named_after_its_fields() {
    use angzarr_client::proto::ConflictResolution;
    use angzarr_client::CommandBuilderExt;
    let fake = angzarr_client::testing::RecordingGatewayClient::new();
    let book = fake
        .command_new("orders")
        .with_expected_sequence(7)
        .with_conflict_resolution(ConflictResolution::Strict)
        .with_command(add_item_url(), &add_item())
        .build()
        .unwrap();
    let handling = book.handling.expect("handling");
    assert_eq!(handling.expected_sequence, Some(7));
    assert_eq!(handling.conflict_resolution(), ConflictResolution::Strict);
}

/// C-0194: a stale expected sequence is ABORTED, classified by a
/// predicate like the other status classes.
#[tokio::test]
async fn a_stale_sequence_is_classified_as_aborted() {
    use angzarr_client::CommandBuilderExt;
    let (_backend, client) = connected().await;
    let err = client
        .command_new("orders")
        .with_expected_sequence(3)
        .with_command(add_item_url(), &add_item())
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
    let client = angzarr_client::QueryClient::connect(backend.endpoint())
        .await
        .unwrap();
    backend.seed_correlated(
        &common::backend::cover("orders", "r", Some("trial")),
        "Done",
        1,
        Some(&proto),
    );
    let books = client.get_correlated_events(corr, None).await.unwrap();
    assert_eq!(books.len(), 2, "main timeline only");
    let trial = client
        .get_correlated_events(corr, Some("trial".into()))
        .await
        .unwrap();
    assert_eq!(trial.len(), 1);
    assert_eq!(
        trial[0].cover.as_ref().and_then(|c| c.edition.as_deref()),
        Some("trial")
    );
}

// ---------------------------------------------------------------------------
// The rules version and the component description (C-0533, C-0535, C-0565;
// component.proto; RV-2).
// ---------------------------------------------------------------------------

mod describe {
    use angzarr_client::proto::component_service_client::ComponentServiceClient;
    use angzarr_client::proto::{ComponentKind, DescribeRequest, EventBook, SagaResponse};
    use angzarr_client::router::{command_handler, saga};
    use angzarr_client::{CommandResult, ComponentHost, HostAddress, ServerConfig};

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct OrderState {
        #[prost(uint32, tag = "1")]
        items: u32,
    }
    impl ::prost::Name for OrderState {
        const NAME: &'static str = "OrderState";
        const PACKAGE: &'static str = "orders.v1";
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct AddItem {}
    impl ::prost::Name for AddItem {
        const NAME: &'static str = "AddItem";
        const PACKAGE: &'static str = "orders.v1";
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct ItemAdded {}
    impl ::prost::Name for ItemAdded {
        const NAME: &'static str = "ItemAdded";
        const PACKAGE: &'static str = "orders.v1";
    }

    pub struct Order;

    #[command_handler(domain = "order", state = OrderState)]
    impl Order {
        #[handles(AddItem)]
        fn add(&self, _cmd: AddItem, _state: &OrderState, _seq: u64) -> CommandResult<EventBook> {
            Ok(EventBook::default())
        }
    }

    pub struct OrderFulfillment;

    #[saga(name = "OrderFulfillment", source = "order", target = "shipping")]
    impl OrderFulfillment {
        #[handles(ItemAdded)]
        fn on_added(&self, _evt: ItemAdded) -> CommandResult<SagaResponse> {
            Ok(SagaResponse::default())
        }
    }

    fn ephemeral() -> ServerConfig {
        ServerConfig {
            port: 0,
            uds_path: None,
        }
    }

    async fn channel(address: &HostAddress) -> tonic::transport::Channel {
        let HostAddress::Tcp(addr) = address else {
            panic!("tcp host");
        };
        tonic::transport::Endpoint::from_shared(format!("http://127.0.0.1:{}", addr.port()))
            .unwrap()
            .connect()
            .await
            .unwrap()
    }

    /// RV-2: the version is baked at build time from the binary's
    /// versionator-maintained VERSION file.
    #[test]
    fn the_build_version_is_baked_in() {
        let version: &'static str = angzarr_client::rules_version!();
        assert_eq!(version, include_str!("../VERSION").trim());
    }

    /// RV-2: a host without a version refuses to start
    /// (MISSING_RULES_VERSION).
    #[tokio::test]
    async fn a_host_without_a_rules_version_refuses_to_start() {
        let err = ComponentHost::new("")
            .with_handler(|| Order)
            .with_transport(ephemeral())
            .start()
            .await
            .expect_err("refused");
        assert!(err.is_invalid_argument(), "got {err:?}");
        assert_eq!(err.code(), "MISSING_RULES_VERSION");
    }

    /// C-0533: every answer reports the host's version.
    #[tokio::test]
    async fn every_answer_reports_the_hosts_rules_version() {
        use angzarr_client::proto::command_handler_service_client::CommandHandlerServiceClient;
        use angzarr_client::proto::ContextualCommand;
        let running = ComponentHost::new("1.4.2+build.7")
            .with_handler(|| Order)
            .with_transport(ephemeral())
            .start()
            .await
            .expect("starts");
        let command = angzarr_client::testing::make_command_book(
            angzarr_client::testing::make_cover("order", [1; 16]),
            angzarr_client::testing::pack_event(&AddItem {}),
            None,
        );
        let response = CommandHandlerServiceClient::new(channel(running.address()).await)
            .handle(ContextualCommand {
                command: Some(command),
                ..Default::default()
            })
            .await
            .expect("handled")
            .into_inner();
        assert_eq!(response.rules_version, "1.4.2+build.7");
        running.shutdown().await.unwrap();
    }

    /// What `Describe` answers for a host serving `host`'s component.
    async fn describe(
        host: ComponentHost<angzarr_client::HostComponent>,
    ) -> Result<angzarr_client::proto::ComponentDescriptor, tonic::Status> {
        let running = host
            .with_transport(ephemeral())
            .start()
            .await
            .expect("starts");
        assert!(running
            .services()
            .contains(&"io.angzarr.v1.ComponentService".to_string()));
        let described = ComponentServiceClient::new(channel(running.address()).await)
            .describe(DescribeRequest {})
            .await
            .map(|r| r.into_inner());
        running.shutdown().await.unwrap();
        described
    }

    /// C-0535: the coordinator pulls the description of the host's one
    /// component: the host's version and the state type of its state
    /// message.
    #[tokio::test]
    async fn describe_answers_the_aggregates_description() {
        let described = describe(ComponentHost::new("1.4.2+build.7").with_handler(|| Order))
            .await
            .expect("described");
        assert_eq!(described.name, "order");
        assert_eq!(described.kind(), ComponentKind::Aggregate);
        assert_eq!(described.rules_version, "1.4.2+build.7");
        assert_eq!(
            described.state.map(|s| s.type_url),
            Some("/orders.v1.OrderState".to_string())
        );
    }

    /// C-0565 (one component per process): a saga is described with its
    /// input domain and no state.
    #[tokio::test]
    async fn describe_answers_the_sagas_description() {
        let described = describe(ComponentHost::new("2.0.1").with_handler(|| OrderFulfillment))
            .await
            .expect("described");
        assert_eq!(described.name, "OrderFulfillment");
        assert_eq!(described.kind(), ComponentKind::Saga);
        assert_eq!(described.rules_version, "2.0.1");
        assert_eq!(
            described
                .inputs
                .iter()
                .map(|t| t.domain.as_str())
                .collect::<Vec<_>>(),
            vec!["order"]
        );
        assert_eq!(described.state, None);
    }

    pub struct Lift;

    #[angzarr_client::router::upcaster(name = "lift", domain = "order")]
    impl Lift {
        #[upcasts(from = AddItem, to = ItemAdded)]
        fn up(_old: AddItem) -> ItemAdded {
            ItemAdded {}
        }
    }

    /// The aggregate's host serves UpcasterService alongside its
    /// aggregate when the aggregate has an upcaster chain, and still
    /// describes the aggregate.
    #[tokio::test]
    async fn an_aggregate_host_serves_its_upcaster_chain() {
        use angzarr_client::proto::upcaster_service_client::UpcasterServiceClient;
        use angzarr_client::proto::{EventPage, UpcastRequest};
        let built = angzarr_client::router::Router::new("order")
            .with_handler(|| Order)
            .with_upcaster(|| Lift)
            .build()
            .expect("builds");
        let running = ComponentHost::new("1.4.2+build.7")
            .with_router(built)
            .with_transport(ephemeral())
            .start()
            .await
            .expect("starts");
        assert!(running
            .services()
            .contains(&"io.angzarr.v1.UpcasterService".to_string()));
        let ch = channel(running.address()).await;
        let page = EventPage {
            payload: Some(angzarr_client::testing::pack_event(&AddItem {})),
            ..Default::default()
        };
        let upcast = UpcasterServiceClient::new(ch.clone())
            .upcast(UpcastRequest {
                domain: "order".into(),
                events: vec![page],
            })
            .await
            .expect("upcast")
            .into_inner();
        assert_eq!(
            upcast.events[0]
                .payload
                .as_ref()
                .map(|a| a.type_url.as_str()),
            Some("/orders.v1.ItemAdded")
        );
        let described = ComponentServiceClient::new(ch)
            .describe(DescribeRequest {})
            .await
            .expect("described")
            .into_inner();
        assert_eq!(described.name, "order");
        running.shutdown().await.unwrap();
    }

    /// Without a chain the host serves no UpcasterService.
    #[tokio::test]
    async fn an_aggregate_host_without_a_chain_serves_no_upcaster() {
        let running = ComponentHost::new("1.0.0")
            .with_handler(|| Order)
            .with_transport(ephemeral())
            .start()
            .await
            .expect("starts");
        assert!(!running
            .services()
            .contains(&"io.angzarr.v1.UpcasterService".to_string()));
        running.shutdown().await.unwrap();
    }
}
