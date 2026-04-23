//! `#[saga]` without the `target` attribute must fail at macro parse time.

use angzarr_client::saga;

struct T;

#[saga(name = "s", source = "a")]
impl T {}

fn main() {}
