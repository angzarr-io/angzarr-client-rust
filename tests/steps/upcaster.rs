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
use prost::Message as _;
use prost_types::Any;

use angzarr_client::full_type_url;
use angzarr_client::proto::{event_page, EventPage, UpcastRequest};
use angzarr_client::router::{Built, Router};
use angzarr_client::{state_factory, upcaster, upcasts};

// Compile-time application of each macro — the test binary linking is
// itself evidence that `#[upcaster(...)]`, `#[upcasts(...)]`, and
// `#[state_factory]` accept the attributes below.

#[derive(Clone, prost::Message)]
struct _FromType {}
impl prost::Name for _FromType {
    const NAME: &'static str = "FromType";
    const PACKAGE: &'static str = "angzarr_client.proto.examples";
    fn full_name() -> String {
        "angzarr_client.proto.examples.FromType".into()
    }
    fn type_url() -> String {
        "/angzarr_client.proto.examples.FromType".into()
    }
}

#[derive(Clone, prost::Message)]
struct _ToType {}
impl prost::Name for _ToType {
    const NAME: &'static str = "ToType";
    const PACKAGE: &'static str = "angzarr_client.proto.examples";
    fn full_name() -> String {
        "angzarr_client.proto.examples.ToType".into()
    }
    fn type_url() -> String {
        "/angzarr_client.proto.examples.ToType".into()
    }
}

struct PlayerUpcaster;

#[upcaster(name = "player-v1-to-v2", domain = "player")]
impl PlayerUpcaster {
    #[upcasts(from = _FromType, to = _ToType)]
    fn upgrade(old: _FromType) -> _ToType {
        let _ = old;
        _ToType::default()
    }

    #[state_factory]
    fn empty_state() -> () {}
}

#[derive(Default, World)]
#[world(init = Self::new)]
pub struct UpcasterWorld {
    class_applied: bool,
    method_applied: bool,
    // Chain dispatch (C-0136 / C-0137 — audit finding #43).
    chain_factories: Vec<ChainFactory>,
    chain_incoming: Option<EventPage>,
    chain_response_type_url: Option<String>,
}

impl std::fmt::Debug for UpcasterWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpcasterWorld")
            .field("class_applied", &self.class_applied)
            .field("method_applied", &self.method_applied)
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

#[given(
    regex = r#"^a class "([^"]+)" decorated as an upcaster named "([^"]+)" in domain "([^"]+)"$"#
)]
async fn given_upcaster_class(
    world: &mut UpcasterWorld,
    _cls: String,
    _name: String,
    _domain: String,
) {
    // The fact that `#[upcaster(name = ..., domain = ...)]` above compiled is the
    // assertion. Flag that the step was reached.
    world.class_applied = true;
}

#[given(regex = r#"^a method declared as upcasting from "([^"]+)" to "([^"]+)"$"#)]
async fn given_upcasts_method(world: &mut UpcasterWorld, _from: String, _to: String) {
    world.method_applied = true;
}

#[given("a method declared as a state factory")]
async fn given_state_factory_method(world: &mut UpcasterWorld) {
    world.method_applied = true;
}

#[then("the class declaration compiles without error")]
async fn then_class_compiles(world: &mut UpcasterWorld) {
    assert!(world.class_applied);
}

#[then("the method declaration compiles without error")]
async fn then_method_compiles(world: &mut UpcasterWorld) {
    assert!(world.method_applied);
}

// ---------------------------------------------------------------------------
// C-0136 / C-0137: chain dispatch semantics (audit finding #43).
//
// Concrete prost types stand in for the cucumber's abstract V1/V2/V3
// names. The Python counterpart reuses fixtures.OrderCreatedV1 / etc.;
// here we define module-local prost Messages so the test binary is
// self-contained.
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
struct OrderCreatedV1 {}
impl prost::Name for OrderCreatedV1 {
    const NAME: &'static str = "OrderCreatedV1";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, prost::Message)]
struct OrderCreated {}
impl prost::Name for OrderCreated {
    const NAME: &'static str = "OrderCreated";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, prost::Message)]
struct OrderCompleted {}
impl prost::Name for OrderCompleted {
    const NAME: &'static str = "OrderCompleted";
    const PACKAGE: &'static str = "order";
}

// Stand-in pair for the C-0137 V3→V4 upcaster: a different From-type
// so the upcaster registered for it never fires against an OrderCreated
// chain — exercising the "chain stops when no further upcaster matches"
// branch. Mirrors the Python `PlayerRegisteredV1 → PlayerRegistered`
// in `tests/client/steps/test_upcaster.py`.
#[derive(Clone, PartialEq, prost::Message)]
struct PlayerRegisteredV1 {}
impl prost::Name for PlayerRegisteredV1 {
    const NAME: &'static str = "PlayerRegisteredV1";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, prost::Message)]
struct PlayerRegistered {}
impl prost::Name for PlayerRegistered {
    const NAME: &'static str = "PlayerRegistered";
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
    #[upcasts(from = PlayerRegisteredV1, to = PlayerRegistered)]
    fn migrate(_old: PlayerRegisteredV1) -> PlayerRegistered {
        PlayerRegistered::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChainFactory {
    V1V2,
    V2V3,
    V3V4,
}

fn chain_event_page<T: prost::Message + prost::Name>(evt: T) -> EventPage {
    EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: full_type_url::<T>(),
            value: evt.encode_to_vec(),
        })),
        ..Default::default()
    }
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
    world.chain_incoming = Some(chain_event_page(OrderCreatedV1::default()));
}

#[when("I dispatch the upcast request")]
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

    let page = world.chain_incoming.clone().expect("incoming event not set");
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
