use angzarr_client::{aggregate, projector};

struct T;
#[derive(Default)]
struct State;

#[aggregate(domain = "x", state = State)]
#[projector(name = "p", domains = ["x"])]
impl T {}

fn main() {}
