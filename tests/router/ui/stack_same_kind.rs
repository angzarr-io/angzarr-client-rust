use angzarr_client::router::command_handler;

struct T;
#[derive(Default)]
struct State;

#[command_handler(domain = "x", state = State)]
#[command_handler(domain = "y", state = State)]
impl T {}

fn main() {}
