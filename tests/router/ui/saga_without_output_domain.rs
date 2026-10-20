//! `#[saga]` without its one destination (`output_domain`) must fail at
//! macro parse time.

use angzarr_client::router::saga;

struct T;

#[saga(name = "s", input_domain = "a")]
impl T {}

fn main() {}
