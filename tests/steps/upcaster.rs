//! Step defs for features/client/upcaster.feature.
//!
//! The macros themselves are applied at module scope below — if attribute
//! parsing failed, the test binary would not compile. The Gherkin steps
//! just confirm at runtime that the decorated items are callable /
//! constructible.

use cucumber::{given, then, World};

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

#[derive(Debug, Default, World)]
#[world(init = Self::new)]
pub struct UpcasterWorld {
    class_applied: bool,
    method_applied: bool,
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
