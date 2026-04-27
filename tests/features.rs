//! Cucumber feature tests for the angzarr-client library.
//!
//! These tests verify client library behavior using Gherkin scenarios.
//! Run with:
//!
//! ```bash
//! cargo test --test features
//! ```

mod steps;

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
use steps::merge_strategy::MergeStrategyWorld;
use steps::state_building::StateBuildingWorld;
use steps::identity::IdentityWorld;
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
use steps::testing::TestingWorld;
use steps::upcaster::UpcasterWorld;
use steps::validation::ValidationWorld;
use steps::wire_parity::WireParityWorld;

#[tokio::main]
async fn main() {
    // Run Parity tests
    println!("\n=== Running Parity Tests ===\n");
    ParityWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/parity.feature")
        .await;

    // Run Identity tests
    println!("\n=== Running Identity Tests ===\n");
    IdentityWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/identity.feature")
        .await;

    // Run Testing tests
    println!("\n=== Running Testing Tests ===\n");
    TestingWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/testing.feature")
        .await;

    // Run Connection tests
    println!("\n=== Running Connection Tests ===\n");
    ConnectionWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/connection.feature")
        .await;

    // Run DomainClient tests
    println!("\n=== Running DomainClient Tests ===\n");
    DomainClientWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/domain-client.feature")
        .await;

    // Run AggregateClient tests
    println!("\n=== Running AggregateClient Tests ===\n");
    AggregateClientWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/aggregate_client.feature")
        .await;

    // Run QueryClient tests
    println!("\n=== Running QueryClient Tests ===\n");
    QueryClientWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/query_client.feature")
        .await;

    // Run SpeculativeClient tests
    println!("\n=== Running SpeculativeClient Tests ===\n");
    SpeculativeClientWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/speculative_client.feature")
        .await;

    // Run CommandBuilder tests
    println!("\n=== Running CommandBuilder Tests ===\n");
    CommandBuilderWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/command_builder.feature")
        .await;

    // Run QueryBuilder tests
    println!("\n=== Running QueryBuilder Tests ===\n");
    QueryBuilderWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/query_builder.feature")
        .await;

    // Run ErrorHandling tests
    println!("\n=== Running ErrorHandling Tests ===\n");
    ErrorHandlingWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/error_handling.feature")
        .await;

    // Legacy router.feature is retired in the Rust tier — routing behavior is
    // covered by the TIER5 suites below (builder / command_handler /
    // multi_handler / process_manager / projector / rejection / saga /
    // rejected_compensation / validation). The feature file remains in
    // angzarr-project for other languages.

    // Run EventDecoding tests
    println!("\n=== Running EventDecoding Tests ===\n");
    EventDecodingWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/event_decoding.feature")
        .await;

    // Run Compensation tests
    println!("\n=== Running Compensation Tests ===\n");
    CompensationWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/compensation.feature")
        .await;

    // ------------------------------------------------------------------
    // TIER5 Router feature suite.
    // ------------------------------------------------------------------

    // Run Builder tests
    println!("\n=== Running Builder Tests ===\n");
    BuilderWorld::cucumber()
        .run("angzarr-project/features/client/builder.feature")
        .await;

    // Run CommandHandler tests
    println!("\n=== Running CommandHandler Tests ===\n");
    CommandHandlerWorld::cucumber()
        .run("angzarr-project/features/client/command_handler.feature")
        .await;

    // Run MultiHandler tests
    println!("\n=== Running MultiHandler Tests ===\n");
    MultiHandlerWorld::cucumber()
        .run("angzarr-project/features/client/multi_handler.feature")
        .await;

    // Run ProcessManager tests
    println!("\n=== Running ProcessManager Tests ===\n");
    ProcessManagerWorld::cucumber()
        .run("angzarr-project/features/client/process_manager.feature")
        .await;

    // Run Projector tests
    println!("\n=== Running Projector Tests ===\n");
    ProjectorWorld::cucumber()
        .run("angzarr-project/features/client/projector.feature")
        .await;

    // Run Rejection tests
    println!("\n=== Running Rejection Tests ===\n");
    RejectionWorld::cucumber()
        .run("angzarr-project/features/client/rejection.feature")
        .await;

    // Run Saga tests
    println!("\n=== Running Saga Tests ===\n");
    SagaWorld::cucumber()
        .run("angzarr-project/features/client/saga.feature")
        .await;

    // Run RejectedCompensation tests
    println!("\n=== Running RejectedCompensation Tests ===\n");
    RejectedCompensationWorld::cucumber()
        .run("angzarr-project/features/client/rejected_compensation.feature")
        .await;

    // Run Validation tests
    println!("\n=== Running Validation Tests ===\n");
    ValidationWorld::cucumber()
        .run("angzarr-project/features/client/validation.feature")
        .await;

    // Run Upcaster tests
    println!("\n=== Running Upcaster Tests ===\n");
    UpcasterWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/upcaster.feature")
        .await;

    // Run Retry tests
    println!("\n=== Running Retry Tests ===\n");
    RetryWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/retry.feature")
        .await;

    // Run Decorators tests (kind-declaration parity, C-0121..C-0126)
    println!("\n=== Running Decorators Tests ===\n");
    DecoratorsWorldCucumber::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/decorators.feature")
        .await;

    // Run cross-language wire-format parity tests
    println!("\n=== Running WireParity Tests ===\n");
    WireParityWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/wire_parity.feature")
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
    FactFlowWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/coordinator-contract/fact_flow.feature")
        .await;

    println!("\n=== Running MergeStrategy (coordinator-contract sim) ===\n");
    MergeStrategyWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/coordinator-contract/merge_strategy.feature")
        .await;

    println!("\n=== Running StateBuilding (coordinator-contract sim) ===\n");
    StateBuildingWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/coordinator-contract/state_building.feature")
        .await;

    // Run Destinations query-surface tests (C-0132..C-0134)
    println!("\n=== Running Destinations Tests ===\n");
    DestinationsWorld::cucumber()
        .fail_on_skipped()
        .run("angzarr-project/features/client/destinations.feature")
        .await;
}
