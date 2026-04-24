//! Step defs for features/client/decorators.feature.
//!
//! Verifies the `#[command_handler]` / `#[saga]` macros produce a
//! [`HandlerConfig`] whose shape matches the Python `@command_handler` /
//! `@saga` decorator output.

use std::sync::Arc;

use cucumber::{given, then, World};

use angzarr_client::{
    command_handler, handles, saga, upcaster, upcasts, Handler, HandlerConfig, HandlerRequest,
    HandlerResponse,
};
use angzarr_client::proto::{BusinessResponse, CommandBook, ContextualCommand};

#[derive(Debug, Default)]
pub struct DecoratorsWorld {
    config: Option<HandlerConfig>,
}

#[derive(cucumber::World)]
#[world(init = Self::new)]
pub struct DecoratorsWorldCucumber(Arc<std::sync::Mutex<DecoratorsWorld>>);

impl std::fmt::Debug for DecoratorsWorldCucumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecoratorsWorldCucumber").finish()
    }
}

impl DecoratorsWorldCucumber {
    fn new() -> Self {
        Self(Arc::new(std::sync::Mutex::new(DecoratorsWorld::default())))
    }

    fn set_config(&self, cfg: HandlerConfig) {
        self.0.lock().unwrap().config = Some(cfg);
    }

    fn kind_str(&self) -> &'static str {
        let g = self.0.lock().unwrap();
        match g.config.as_ref().expect("no config") {
            HandlerConfig::CommandHandler { .. } => "command_handler",
            HandlerConfig::Saga { .. } => "saga",
            HandlerConfig::ProcessManager { .. } => "process_manager",
            HandlerConfig::Projector { .. } => "projector",
            HandlerConfig::Upcaster { .. } => "upcaster",
        }
    }

    fn with_cmd<F: FnOnce(&str) -> R, R>(&self, f: F) -> R {
        let g = self.0.lock().unwrap();
        match g.config.as_ref().expect("no config") {
            HandlerConfig::CommandHandler { domain, .. } => f(domain),
            _ => panic!("expected command_handler config"),
        }
    }

    fn with_saga<F: FnOnce(&str, &str, &str) -> R, R>(&self, f: F) -> R {
        let g = self.0.lock().unwrap();
        match g.config.as_ref().expect("no config") {
            HandlerConfig::Saga {
                name,
                source,
                target,
                ..
            } => f(name, source, target),
            _ => panic!("expected saga config"),
        }
    }

    fn with_upcaster<F: FnOnce(&str, &str) -> R, R>(&self, f: F) -> R {
        let g = self.0.lock().unwrap();
        match g.config.as_ref().expect("no config") {
            HandlerConfig::Upcaster { name, domain, .. } => f(name, domain),
            _ => panic!("expected upcaster config"),
        }
    }
}

#[derive(Default, Clone)]
pub struct OrderState;

#[derive(Clone, prost::Message)]
struct FakeCmd {}
impl prost::Name for FakeCmd {
    const NAME: &'static str = "FakeCmd";
    const PACKAGE: &'static str = "angzarr_client.proto.examples";
    fn full_name() -> String {
        "angzarr_client.proto.examples.FakeCmd".into()
    }
    fn type_url() -> String {
        "/angzarr_client.proto.examples.FakeCmd".into()
    }
}

struct OrderHandler;

#[command_handler(domain = "order", state = OrderState)]
impl OrderHandler {
    #[handles(FakeCmd)]
    fn handle_fake(&self, _cmd: FakeCmd, _state: &OrderState, _seq: u32)
        -> angzarr_client::CommandResult<angzarr_client::proto::EventBook>
    {
        unreachable!("decorators.feature does not dispatch")
    }
}

struct OrderFulfillmentSaga;

