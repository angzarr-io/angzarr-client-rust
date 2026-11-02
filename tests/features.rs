//! Cucumber feature tests for the angzarr-client library.
//!
//! These tests verify client library behavior using Gherkin scenarios.
//! Run with:
//!
//! ```bash
//! cargo test --test features
//! ```
//!
//! Every suite runs, then the process exits non-zero if any of them failed:
//! a scenario with a failing step, a step that matches no binding (cucumber's
//! `fail_on_skipped`), or a feature file that does not parse. cucumber's
//! `run` only reports; the verdict is its writer's `execution_has_failed`, as
//! angzarr-embedded's conformance runner and angzarr-cli's codegen runner
//! read it.

mod steps;

use std::fmt::Debug;

use cucumber::writer::Stats as _;
use cucumber::World;
use steps::aggregate_client::AggregateClientWorld;
use steps::builder::BuilderWorld;
use steps::command_builder::CommandBuilderWorld;
use steps::command_handler::CommandHandlerWorld;
use steps::compensation::CompensationWorld;
use steps::connection::ConnectionWorld;
use steps::decorators::DecoratorsWorldCucumber;
use steps::destinations::DestinationsWorld;
use steps::domain_client::DomainClientWorld;
use steps::error_handling::ErrorHandlingWorld;
use steps::event_decoding::EventDecodingWorld;
use steps::fact_flow::FactFlowWorld;
use steps::identity::IdentityWorld;
use steps::merge_strategy::MergeStrategyWorld;
use steps::multi_handler::MultiHandlerWorld;
use steps::parity::ParityWorld;
use steps::process_manager::ProcessManagerWorld;
use steps::projector::ProjectorWorld;
use steps::query_builder::QueryBuilderWorld;
use steps::query_client::QueryClientWorld;
use steps::rejected_compensation::RejectedCompensationWorld;
use steps::rejection::RejectionWorld;
use steps::retry::RetryWorld;
use steps::saga::SagaWorld;
use steps::speculative_client::SpeculativeClientWorld;
use steps::state_building::StateBuildingWorld;
use steps::testing::TestingWorld;
use steps::upcaster::UpcasterWorld;
use steps::validation::ValidationWorld;
use steps::wire_parity::WireParityWorld;

/// Run the feature at `path` with `W`'s steps, recording `path` in `failed`
/// if it failed.
async fn run<W>(path: &'static str, failed: &mut Vec<&'static str>)
where
    W: World + Debug + cucumber::codegen::WorldInventory,
{
    let writer = W::cucumber().fail_on_skipped().run(path).await;
    if writer.execution_has_failed() {
        failed.push(path);
    }
}

