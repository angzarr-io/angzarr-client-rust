use angzarr_client::{command_handler, projector};

struct T;
#[derive(Default)]
struct State;

#[command_handler(domain = "x", state = State)]
#[projector(name = "p", domains = ["x"])]
impl T {}

fn main() {}
