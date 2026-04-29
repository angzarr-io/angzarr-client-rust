//! `#[process_manager]` with a `sync_targets` entry that's not in `targets`
//! must fail at macro parse time. Audit #74.

use angzarr_client::process_manager;

struct T;
#[derive(Default)]
struct State;

#[process_manager(
    name = "pm",
    pm_domain = "d",
    sources = ["a"],
    targets = ["inventory"],
    sync_targets = ["shipping"],
    state = State
)]
impl T {}

fn main() {}
