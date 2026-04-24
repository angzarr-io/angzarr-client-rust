> **⚠️ Notice:** This repository was recently extracted from the [angzarr monorepo](https://github.com/angzarr-io/angzarr) and has not yet been validated as a standalone project. Expect rough edges. See the [Angzarr documentation](https://angzarr.io/) for more information.

---
title: Rust SDK
sidebar_label: Rust
---

# angzarr-client

Rust client library for the Angzarr CQRS/ES framework.

:::tip Unified Documentation
For cross-language API reference with side-by-side comparisons, see the [SDK Documentation](/sdks).
:::

## Installation

```toml
[dependencies]
angzarr-client = "0.5"
```

## Quick Start

```rust,no_run
use angzarr_client::{DomainClient, CommandBuilderExt, QueryBuilderExt};
use uuid::Uuid;

#[tokio::main]
async fn main() -> angzarr_client::Result<()> {
    let client = DomainClient::connect("http://localhost:1310").await?;

    // Build and execute a command
    let order_id = Uuid::new_v4();
    let response = client
        .command_handler
        .command("order", order_id)
        .with_command("type.googleapis.com/examples.CreateOrder", &create_order_msg)
        .execute()
        .await?;

    // Query events
    let events = client
        .query
        .query("order", order_id)
        .get_event_book()
        .await?;

    Ok(())
}
```

## Handler Kinds

Handler classes are declared with attribute macros. No trait is implemented by hand — the macro emits `impl Handler`/`impl HandlerKind` from the attribute metadata and the method markers in the impl block.

| Kind | Macro | Purpose |
|------|-------|---------|
| Command handler | `#[command_handler(domain, state)]` | Validate commands, emit events |
| Saga | `#[saga(name, source, target)]` | Translate source-domain events into target-domain commands |
| Process manager | `#[process_manager(name, pm_domain, sources, targets, state)]` | Stateful multi-domain orchestrator |
| Projector | `#[projector(name, domains)]` | Side-effect fan-out over event books |
| Upcaster | `#[upcaster(name, domain)]` | Transform legacy event versions in place |

Method markers inside the impl block:

| Marker | Applies to | Role |
|--------|-----------|------|
| `#[handles(MessageType)]` | Any kind | Register a handler for an incoming message |
| `#[applies(EventType)]` | command_handler, process_manager | Mutate state during replay |
| `#[rejected(domain, command)]` | command_handler, saga, process_manager | Receive a rejection notification and emit compensation |
| `#[state_factory]` | command_handler, process_manager | Override `Default::default()` for initial state |
| `#[upcasts(from, to)]` | upcaster | Transform one event type into another |

### Command-handler example

```rust,ignore
use angzarr_client::{command_handler, handles, applies, CommandResult};
use angzarr_client::proto::EventBook;

#[derive(Default, Clone)]
struct PlayerState {
    player_id: String,
    bankroll: u64,
}

struct Player {
    db_pool: DbPool,
}

impl Player {
    pub fn new(db_pool: DbPool) -> Self { Self { db_pool } }
}

#[command_handler(domain = "player", state = PlayerState)]
impl Player {
    #[applies(PlayerRegistered)]
    fn apply_registered(state: &mut PlayerState, evt: PlayerRegistered) {
        state.player_id = evt.player_id;
    }

    #[handles(RegisterPlayer)]
    fn register(
        &self,
        cmd: RegisterPlayer,
        state: &PlayerState,
        seq: u32,
    ) -> CommandResult<EventBook> {
        if !state.player_id.is_empty() {
            return Err(CommandRejectedError::precondition_failed("player already exists").into());
        }
        // build and return the event book
        // ...
    }
}
```

## Router

One builder, one entry point:

```rust,ignore
use angzarr_client::{run_server, Router};

#[tokio::main]
async fn main() -> angzarr_client::Result<()> {
    let built = Router::new("agg-player")
        .with_handler({
            let pool = db_pool.clone();
            move || Player::new(pool.clone())
        })
        .with_handler({
            let rng = rng.clone();
            move || Hand::new(rng.clone())
        })
        .build()?;

    run_server("agg-player", 50001, built).await?;
    Ok(())
}
```

The factory closure runs once per dispatch, so each request gets a fresh handler instance. Close over shared deps (`move || Player::new(pool.clone())`) or hand in a pool-checkout closure.

`Router::build()` returns `Built::CommandHandler / Saga / ProcessManager / Projector / Upcaster` based on the kinds present. Mixing kinds in one router is a `BuildError::MixedKinds`.

## Clients

| Client | Purpose |
|--------|---------|
| `CommandHandlerClient` | Send commands to a coordinator |
| `QueryClient` | Fetch event books |
| `SpeculativeClient` | Dry-run commands without persisting |
| `DomainClient` | Bundle of all three, scoped to a domain |

All four carry a `connect(endpoint, retry)` constructor, a `from_channel` for caller-managed channels, and a `from_env(env_var, default)` helper.

## Error handling

```rust,ignore
use angzarr_client::ClientError;

match client.command_handler.handle(cmd).await {
    Ok(resp) => { /* ... */ }
    Err(ClientError::Grpc(status)) if status.code() == tonic::Code::FailedPrecondition => {
        // Sequence mismatch (optimistic locking)
    }
    Err(e) if e.is_not_found() => { /* aggregate missing */ }
    Err(e) if e.is_invalid_argument() => { /* bad input */ }
    Err(e) if e.is_connection_error() => { /* transport failure */ }
    Err(e) => return Err(e),
}
```

`ClientError` is a `thiserror` enum (`Connection / Transport / Grpc / InvalidArgument / InvalidTimestamp`). The `is_*` predicates cover the cases you care about without matching on variants. `CommandRejectedError` carries a `reason: String` and a `status_code` (`FAILED_PRECONDITION` / `INVALID_ARGUMENT` / `NOT_FOUND`) with named factory methods (`precondition_failed`, `invalid_argument`, `not_found`).

## Retry

```rust,ignore
use angzarr_client::ExponentialBackoffRetry;
use std::time::Duration;

let policy = ExponentialBackoffRetry::default()
    .with_max_attempts(5)
    .with_max_delay(Duration::from_secs(2))
    .with_on_retry(|attempt, err| eprintln!("retry {attempt}: {err}"));

let result = policy.execute(|| try_something());
```

Defaults match the cross-language spec: 10 attempts, 100 ms → 5 s with jitter. `RetryPolicy` is an alias for `ExponentialBackoffRetry`.

## Coming from Python?

Everything maps. The shape differs; the names and semantics don't.

| Concept | Python | Rust |
|---------|--------|------|
| Kind declaration | `@command_handler(domain="p", state=PlayerState)` class decorator | `#[command_handler(domain = "p", state = PlayerState)]` attribute macro on `impl` |
| Method marker | `@handles(Cmd)` | `#[handles(Cmd)]` |
| Router | `Router("x").with_handler(cls, factory)` | `Router::new("x").with_handler::<H, _>(factory)` (type inferred from the closure's return) |
| Factory | `lambda: Player(db)` | `\|\| Player::new(db.clone())` |
| Handler state | Class instance | Struct instance from factory closure |
| Cover accessors | Free functions: `domain(cover)`, `correlation_id(cover)`, … | Extension trait methods: `cover.domain()`, `cover.correlation_id()`, … (via `CoverExt`) |
| Event book helpers | Free functions: `next_sequence(book)` | Extension trait methods: `book.next_sequence()` (via `EventBookExt`) |
| Wrapper objects | `CoverW(cover).domain()` | Call extension trait method directly: `cover.domain()` |
| Error surface | Exception hierarchy: `ClientError → GRPCError / ConnectionError / …` | `thiserror` enum `ClientError { Connection, Transport, Grpc, InvalidArgument, InvalidTimestamp }` with `is_*` predicate methods |
| Rejection factories | `CommandRejectedError.precondition_failed(msg)` | `CommandRejectedError::precondition_failed(msg)` |
| Retry | `ExponentialBackoffRetry(max_attempts=5)` | `ExponentialBackoffRetry::default().with_max_attempts(5)` |
| Retry callback | `on_retry=lambda i, e: ...` | `.with_on_retry(\|i, e\| ...)` |
| Compensation options | `delegate_to_framework(reason, send_to_dead_letter=True)` (kwargs) | `delegate_to_framework(reason)` **or** `delegate_to_framework_with_options(reason, emit, send_to_dead_letter, escalate, abort)` (two-function idiom) |
| Type-URL match | `type_url_matches(url, name)` (primary) | `type_url_matches(url, name)` (primary) — `type_url_matches_exact` is a Python-compat alias |

## Saga / PM design philosophy

**Sagas and PMs are coordinators, not decision makers.**

| Output | When to use |
|--------|-------------|
| Commands (preferred) | Normal flow — the target aggregate validates and decides |
| Facts | Inject external data the target aggregate can't derive |

Key principles:

1. **Don't rebuild destination state** — use `Destinations` for sequences only.
2. **Let aggregates decide** — business logic in aggregates, not coordinators.
3. **Prefer commands with sync mode** — use `SyncMode::Simple` for immediate feedback.
4. **Use facts sparingly** — only for external data injection.

## Speculative execution

Test commands without persisting to the event store.

```rust,ignore
use angzarr_client::SpeculativeClient;
use angzarr_client::proto::SpeculateCommandHandlerRequest;

let client = SpeculativeClient::connect("http://localhost:1310").await?;
let request = SpeculateCommandHandlerRequest {
    command: Some(command_book),
    events: prior_events,
    ..Default::default()
};
let response = client.command_handler(request, None).await?;
```

## License

AGPL-3.0-only

## Development

Install git hooks (requires [lefthook](https://github.com/evilmartians/lefthook)):

```bash
lefthook install
```

This configures a pre-commit hook that auto-formats code before each commit.

### Recipes

```bash
just -l              # list recipes
just build           # build the library
just test            # run lib + cucumber tests
just lint            # cargo clippy -D warnings
just fmt             # cargo fmt --check
just fmt-fix         # cargo fmt
just mutation-test   # cargo-mutants (70% kill-rate threshold)
```
