//! Cucumber runner for the angzarr client spec.
//!
//! Runs every feature in `angzarr-project/features/client/` and
//! `angzarr-project/parity/client/` against its step world. The process
//! exits non-zero when any scenario fails, any step is undefined or
//! skipped, any step match is ambiguous, any feature fails to parse, or a
//! feature file in those directories has no registered world.
//!
//! [`PENDING`] lists scenarios the library does not satisfy yet, each with
//! the finding that tracks it. They run separately and must fail: a pending
//! scenario that passes fails the gate until it is removed from the list.
//!
//! ```bash
//! cargo test --test features
//! ANGZARR_FEATURES=saga,router cargo test --test features   # subset
//! ```

mod common;
mod steps;

use std::collections::BTreeSet;
use std::path::Path;

use cucumber::{writer::Stats, World};

use steps::aggregate_client_steps::AggregateClientWorld;
use steps::builder_steps::BuilderWorld;
use steps::command_builder::CommandBuilderWorld;
use steps::command_handler_steps::CommandHandlerWorld;
use steps::compensation_steps::CompensationWorld;
use steps::connection::ConnectionWorld;
use steps::decorators::DecoratorsWorldCucumber;
use steps::destinations::DestinationsWorld;
use steps::domain_client_steps::DomainClientWorld;
use steps::error_handling::ErrorHandlingWorld;
use steps::event_decoding::EventDecodingWorld;
use steps::identity::IdentityWorld;
use steps::multi_handler_steps::MultiHandlerWorld;
use steps::parity::ParityWorld;
use steps::process_manager_steps::ProcessManagerWorld;
use steps::projector_steps::ProjectorWorld;
use steps::query_builder::QueryBuilderWorld;
use steps::query_client_steps::QueryClientWorld;
use steps::rejected_compensation_steps::RejectedCompensationWorld;
use steps::rejection_steps::RejectionWorld;
use steps::retry::RetryWorld;
use steps::router_steps::RouterWorld;
use steps::saga_steps::SagaWorld;
use steps::speculative_client_steps::SpeculativeClientWorld;
use steps::testing::TestingWorld;
use steps::upcaster_steps::UpcasterWorld;
use steps::validation_steps::ValidationWorld;
use steps::wire_parity::WireParityWorld;

const CLIENT_DIR: &str = "angzarr-project/features/client";
const PARITY_DIR: &str = "angzarr-project/parity/client";

/// Scenarios (by `@C-NNNN` tag) the library does not satisfy yet, with the
/// finding that tracks each.
const PENDING: &[(&str, &str)] = &[
    ("C-0336", "no configurable connect timeout"),
    ("C-0337", "no configurable HTTP/2 keep-alive"),
];

/// Outcome of one feature run.
struct SuiteResult {
    path: String,
    problems: Vec<String>,
}

fn pending_reason(tags: &[String]) -> Option<&'static str> {
    PENDING
        .iter()
        .find(|(id, _)| tags.iter().any(|t| t == id))
        .map(|(_, why)| *why)
}

/// Comma-separated feature stems from `ANGZARR_FEATURES`; `None` runs all.
fn selected_features() -> Option<BTreeSet<String>> {
    let raw = std::env::var("ANGZARR_FEATURES").ok()?;
    Some(
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

/// Run one feature file against world `W`; skipped (undefined) steps count
/// as failures. Pending scenarios are excluded from the main run and then
/// each re-run alone, where it must fail.
macro_rules! run_suite {
    ($selected:expr, $world:ty, $path:expr) => {{
        let path: String = $path;
        let stem = Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut problems = Vec::new();
        if $selected
            .as_ref()
            .map_or(true, |s: &BTreeSet<String>| s.contains(&stem))
        {
            println!("\n=== {} ===\n", path);
            let writer = <$world>::cucumber()
                .fail_on_skipped()
                .filter_run(path.clone(), |_, _, sc| pending_reason(&sc.tags).is_none())
                .await;
            if writer.execution_has_failed()
                || writer.skipped_steps() > 0
                || writer.parsing_errors() > 0
            {
                problems.push(format!("{path}: failed"));
            }
            for (id, why) in PENDING {
                let tag = id.to_string();
                let has_tag = std::fs::read_to_string(&path)
                    .map(|text| text.contains(&format!("@{tag}")))
                    .unwrap_or(false);
                if !has_tag {
                    continue;
                }
                println!("\n--- pending {id} ({why}) must fail ---\n");
                let writer = <$world>::cucumber()
                    .fail_on_skipped()
                    .filter_run(path.clone(), move |_, _, sc| sc.tags.contains(&tag))
                    .await;
                if !writer.execution_has_failed() {
                    problems.push(format!(
                        "{path}: pending {id} passes; remove it from PENDING"
                    ));
                }
            }
        }
        SuiteResult { path, problems }
    }};
}

/// Feature file names (without `.feature`) present in `dir`.
fn feature_stems(dir: &str) -> BTreeSet<String> {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read feature directory {dir}: {e}"));
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "feature" {
                return None;
            }
            Some(path.file_stem()?.to_string_lossy().into_owned())
        })
        .collect()
}

