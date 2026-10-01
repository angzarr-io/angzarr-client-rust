//! A `#[rejected]` marker missing its `command` must fail to compile
//! instead of being silently dropped.

use angzarr_client::router::command_handler;
use angzarr_client::proto::{BusinessResponse, Notification};
use angzarr_client::CommandResult;

#[derive(Default)]
struct State;

struct T;

#[command_handler(domain = "x", state = State)]
impl T {
    #[rejected(domain = "inventory")]
    fn compensate(&self, _n: &Notification, _state: &State) -> CommandResult<BusinessResponse> {
        Ok(BusinessResponse::default())
    }
}

fn main() {}
