//! Step defs for features/client/upcaster.feature.
//!
//! C-0123..C-0125 pin the symbol + attribute surface (decorator shape).
//! The macros themselves are applied at module scope below — if
//! attribute parsing failed, the test binary would not compile.
//!
//! C-0136..C-0137 pin dispatch chain semantics (audit finding #43): a
//! V1 event runs through V1→V2 and V2→V3 in registration order; the
//! chain stops when no further upcaster matches the running event type.

use cucumber::{given, then, when, World};

use angzarr_client::full_type_url;
use angzarr_client::proto::{event_page, EventPage, UpcastRequest};
use angzarr_client::router::{Built, HandlerConfig, HandlerKind, Router};
use angzarr_client::upcaster;

// Compile-time application of each macro — the test binary linking is
// itself evidence that `#[upcaster(...)]`, `#[upcasts(...)]`, and
// `#[state_factory]` accept the attributes below.

#[derive(Clone, PartialEq, prost::Message)]
struct OrderCreatedV2 {}
impl prost::Name for OrderCreatedV2 {
    const NAME: &'static str = "OrderCreatedV2";
    const PACKAGE: &'static str = "order";
}

/// Upcaster declared for the declaration-surface scenarios.
struct OrderUpcaster;

#[upcaster(name = "order-v1-to-v2", domain = "order")]
impl OrderUpcaster {
    #[upcasts(from = OrderCreatedV1, to = OrderCreatedV2)]
    fn upgrade(_old: OrderCreatedV1) -> OrderCreatedV2 {
        OrderCreatedV2::default()
    }
}

/// Upcaster that also declares a state factory.
struct StatefulUpcaster;

#[upcaster(name = "order-stateful", domain = "order")]
impl StatefulUpcaster {
    #[upcasts(from = OrderCreatedV1, to = OrderCreatedV2)]
    fn upgrade(_old: OrderCreatedV1) -> OrderCreatedV2 {
        OrderCreatedV2::default()
    }

    #[state_factory]
    fn empty_state() {}
}

#[derive(Default, World)]
#[world(init = Self::new)]
pub struct UpcasterWorld {
    declared: Option<HandlerConfig>,
    expected_name: Option<(String, String)>,
    expected_rule: Option<(String, String)>,
    chain_factories: Vec<ChainFactory>,
    chain_incoming: Option<EventPage>,
    chain_response_type_url: Option<String>,
}

impl std::fmt::Debug for UpcasterWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpcasterWorld")
            .field("declared", &self.declared)
            .field("chain_factories", &self.chain_factories.len())
            .field("chain_incoming", &self.chain_incoming.is_some())
            .field("chain_response_type_url", &self.chain_response_type_url)
            .finish()
    }
}

impl UpcasterWorld {
    fn new() -> Self {
        Self::default()
    }
}

#[given(regex = r#"^an upcaster named "([^"]+)" in domain "([^"]+)"$"#)]
async fn given_upcaster_named(world: &mut UpcasterWorld, name: String, domain: String) {
    world.declared = Some(<OrderUpcaster as HandlerKind>::handler_config());
    world.expected_name = Some((name, domain));
}

#[given(regex = r#"^an upcasting rule from "([^"]+)" to "([^"]+)"$"#)]
async fn given_upcasting_rule(world: &mut UpcasterWorld, from: String, to: String) {
    world.declared = Some(<OrderUpcaster as HandlerKind>::handler_config());
    world.expected_rule = Some((from, to));
}

#[given("an upcaster with a state factory")]
async fn given_upcaster_with_state_factory(world: &mut UpcasterWorld) {
    world.declared = Some(<StatefulUpcaster as HandlerKind>::handler_config());
}

#[then("the declaration is accepted")]
async fn then_declaration_accepted(world: &mut UpcasterWorld) {
    let Some(HandlerConfig::Upcaster {
        name,
        domain,
        upcasts,
    }) = world.declared.clone()
    else {
        panic!("expected an upcaster config, got {:?}", world.declared);
    };
    assert_eq!(upcasts.len(), 1, "upcasts: {upcasts:?}");
    if let Some((n, d)) = &world.expected_name {
        assert_eq!((&name, &domain), (n, d));
    }
    if let Some((from, to)) = &world.expected_rule {
        let (from_url, to_url) = &upcasts[0];
        assert_eq!(from_url.rsplit('.').next(), Some(from.as_str()));
        assert_eq!(to_url.rsplit('.').next(), Some(to.as_str()));
        assert_eq!(from_url, &full_type_url::<OrderCreatedV1>());
        assert_eq!(to_url, &full_type_url::<OrderCreatedV2>());
    }
}

