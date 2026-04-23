//! `#[process_manager]` without `sources` must fail at macro parse time.

use angzarr_client::process_manager;

struct T;
#[derive(Default)]
struct State;

#[process_manager(name = "pm", pm_domain = "d", targets = ["b"], state = State)]
impl T {}

fn main() {}
