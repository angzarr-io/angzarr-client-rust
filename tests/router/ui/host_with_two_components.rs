//! C-0579: a host given a second component cannot be expressed: Rust makes
//! the case unrepresentable, so it does not compile.

use angzarr_client::proto::EventBook;
use angzarr_client::router::command_handler;
use angzarr_client::{CommandResult, ComponentHost};

#[derive(Clone, PartialEq, ::prost::Message)]
struct Ping {}
impl ::prost::Name for Ping {
    const NAME: &'static str = "Ping";
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

fn main() {
    let _ = ComponentHost::new("1.0.0")
        .with_handler(|| A)
        .with_handler(|| A);
}