#[tokio::main]
async fn main() {
    let mut failed = Vec::new();

    // Run Parity tests
    println!("\n=== Running Parity Tests ===\n");
    run::<ParityWorld>(
        "angzarr-project/features/client/parity.feature",
        &mut failed,
    )
    .await;

    // Run Identity tests
    println!("\n=== Running Identity Tests ===\n");
    run::<IdentityWorld>(
        "angzarr-project/features/client/identity.feature",
        &mut failed,
    )
    .await;

    // Run Testing tests
    println!("\n=== Running Testing Tests ===\n");
    run::<TestingWorld>(
        "angzarr-project/features/client/testing.feature",
        &mut failed,
    )
    .await;

    // Run Connection tests
    println!("\n=== Running Connection Tests ===\n");
    run::<ConnectionWorld>(
        "angzarr-project/features/client/connection.feature",
        &mut failed,
    )
    .await;

    // Run DomainClient tests
    println!("\n=== Running DomainClient Tests ===\n");
    run::<DomainClientWorld>(
        "angzarr-project/features/client/domain-client.feature",
        &mut failed,
    )
    .await;

    // Run AggregateClient tests
    println!("\n=== Running AggregateClient Tests ===\n");
    run::<AggregateClientWorld>(
        "angzarr-project/features/client/aggregate_client.feature",
        &mut failed,
    )
    .await;

    // Run QueryClient tests
    println!("\n=== Running QueryClient Tests ===\n");
    run::<QueryClientWorld>(
        "angzarr-project/features/client/query_client.feature",
        &mut failed,
    )
    .await;

    // Run SpeculativeClient tests
    println!("\n=== Running SpeculativeClient Tests ===\n");
    run::<SpeculativeClientWorld>(
        "angzarr-project/features/client/speculative_client.feature",
        &mut failed,
    )
    .await;

    // Run CommandBuilder tests
    println!("\n=== Running CommandBuilder Tests ===\n");
    run::<CommandBuilderWorld>(
        "angzarr-project/features/client/command_builder.feature",
        &mut failed,
    )
    .await;

    // Run QueryBuilder tests
    println!("\n=== Running QueryBuilder Tests ===\n");
    run::<QueryBuilderWorld>(
        "angzarr-project/features/client/query_builder.feature",
        &mut failed,
    )
    .await;

    // Run ErrorHandling tests
    println!("\n=== Running ErrorHandling Tests ===\n");
    run::<ErrorHandlingWorld>(
        "angzarr-project/features/client/error_handling.feature",
        &mut failed,
    )
    .await;

    // Legacy router.feature is retired in the Rust tier — routing behavior is
    // covered by the TIER5 suites below (builder / command_handler /
    // multi_handler / process_manager / projector / rejection / saga /
    // rejected_compensation / validation). The feature file remains in
    // angzarr-project for other languages.

    // Run EventDecoding tests
    println!("\n=== Running EventDecoding Tests ===\n");
    run::<EventDecodingWorld>(
        "angzarr-project/features/client/event_decoding.feature",
        &mut failed,
    )
    .await;

    // Run Compensation tests
    println!("\n=== Running Compensation Tests ===\n");
    run::<CompensationWorld>(
        "angzarr-project/features/client/compensation.feature",
        &mut failed,
    )
    .await;

    // ------------------------------------------------------------------
    // TIER5 Router feature suite.
    // ------------------------------------------------------------------

    // Run Builder tests
    println!("\n=== Running Builder Tests ===\n");
    run::<BuilderWorld>(
        "angzarr-project/features/client/builder.feature",
        &mut failed,
    )
    .await;

    // Run CommandHandler tests
    println!("\n=== Running CommandHandler Tests ===\n");
    run::<CommandHandlerWorld>(
        "angzarr-project/features/client/command_handler.feature",
        &mut failed,
    )
    .await;

    // Run MultiHandler tests
    println!("\n=== Running MultiHandler Tests ===\n");
    run::<MultiHandlerWorld>(
        "angzarr-project/features/client/multi_handler.feature",
        &mut failed,
    )
    .await;

    // Run ProcessManager tests
    println!("\n=== Running ProcessManager Tests ===\n");
    run::<ProcessManagerWorld>(
        "angzarr-project/features/client/process_manager.feature",
        &mut failed,
    )
    .await;

    // Run Projector tests
    println!("\n=== Running Projector Tests ===\n");
    run::<ProjectorWorld>(
        "angzarr-project/features/client/projector.feature",
        &mut failed,
    )
    .await;

    // Run Rejection tests
    println!("\n=== Running Rejection Tests ===\n");
    run::<RejectionWorld>(
        "angzarr-project/features/client/rejection.feature",
        &mut failed,
    )
    .await;

    // Run Saga tests
    println!("\n=== Running Saga Tests ===\n");
    run::<SagaWorld>("angzarr-project/features/client/saga.feature", &mut failed).await;

    // Run RejectedCompensation tests
    println!("\n=== Running RejectedCompensation Tests ===\n");
    run::<RejectedCompensationWorld>(
        "angzarr-project/features/client/rejected_compensation.feature",
        &mut failed,
    )
    .await;

    // Run Validation tests
    println!("\n=== Running Validation Tests ===\n");
    run::<ValidationWorld>(
        "angzarr-project/features/client/validation.feature",
        &mut failed,
    )
    .await;

    // Run Upcaster tests
    println!("\n=== Running Upcaster Tests ===\n");
    run::<UpcasterWorld>(
        "angzarr-project/features/client/upcaster.feature",
        &mut failed,
    )
    .await;

    // Run Retry tests
    println!("\n=== Running Retry Tests ===\n");
    run::<RetryWorld>("angzarr-project/features/client/retry.feature", &mut failed).await;

    // Run Decorators tests (kind-declaration parity, C-0121..C-0126)
    println!("\n=== Running Decorators Tests ===\n");
    run::<DecoratorsWorldCucumber>(
        "angzarr-project/features/client/decorators.feature",
        &mut failed,
    )
    .await;

    // Run cross-language wire-format parity tests
    println!("\n=== Running WireParity Tests ===\n");
    run::<WireParityWorld>(
        "angzarr-project/features/client/wire_parity.feature",
        &mut failed,
    )
    .await;

    // ------------------------------------------------------------------
    // Coordinator-contract simulations.
    //
    // These three suites describe coordinator-side behavior that the
    // client cannot directly exercise (no public API matches the
    // documented surface). The features moved to
    // `features/coordinator-contract/` per audit findings #22, #26,
    // #28; the step files are explicit simulations of the
    // coordinator's contract — kept here so the cucumber prose
    // doesn't bit-rot. When a coordinator-tier suite exists in its
    // own repo, these become the canonical reference. See
    // PARITY_AUDIT.md for context.
    // ------------------------------------------------------------------

    println!("\n=== Running FactFlow (coordinator-contract sim) ===\n");
    run::<FactFlowWorld>(
        "angzarr-project/features/coordinator-contract/fact_flow.feature",
        &mut failed,
    )
    .await;

    println!("\n=== Running MergeStrategy (coordinator-contract sim) ===\n");
    run::<MergeStrategyWorld>(
        "angzarr-project/features/coordinator-contract/merge_strategy.feature",
        &mut failed,
    )
    .await;

    println!("\n=== Running StateBuilding (coordinator-contract sim) ===\n");
    run::<StateBuildingWorld>(
        "angzarr-project/features/coordinator-contract/state_building.feature",
        &mut failed,
    )
    .await;

    // Run Destinations query-surface tests (C-0132..C-0134)
    println!("\n=== Running Destinations Tests ===\n");
    run::<DestinationsWorld>(
        "angzarr-project/features/client/destinations.feature",
        &mut failed,
    )
    .await;

    if !failed.is_empty() {
        eprintln!("\n{} feature suites failed:", failed.len());
        for path in &failed {
            eprintln!("  {path}");
        }
        std::process::exit(1);
    }
}
