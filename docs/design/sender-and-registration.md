# Proposal: sender delivery modes, Handling, and the registration hello

**Status:** proposed, awaiting owner sign-off. Nothing here is implemented.
The red tests are `tests/proposal.rs`, built only with
`--features proposal-tests` (they do not compile yet).

**Spec:** angzarr-project c0a00b9 (framework protos as at 619933c).
- `command_handler.proto`: the DECISION / ASYNC / SIMPLE coordinator RPCs.
- `types.proto`: `Handling`, `ComponentDescriptor`.
- `features/client/aggregate_client.feature`: C-0194..C-0198.
- `features/client/query_client.feature`: C-0241.
- `features/handler-contract/handling.feature`: C-0533..C-0535.
- Backlog RV-2; decisions 14.1 and 14.6.

The router (angzarr-router ce5853f) already builds the hello
(`component_descriptor()`, `Router::component_descriptors()`) and stamps a
declared rules version on every response (`with_rules_version`). What it does
not cover is the client's own surface: how a sender picks a mode, how a
business binary gets its version, and how the hello leaves the process.

## Symbols

| Symbol | Purpose | Spec |
|---|---|---|
| `CommandBuilder::execute()` (unchanged) | DECISION: `Handle`, the decision and its events | command_handler.proto |
| `CommandBuilder::enqueue() -> Result<()>` | ASYNC: `HandleAsync`, accepted for processing, no decision | C-0197 |
| `CommandBuilder::execute_projected() -> Result<CommandResponse>` | SIMPLE: `HandleSync`, the decision plus `outcome.projections` | C-0198 |
| `GatewayClient::{enqueue, execute_projected}` (required trait methods); `CommandHandlerClient::{handle_async, handle_sync}` | the same modes for fakes and transports | |
| `CommandBuilder::with_expected_sequence(u64)` (rename of the port's `with_sequence`) and `with_conflict_resolution(ConflictResolution)` (in the port) | `Handling.expected_sequence` / `Handling.conflict_resolution`, named after the proto fields as client-python names them | Handling |
| `CommandBuilder::at_head()` | send no `expected_sequence`, so the coordinator stamps the head | Handling |
| `ClientError::is_aborted()` | classify a stale-sequence or conflict refusal (ABORTED) | C-0194, C-0195 |
| `QueryClient::get_correlated_events(Uuid, Option<edition>) -> Vec<EventBook>` | `GetCorrelatedEvents`, the cross-domain read | C-0241 |
| `Router::with_rules_version(v)` | the version every table of the router reports and registers | C-0533, C-0535 |
| `angzarr_client::rules_version!()` | `&'static str` baked at build time from the binary's versionator version | RV-2 |
| `{CommandHandler,Saga,ProcessManager,Projector}Router::component_descriptors()`, `RunningHost::component_descriptors()` | the hello: the router's descriptors | C-0535 |
| kind macros declare the state type when the state is a `prost::Name` | `StateDescriptor.type_url` in the hello | C-0535 |

`ComponentHost::start` refuses to start (INVALID_ARGUMENT,
`MISSING_RULES_VERSION`) when any hosted component's hello fails.

## Usage

Sender:

```rust,ignore
let client = DomainClient::connect(endpoint).await?;
// DECISION (default): wait for the decision.
let resp = client.command_handler.command("orders", id)
    .with_expected_sequence(4)
    .with_conflict_resolution(ConflictResolution::Strict)
    .with_command(url, &cmd)
    .execute().await?;
// ASYNC: fire and forget.
client.command_handler.command("orders", id).at_head()
    .with_command(url, &cmd).enqueue().await?;
// SIMPLE: wait for the synchronous projectors.
let resp = client.command_handler.command("orders", id)
    .with_command(url, &cmd).execute_projected().await?;
match resp { Err(e) if e.is_aborted() => { /* re-read and retry */ } _ => {} }
```

Business binary:

```rust,ignore
// build.rs: angzarr_client::build::export_rules_version()?;  (reads versionator's VERSION)
let built = Router::new("order")
    .with_rules_version(angzarr_client::rules_version!())
    .with_handler(|| Order)
    .build()?;
ComponentHost::new().with_router(built).serve().await?;   // sends the hello
```

## Open questions for the owner

1. **Hello transport.** The spec defines `ComponentDescriptor` but no RPC
   carries it ("the registration hello a component sends its coordinator on
   startup", decision 14.1). The tests only pin what the host announces
   (`RunningHost::component_descriptors()`). Which RPC sends it, and when
   does the host send it relative to health `SERVING`?
2. **Version source.** `rules_version!()` reading an `ANGZARR_RULES_VERSION`
   env var set by a build helper from versionator's `VERSION` file; or the
   binary's `CARGO_PKG_VERSION`; or a value the generated code (angzarr-cli)
   bakes. The design-handling doc in angzarr-router lists this as open.
3. **Default expected sequence.** The builder keeps 0 (parity C-0308);
   `at_head()` is the opt-out. Alternatively unset could mean "stamp the
   head", which breaks C-0308 and the Python parity.
4. **Naming.** The spec names only the proto fields and RPCs, no client
   methods. `with_expected_sequence` / `with_conflict_resolution` follow the
   `Handling` fields (as client-python does), but parity C-0311's docstring
   still says `.with_sequence(3)`. Nothing in the spec names the delivery
   methods (`enqueue` / `execute_projected` here, versus `execute_async` /
   `execute_sync` after the RPCs, where `async` reads as Rust async),
   `at_head`, `is_aborted`, `get_correlated_events`, `with_rules_version`,
   `rules_version!` or `component_descriptors`; the last two match the
   router's names.

## Rejected alternatives

- **`execute_with(Delivery)` returning one type.** ASYNC answers `Empty`, not
  a `CommandResponse`; one method would return a fake response for it.
- **Default trait methods for the new modes.** A fake that silently falls
  back to `Handle` would pass C-0197 for the wrong reason (the old
  `execute_with_sync_mode` default did exactly that).
- **A per-component version.** The version is the binary's (decision 14.1);
  co-hosted components share it, so it is set once per router.
- **A predicate per conflict code.** `is_aborted` matches the existing
  status-class predicates; the code (`CONFLICT_OVERLAP`, ...) is in the
  ErrorInfo for those who need it.
