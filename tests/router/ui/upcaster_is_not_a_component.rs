//! An upcaster is part of its component, never a component of its own: it
//! cannot be a router's or a host's component.

use angzarr_client::proto::EventBook;
use angzarr_client::router::{command_handler, upcaster, Router};
use angzarr_client::CommandResult;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Ping {}
impl ::prost::Name for Ping {
    const NAME: &'static str = "Ping";
    const PACKAGE: &'static str = "one";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct PingV2 {}
impl ::prost::Name for PingV2 {
    const NAME: &'static str = "PingV2";
    const PACKAGE: &'static str = "one";
}

#[derive(Default)]
struct NoState;

struct A;

#[command_handler(domain = "a", state = NoState)]
impl A {
    #[handles(Ping)]
    fn on_ping(&self, _cmd: Ping, _state: &NoState, _seq: u64) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

struct Lift;

#[upcaster(name = "lift", domain = "a")]
impl Lift {
    #[upcasts(from = Ping, to = PingV2)]
    fn up(_old: Ping) -> PingV2 {
        PingV2 {}
    }
}

fn main() {
    let _ = Router::new("x").with_handler(|| Lift);
    let _ = angzarr_client::ComponentHost::new("1.0.0").with_handler(|| Lift);
}
