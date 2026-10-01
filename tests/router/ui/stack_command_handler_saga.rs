use angzarr_client::router::{command_handler, saga};

struct T;
#[derive(Default)]
struct State;

#[command_handler(domain = "x", state = State)]
#[saga(name = "s", source = "a", target = "b")]
impl T {}

fn main() {}
