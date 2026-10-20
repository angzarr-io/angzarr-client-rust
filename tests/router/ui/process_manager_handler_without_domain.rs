//! A process manager's `#[handles]` names its trigger's domain
//! (`#[handles(Event, domain = "...")]`, the spec's per-event domains); one
//! without fails at macro expansion, and no other kind's `#[handles]` takes
//! a domain.

use angzarr_client::proto::{EventBook, ProcessManagerHandleResponse};
use angzarr_client::router::{command_handler, process_manager};
use angzarr_client::CommandResult;

#[derive(Clone, PartialEq, ::prost::Message)]
struct Ping {}
impl ::prost::Name for Ping {
    const NAME: &'static str = "Ping";
    const PACKAGE: &'static str = "pm";
}

#[derive(Default)]
struct State;

struct T;

#[process_manager(name = "pm", domain = "d", output_domain = "b", state = State)]
impl T {
    #[handles(Ping)]
    fn on(&self, _e: Ping, _s: &State) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
}

struct A;

#[command_handler(domain = "a", state = State)]
impl A {
    #[handles(Ping, domain = "x")]
    fn on(&self, _c: Ping, _s: &State, _seq: u64) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

fn main() {}
