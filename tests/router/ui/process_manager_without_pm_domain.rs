//! `#[process_manager]` without `pm_domain` must fail at macro parse time.

use angzarr_client::process_manager;

struct T;
#[derive(Default)]
struct State;

#[process_manager(name = "pm", sources = ["a"], targets = ["b"], state = State)]
impl T {}

fn main() {}
