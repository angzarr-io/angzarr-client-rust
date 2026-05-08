//! Single-feature runner for sour-mutants scoring.
//! Runs ONLY builder.feature against BuilderWorld.

#![allow(unused_imports, dead_code)]

mod steps;

use cucumber::World;
use steps::builder::BuilderWorld;

#[tokio::main]
async fn main() {
    BuilderWorld::cucumber()
        .fail_on_skipped()
        .run_and_exit("angzarr-project/features/client/builder.feature")
        .await;
}
