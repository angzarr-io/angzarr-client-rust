use angzarr_client::aggregate;

struct T;
#[derive(Default)]
struct State;

#[aggregate(domain = "x", state = State)]
#[aggregate(domain = "y", state = State)]
impl T {}

fn main() {}
