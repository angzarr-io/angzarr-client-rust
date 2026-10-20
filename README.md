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
| Saga | `#[saga(name, input_domain, output_domain)]` | Translate one source domain's events into commands for its one destination domain (several destinations make a process manager) |
| Process manager | `#[process_manager(name, domain, state, output_domain, output_domains)]`, each `#[handles(Event, domain = "...")]` naming its trigger's domain | Stateful multi-domain orchestrator |
| Projector | `#[projector(name, domains)]` | Side-effect fan-out over event books |
| Upcaster chain | `#[upcaster(name, domain)]` | Transform legacy event versions in place; part of its component (`Router::with_upcaster`), whose host serves `UpcasterService` |

Method markers inside the impl block:

| Marker | Applies to | Role |
|--------|-----------|------|
| `#[handles(MessageType)]` | Any kind | Register a handler for an incoming message; a process manager's also names its trigger's domain: `#[handles(Event, domain = "...")]` |
| `#[handles_fact(EventType)]` | command_handler | Record a fact (optionally with flag events) |
| `#[applies(EventType)]` | command_handler, process_manager | Mutate state during rebuild |
| `#[state_factory]` | command_handler, process_manager | Override `Default::default()` for initial state |
| `#[upcasts(from, to)]` | upcaster | Transform one event type into another |

The attributes and markers live in `angzarr_client::router`, next to
`angzarr_client::router::binding` — the angzarr-router engine they register
handlers with. Dispatch behaviour is the router's, specified by
angzarr-router's conformance suite.

### Command-handler example

```rust,ignore
use angzarr_client::proto::EventBook;
use angzarr_client::router::{applies, command_handler, handles};
use angzarr_client::{CommandRejectedError, CommandResult};

#[derive(Default, Clone)]
struct CounterState {
    value: u64,
}

struct Counter;

#[command_handler(domain = "counter", state = CounterState)]
impl Counter {
    #[applies(Increased)]
    fn apply_increased(state: &mut CounterState, evt: Increased) {
        state.value += evt.by;
    }

    #[handles(Increase)]
    fn increase(&self, cmd: Increase, state: &CounterState, seq: u64) -> CommandResult<EventBook> {
        if cmd.by == 0 {
            return Err(CommandRejectedError::invalid_argument(
                "ZERO_INCREASE",
                "increase must be positive",
                [("by", "0")],
            ));
        }
        // build and return the event book
        // ...
    }
}
```

## Component host

`ComponentHost` serves components next to the gRPC health service and the
component service (`ComponentService.Describe`, which the coordinator calls
to learn the hosted components), plus any gRPC services the application
registers. It takes the binary's build version once, baked in from the
crate's versionator `VERSION` file by `rules_version!()`; every component
reports it on every answer and in its description:

```rust,ignore
use angzarr_client::{configure_logging, ComponentHost};

#[tokio::main]
async fn main() -> angzarr_client::Result<()> {
    configure_logging();
    ComponentHost::new(angzarr_client::rules_version!())
        .with_handler(|| Counter)
        .with_service(MyReportServiceServer::new(MyReports::default()))
        .serve()
        .await
}
```

- Transport comes from the environment (`get_transport_config`): TCP, or a
  Unix socket the host removes on shutdown.
- A host serves exactly one component (one coordinator = one component =
  one business process); registering a second, or starting with none, does
  not compile. The factory closure runs once per dispatch, so close over
  shared dependencies (`move || Ledger::new(pool.clone())`).
- Health reports `NOT_SERVING` until the listener is bound and every sync
  output domain is reachable, then `SERVING`; on SIGINT / SIGTERM it reports
  `NOT_SERVING`, waits the drain period (`with_drain_period`), then stops
  accepting and lets in-flight calls finish.
- A host with an empty rules version refuses to start
  (`MISSING_RULES_VERSION`).

## Clients

| Client | Purpose |
|--------|---------|
| `CommandHandlerClient` | Send commands to a coordinator |
| `QueryClient` | Fetch event books |
| `DomainClient` | Bundle of both, scoped to a domain |

All three carry a `connect(endpoint, retry)` constructor, a `from_channel` for caller-managed channels, and a `from_env(env_var, default)` helper.

A command goes out by one of the coordinator's three RPCs, named on the
client (`handle_async`, `handle`, `handle_sync`) and on the builder
(`execute_async`, `execute`, `execute_sync`): send and return at once, wait
for the decision, or also wait for the synchronous projectors. The builder
sends only what the caller sets: `with_expected_sequence`,
`with_conflict_resolution` and `with_correlation_id` have no defaults, and
the coordinator stamps or assigns what is missing. A stale sequence or a
conflict comes back ABORTED (`ClientError::is_aborted`).
`QueryClient::get_correlated_events` reads one correlation across domains.

## Error handling

```rust,ignore
use angzarr_client::ClientError;

match client.command_handler.handle(cmd).await {
    Ok(resp) => { /* ... */ }
    Err(e) if e.is_aborted() => {
        // Stale expected sequence (optimistic locking) or a conflict
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
| Cover accessors | Free functions: `domain(cover)`, `edition(cover)`, … | Extension trait methods: `cover.domain()`, `cover.edition()`, … (via `CoverExt`) |
| Event book helpers | Free functions: `next_sequence(book)` | Extension trait methods: `book.next_sequence()` (via `EventBookExt`) |
| Wrapper objects | `CoverW(cover).domain()` | Call extension trait method directly: `cover.domain()` |
| Error surface | Exception hierarchy: `ClientError → GRPCError / ConnectionError / …` | `thiserror` enum `ClientError { Connection, Transport, Grpc, InvalidArgument, InvalidTimestamp }` with `is_*` predicate methods |
| Rejection factories | `CommandRejectedError.precondition_failed(msg)` | `CommandRejectedError::precondition_failed(msg)` |
| Retry | `ExponentialBackoffRetry(max_attempts=5)` | `ExponentialBackoffRetry::default().with_max_attempts(5)` |
| Retry callback | `on_retry=lambda i, e: ...` | `.with_on_retry(\|i, e\| ...)` |
| Type-URL match | `type_url_matches(url, name)` (primary) | `type_url_matches(url, name)` (primary) — `type_url_matches_exact` is a Python-compat alias |

## Saga / PM design philosophy

**Sagas and PMs are coordinators, not decision makers.**

| Output | When to use |
|--------|-------------|
| Commands (preferred) | Normal flow — the target aggregate validates and decides |
| Facts | Inject external data the target aggregate can't derive |

Key principles:

1. **Don't rebuild destination state** — a saga gets only the triggering EventBook; the coordinator stamps sequences.
2. **Let aggregates decide** — business logic in aggregates, not coordinators.
3. **Use facts sparingly** — only for external data injection.

A what-if run is an ordinary command sent to a named edition
(`Cover.edition`); there is no separate speculative client.

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
just mutation-test   # cargo-mutants (70% kill-rate threshold; ephemeral container)
```

> **Mutation testing is ephemeral.** `just mutation-test` runs inside a
> container with the workspace mounted **read-only**; mutated source lives
> in the container's writable overlay and is destroyed by `--rm` on exit.
> Running `cargo mutants` directly on the host is **forbidden** — a crashed
> run would leak mutated files into your working tree. Only
> `mutants.out/outcomes.json` is copied back to the host. The
> `.mutants-cache/` directory holds compiled artifacts (never mutated
> source) and is gitignored; purge with `just mutants-purge-cache`.
