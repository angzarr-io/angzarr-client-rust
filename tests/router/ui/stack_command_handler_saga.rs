use angzarr_client::router::{command_handler, saga};

struct T;
#[derive(Default)]
struct State;

#[command_handler(domain = "x", state = State)]
#[saga(name = "s", input_domain = "a", output_domain = "b")]
impl T {}

fn main() {}
