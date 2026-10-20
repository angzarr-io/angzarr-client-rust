//! A `#[handles]` argument other than `domain` is a precise error.

use angzarr_client::proto::ProcessManagerHandleResponse;
use angzarr_client::router::process_manager;
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
    #[handles(Ping, source = "a")]
    fn on(&self, _e: Ping, _s: &State) -> CommandResult<ProcessManagerHandleResponse> {
        Ok(ProcessManagerHandleResponse::default())
    }
}

fn main() {}