#[tokio::main]
async fn main() {
    let client = |stem: &str| format!("{CLIENT_DIR}/{stem}.feature");
    let parity = |stem: &str| format!("{PARITY_DIR}/{stem}.feature");

    let sel = selected_features();
    let results = vec![
        // features/client — router / dispatch and client-surface tiers.
        run_suite!(sel, AggregateClientWorld, client("aggregate_client")),
        run_suite!(sel, BuilderWorld, client("builder")),
        run_suite!(sel, CommandHandlerWorld, client("command_handler")),
        run_suite!(sel, CompensationWorld, client("compensation")),
        run_suite!(sel, DomainClientWorld, client("domain-client")),
        run_suite!(sel, MultiHandlerWorld, client("multi_handler")),
        run_suite!(sel, ProcessManagerWorld, client("process_manager")),
        run_suite!(sel, ProjectorWorld, client("projector")),
        run_suite!(sel, QueryClientWorld, client("query_client")),
        run_suite!(
            sel,
            RejectedCompensationWorld,
            client("rejected_compensation")
        ),
        run_suite!(sel, RejectionWorld, client("rejection")),
        run_suite!(sel, RouterWorld, client("router")),
        run_suite!(sel, SagaWorld, client("saga")),
        run_suite!(sel, SpeculativeClientWorld, client("speculative_client")),
        run_suite!(sel, UpcasterWorld, client("upcaster")),
        run_suite!(sel, ValidationWorld, client("validation")),
        // parity/client — cross-language surface parity.
        run_suite!(sel, CommandBuilderWorld, parity("command_builder")),
        run_suite!(sel, ConnectionWorld, parity("connection")),
        run_suite!(sel, DecoratorsWorldCucumber, parity("decorators")),
        run_suite!(sel, DestinationsWorld, parity("destinations")),
        run_suite!(sel, ErrorHandlingWorld, parity("error_handling")),
        run_suite!(sel, EventDecodingWorld, parity("event_decoding")),
        run_suite!(sel, IdentityWorld, parity("identity")),
        run_suite!(sel, ParityWorld, parity("parity")),
        run_suite!(sel, QueryBuilderWorld, parity("query_builder")),
        run_suite!(sel, RetryWorld, parity("retry")),
        run_suite!(sel, TestingWorld, parity("testing")),
        run_suite!(sel, WireParityWorld, parity("wire_parity")),
    ];

    let mut problems: Vec<String> = Vec::new();
    let mut known_ids: BTreeSet<&str> = BTreeSet::new();
    let covered: BTreeSet<String> = results.iter().map(|r| r.path.clone()).collect();
    if sel.is_none() {
        for dir in [CLIENT_DIR, PARITY_DIR] {
            for stem in feature_stems(dir) {
                let path = format!("{dir}/{stem}.feature");
                if !covered.contains(&path) {
                    problems.push(format!("{path}: no step world registered"));
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                for (id, _) in PENDING {
                    if text.contains(&format!("@{id}")) {
                        known_ids.insert(*id);
                    }
                }
            }
        }
        for (id, _) in PENDING {
            if !known_ids.contains(id) {
                problems.push(format!("PENDING {id} matches no scenario"));
            }
        }
    }
    for r in results {
        if !Path::new(&r.path).is_file() {
            problems.push(format!("{}: feature file missing", r.path));
        }
        problems.extend(r.problems);
    }

    if !problems.is_empty() {
        eprintln!("\n=== FEATURE GATE FAILED ===");
        for p in &problems {
            eprintln!("  {p}");
        }
        std::process::exit(1);
    }
    println!("\n=== feature gate passed ({} pending) ===", PENDING.len());
}
