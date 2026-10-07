//! Marker arguments may be paths to the message type, not only bare names.

use angzarr_client::router::command_handler;
use angzarr_client::proto::EventBook;
use angzarr_client::CommandResult;

mod msgs {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Cmd {}
    impl ::prost::Name for Cmd {
        const NAME: &'static str = "Cmd";
        const PACKAGE: &'static str = "test";
    }
}

#[derive(Default)]
struct State;

struct T;

#[command_handler(domain = "x", state = State)]
impl T {
    #[handles(msgs::Cmd)]
    fn handle(&self, _cmd: msgs::Cmd, _state: &State, _seq: u32) -> CommandResult<EventBook> {
        Ok(EventBook::default())
    }
}

fn main() {
    use angzarr_client::router::{HandlerConfig, HandlerKind};
    let HandlerConfig::CommandHandler { handled, .. } = <T as HandlerKind>::handler_config() else {
        panic!("not a command handler");
    };
    assert_eq!(handled, vec![angzarr_client::full_type_url::<msgs::Cmd>()]);
}
