//! C-0504: a host with no component cannot be started: Rust makes the case
//! unrepresentable, so it does not compile.

use angzarr_client::ComponentHost;

async fn none() {
    let _ = ComponentHost::new("1.0.0").start().await;
}

fn main() {
    let _ = none();
}
