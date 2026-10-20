//! A router with no component cannot be built.

use angzarr_client::router::Router;

fn main() {
    let _ = Router::new("x").build();
}