use crate::common::fixtures::{OrderCompleted, OrderCreated, OrderCreatedV1};

#[derive(Clone, PartialEq, prost::Message)]
struct OrderUnrelatedV1 {}
impl prost::Name for OrderUnrelatedV1 {
    const NAME: &'static str = "OrderUnrelatedV1";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, prost::Message)]
struct OrderUnrelated {}
impl prost::Name for OrderUnrelated {
    const NAME: &'static str = "OrderUnrelated";
    const PACKAGE: &'static str = "order";
}

struct V1ToV2;

#[upcaster(name = "upcaster-v1-v2", domain = "order")]
impl V1ToV2 {
    #[upcasts(from = OrderCreatedV1, to = OrderCreated)]
    fn migrate(_old: OrderCreatedV1) -> OrderCreated {
        OrderCreated::default()
    }
}

struct V2ToV3;

#[upcaster(name = "upcaster-v2-v3", domain = "order")]
impl V2ToV3 {
    #[upcasts(from = OrderCreated, to = OrderCompleted)]
    fn migrate(_old: OrderCreated) -> OrderCompleted {
        OrderCompleted::default()
    }
}

struct V3ToV4;

#[upcaster(name = "upcaster-other", domain = "order")]
impl V3ToV4 {
    #[upcasts(from = OrderUnrelatedV1, to = OrderUnrelated)]
    fn migrate(_old: OrderUnrelatedV1) -> OrderUnrelated {
        OrderUnrelated::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChainFactory {
    V1V2,
    V2V3,
    V3V4,
}

fn chain_event_page<T: prost::Message + prost::Name>(evt: &T) -> EventPage {
    angzarr_client::testing::make_event_page(0, angzarr_client::testing::pack_event(evt))
}

#[given("an upcaster registered for V1 → V2")]
async fn given_v1_v2(world: &mut UpcasterWorld) {
    world.chain_factories.push(ChainFactory::V1V2);
}

#[given("an upcaster registered for V2 → V3")]
async fn given_v2_v3(world: &mut UpcasterWorld) {
    world.chain_factories.push(ChainFactory::V2V3);
}

#[given("an upcaster registered for V3 → V4")]
async fn given_v3_v4(world: &mut UpcasterWorld) {
    world.chain_factories.push(ChainFactory::V3V4);
}

#[given("an incoming event of type V1")]
async fn given_incoming_v1(world: &mut UpcasterWorld) {
    world.chain_incoming = Some(chain_event_page(&OrderCreatedV1::default()));
}

#[when("the V1 event is upcasted")]
async fn when_dispatch_chain(world: &mut UpcasterWorld) {
    let mut builder = Router::new("upcaster-chain");
    for f in &world.chain_factories {
        builder = match f {
            ChainFactory::V1V2 => builder.with_handler(|| V1ToV2),
            ChainFactory::V2V3 => builder.with_handler(|| V2ToV3),
            ChainFactory::V3V4 => builder.with_handler(|| V3ToV4),
        };
    }
    let built = builder.build().expect("build");
    let Built::Upcaster(router) = built else {
        panic!("expected Upcaster router");
    };

    let page = world
        .chain_incoming
        .clone()
        .expect("incoming event not set");
    let response = router
        .dispatch(UpcastRequest {
            domain: "order".into(),
            events: vec![page],
        })
        .expect("dispatch");

    let event = response
        .events
        .into_iter()
        .next()
        .and_then(|p| match p.payload {
            Some(event_page::Payload::Event(any)) => Some(any.type_url),
            _ => None,
        })
        .expect("response had no event");
    world.chain_response_type_url = Some(event);
}

#[then(regex = r#"^the emitted event has type (V[123])$"#)]
async fn then_emitted_type(world: &mut UpcasterWorld, version: String) {
    let actual = world
        .chain_response_type_url
        .as_deref()
        .expect("no response captured");
    let expected = match version.as_str() {
        "V1" => full_type_url::<OrderCreatedV1>(),
        "V2" => full_type_url::<OrderCreated>(),
        "V3" => full_type_url::<OrderCompleted>(),
        v => panic!("unknown version label: {v}"),
    };
    assert_eq!(actual, expected, "got {actual:?}, expected {expected:?}");
}
