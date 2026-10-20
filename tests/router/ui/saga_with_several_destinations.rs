//! A saga has exactly one destination (decision 14.24): one declaring
//! several fails at macro parse time; that is a process manager.

use angzarr_client::router::saga;

struct T;

#[saga(name = "s", input_domain = "a", output_domains = ["b", "c"])]
impl T {}

struct U;

#[saga(name = "u", input_domain = "a", output_domain = ["b", "c"])]
impl U {}

fn main() {}
