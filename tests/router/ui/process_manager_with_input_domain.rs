//! A process manager declares no `input_domain`: its trigger domains are
//! per event (options.proto: "empty (per-event domains)").

use angzarr_client::router::process_manager;

struct T;
#[derive(Default)]
struct State;

#[process_manager(name = "pm", domain = "d", input_domain = "a", state = State)]
impl T {}

fn main() {}
