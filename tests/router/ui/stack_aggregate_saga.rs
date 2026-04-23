use angzarr_client::{aggregate, saga};

struct T;
#[derive(Default)]
struct State;

#[aggregate(domain = "x", state = State)]
#[saga(name = "s", source = "a", target = "b")]
impl T {}

fn main() {}
