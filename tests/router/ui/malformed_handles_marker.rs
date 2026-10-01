//! A `#[handles]` marker whose argument is not a type must fail to compile
//! instead of being silently dropped.

use angzarr_client::command_handler;
use angzarr_client::proto::EventBook;
use angzarr_client::CommandResult;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Cmd {}
impl ::prost::Name for Cmd {
    const NAME: &'static str = "Cmd";
    const PACKAGE: &'static str = "test";
}

#[derive(Default)]
struct State;

struct T;

#[command_handler(domain = "x", state = State)]
impl T {
    #[handles("Cmd")]
    fn handle(&self, _cmd: Cmd, _state: &State, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

fn main() {}
