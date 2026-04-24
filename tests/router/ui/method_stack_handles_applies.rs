//! Stacking conflicting method-level decorators (@handles + @applies on the
//! same method) must fail at macro parse time.

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
    #[handles(Cmd)]
    #[applies(Cmd)]
    fn handle_create(&self, _cmd: Cmd, _state: &State, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

fn main() {}
