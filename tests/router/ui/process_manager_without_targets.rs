//! `#[process_manager]` without `targets` must fail at macro parse time.

use angzarr_client::process_manager;

struct T;
#[derive(Default)]
struct State;

#[process_manager(name = "pm", pm_domain = "d", sources = ["a"], state = State)]
impl T {}

fn main() {}