#[saga(name = "OrderFulfillment", source = "order", target = "inventory")]
impl OrderFulfillmentSaga {
    #[handles(FakeCmd)]
    fn translate(&self, _evt: FakeCmd)
        -> angzarr_client::CommandResult<angzarr_client::proto::SagaResponse>
    {
        unreachable!("decorators.feature does not dispatch")
    }
}

struct PlayerUpcaster;

#[upcaster(name = "player-v1-to-v2", domain = "player")]
impl PlayerUpcaster {
    #[upcasts(from = FakeCmd, to = FakeCmd)]
    fn noop(old: FakeCmd) -> FakeCmd {
        old
    }
}

#[given(
    regex = r#"^a class "([^"]+)" decorated as a command handler for domain "([^"]+)" with state (\S+)$"#
)]
async fn given_command_handler(
    world: &mut DecoratorsWorldCucumber,
    _name: String,
    _domain: String,
    _state_type: String,
) {
    let cfg = OrderHandler.config();
    world.set_config(cfg);
}

#[given(
    regex = r#"^a class "([^"]+)" decorated as a saga named "([^"]+)" from "([^"]+)" to "([^"]+)"$"#
)]
async fn given_saga(
    world: &mut DecoratorsWorldCucumber,
    _name: String,
    _saga_name: String,
    _source: String,
    _target: String,
) {
    let cfg = OrderFulfillmentSaga.config();
    world.set_config(cfg);
}

#[then(regex = r#"^the class exposes a handler config of kind "([^"]+)"$"#)]
async fn then_kind(world: &mut DecoratorsWorldCucumber, kind: String) {
    assert_eq!(world.kind_str(), kind);
}

#[then(regex = r#"^the handler config's domain is "([^"]+)"$"#)]
async fn then_domain(world: &mut DecoratorsWorldCucumber, expected: String) {
    world.with_cmd(|d| assert_eq!(d, expected.as_str()));
}

#[then(regex = r#"^the handler config's saga name is "([^"]+)"$"#)]
async fn then_saga_name(world: &mut DecoratorsWorldCucumber, expected: String) {
    world.with_saga(|n, _, _| assert_eq!(n, expected.as_str()));
}

#[then(regex = r#"^the handler config's saga source is "([^"]+)"$"#)]
async fn then_saga_source(world: &mut DecoratorsWorldCucumber, expected: String) {
    world.with_saga(|_, s, _| assert_eq!(s, expected.as_str()));
}

#[then(regex = r#"^the handler config's saga target is "([^"]+)"$"#)]
async fn then_saga_target(world: &mut DecoratorsWorldCucumber, expected: String) {
    world.with_saga(|_, _, t| assert_eq!(t, expected.as_str()));
}

#[given(
    regex = r#"^a class "([^"]+)" decorated as an upcaster named "([^"]+)" in domain "([^"]+)"$"#
)]
async fn given_upcaster(
    world: &mut DecoratorsWorldCucumber,
    _cls: String,
    _name: String,
    _domain: String,
) {
    let cfg = PlayerUpcaster.config();
    world.set_config(cfg);
}

#[then(regex = r#"^the handler config's upcaster name is "([^"]+)"$"#)]
async fn then_upcaster_name(world: &mut DecoratorsWorldCucumber, expected: String) {
    world.with_upcaster(|n, _| assert_eq!(n, expected.as_str()));
}

#[then(regex = r#"^the handler config's upcaster domain is "([^"]+)"$"#)]
async fn then_upcaster_domain(world: &mut DecoratorsWorldCucumber, expected: String) {
    world.with_upcaster(|_, d| assert_eq!(d, expected.as_str()));
}

// Silence unused-import warnings on types we intentionally reference via
// the proc-macro expansion of OrderHandler / OrderFulfillmentSaga.
#[allow(dead_code)]
fn _touch() {
    let _: fn(ContextualCommand) -> HandlerRequest = HandlerRequest::CommandHandler;
    let _: fn(BusinessResponse) -> HandlerResponse = HandlerResponse::CommandHandler;
    let _: Option<CommandBook> = None;
}
