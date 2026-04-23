//! `#[aggregate]` without the `state` attribute must fail at macro parse time.

use angzarr_client::aggregate;

struct T;

#[aggregate(domain = "x")]
impl T {}

fn main() {}
