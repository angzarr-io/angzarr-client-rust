> **⚠️ Notice:** This repository was recently extracted from the [angzarr monorepo](https://github.com/angzarr-io/angzarr) and has not yet been validated as a standalone project. Expect rough edges. See the [Angzarr documentation](https://angzarr.io/) for more information.

# angzarr-client-rust

Rust client library for Angzarr event sourcing framework.

## Installation

```toml
[dependencies]
angzarr-client = "0.2"
```

## Component Types

### Command Handlers (Aggregates)

```rust
use angzarr_client::{CommandHandlerRouter, CommandHandlerDomainHandler};

struct PlayerHandler { /* ... */ }

impl CommandHandlerDomainHandler for PlayerHandler {
    type State = PlayerState;

    fn command_types(&self) -> Vec<String> {
        vec!["RegisterPlayer".into(), "DepositFunds".into()]
    }

    fn handle(&self, cmd: &CommandBook, payload: &Any, state: &Self::State, seq: u32)
        -> CommandResult<EventBook> {
        // Validate and emit events
    }
}

let router = CommandHandlerRouter::new("agg-player", "player", handler);
```

### Sagas

Stateless event translators (events → commands).

```rust
use angzarr_client::{SagaRouter, SagaDomainHandler, Destinations};

struct OrderSagaHandler;

impl SagaDomainHandler for OrderSagaHandler {
    fn event_types(&self) -> Vec<String> {
        vec!["OrderCreated".into()]
    }

    fn handle(&self, source: &EventBook, event: &Any, destinations: &Destinations)
        -> CommandResult<SagaHandlerResponse> {
        // Translate event to command, stamp with destination sequence
        let seq = destinations.sequence_for("inventory")?;
        Ok(SagaHandlerResponse { commands: vec![cmd], events: vec![] })
    }
}

let router = SagaRouter::new("saga-order-inventory", "order", handler);
```

### Process Managers

Stateful multi-domain orchestrators.

```rust
use angzarr_client::{ProcessManagerRouter, ProcessManagerDomainHandler, Destinations};

struct BuyInPmHandler;

impl ProcessManagerDomainHandler<BuyInState> for BuyInPmHandler {
    fn event_types(&self) -> Vec<String> {
        vec!["BuyInRequested".into(), "PlayerSeated".into()]
    }

    fn prepare(&self, trigger: &EventBook, state: &BuyInState, event: &Any) -> Vec<Cover> {
        // Declare destination domains needed
        vec![Cover { domain: "table".into(), .. }]
    }

    fn handle(&self, trigger: &EventBook, state: &BuyInState, event: &Any, destinations: &Destinations)
        -> CommandResult<ProcessManagerResponse> {
        // Emit commands or facts
        let seq = destinations.sequence_for("table")?;
        Ok(ProcessManagerResponse { commands: vec![cmd], process_events: None, facts: vec![] })
    }
}

let router = ProcessManagerRouter::new("pmg-buy-in", "pmg-buy-in", |_| BuyInState::default())
    .domain("player", BuyInPmHandler)
    .domain("table", BuyInPmHandler);
```

## Saga/PM Design Philosophy

**Sagas and PMs are coordinators, NOT decision makers.**

### Output Options

| Output | When to Use |
|--------|-------------|
| **Commands** (preferred) | Normal flow - aggregate validates and decides |
| **Facts** | Inject external data aggregate can't derive |

### Key Principles

1. **Don't rebuild destination state** - Use `Destinations` for sequences only
2. **Let aggregates decide** - Business logic in aggregates, not coordinators
3. **Prefer commands with sync mode** - Use `SyncMode::Simple` for immediate feedback
4. **Use facts sparingly** - Only for external data injection

## License

AGPL-3.0-only
