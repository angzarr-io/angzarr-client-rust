//! `#[projector]` without `domains` must fail at macro parse time.

use angzarr_client::projector;

struct T;

#[projector(name = "p")]
impl T {}

fn main() {}
