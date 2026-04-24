//! `#[command_handler]` without the `state` attribute must fail at macro parse time.

use angzarr_client::command_handler;

struct T;

#[command_handler(domain = "x")]
impl T {}

fn main() {}
