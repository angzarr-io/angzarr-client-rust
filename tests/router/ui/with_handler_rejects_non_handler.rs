//! `with_handler` must refuse types that don't impl `Handler`.

use angzarr_client::router::Router;

struct Plain;

fn main() {
    let _ = Router::new("x").with_handler(|| Plain);
}
