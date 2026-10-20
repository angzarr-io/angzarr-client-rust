//! `#[process_manager]` without `domain` (the workflow stream it owns;
//! options.proto ComponentOptions) must fail at macro parse time.

use angzarr_client::router::process_manager;

struct T;
#[derive(Default)]
struct State;

#[process_manager(name = "pm", output_domains = ["b"], state = State)]
impl T {}

fn main() {}
