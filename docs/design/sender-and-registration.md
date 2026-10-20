# Sender delivery modes, Handling, and the component description

**Status:** signed off by the owner (with changes, recorded below) and
implemented. Tests: `tests/sender_and_registration.rs`.

**Spec:** angzarr-project `spec/20261019-draft`.
- `command_handler.proto`: the DECISION / ASYNC / SIMPLE coordinator RPCs.
- `types.proto`: `Handling`, `ComponentDescriptor`, `ComponentDescriptors`.
- `component.proto`: `ComponentService.Describe` (decision 14.10).
- `features/client/aggregate_client.feature`: C-0194..C-0198.
- `features/client/query_client.feature`: C-0241.
- `features/handler-contract/handling.feature`: C-0533, C-0535, C-0565.
- Backlog RV-2; decisions 14.1, 14.6, 14.10.

## Symbols

| Symbol | Purpose | Spec |
|---|---|---|
| `CommandHandlerClient::{handle_async, handle, handle_sync}`, the same on `GatewayClient` and `DomainClient` | one method per coordinator RPC; each returns the RPC's own answer: nothing, the decision (`CommandResponse`), the decision plus `outcome.projections` | command_handler.proto, C-0197, C-0198 |
| `CommandBuilder::{execute_async, execute, execute_sync}` | build, then call the matching RPC; `execute` is DECISION | same |
| `CommandBuilder::with_expected_sequence(u64)`, `with_conflict_resolution(ConflictResolution)` | the `Handling` fields of the same names. No default: unset sends nothing and the coordinator stamps the head; the resolution defaults to UNSPECIFIED | Handling |
| `ClientError::is_aborted()` | a stale sequence or conflict refusal (ABORTED) | C-0194, C-0195 |
| `QueryClient::get_correlated_events(Uuid, Option<String>)` | `GetCorrelatedEvents`, the cross-domain read | C-0241 |
| `angzarr_client::rules_version!()` | `&'static str` baked at build time from the binary's versionator `VERSION` file | RV-2 |
| `ComponentHost::new(rules_version)` | the binary's version, declared once and stamped onto every hosted component's tables; a host without one refuses to start (MISSING_RULES_VERSION) | RV-2, C-0533 |
| `ComponentService` served by every host | `Describe` answers the router's `component_descriptors()`; the coordinator pulls it, the client never pushes | component.proto, C-0535, C-0565 |
| kind macros declare the state type, snapshot image and loader when the state is a protobuf message | `StateDescriptor.type_url`; the post-command state on every response | C-0535 |
| aggregates are described by their domain | `ComponentDescriptor.name` | C-0565 |

The router's per-table `with_rules_version` is plumbing, not author API.
Snapshots: the client does nothing; the router returns the post-command
state on every response.

## Usage

```rust,ignore
// Sender
let resp = client.command("orders", id)
    .with_expected_sequence(4)
    .with_conflict_resolution(ConflictResolution::Strict)
    .with_command(url, &cmd)
    .execute().await;                       // DECISION
client.command("orders", id).with_command(url, &cmd).execute_async().await?;
let projected = client.command("orders", id).with_command(url, &cmd).execute_sync().await?;
if let Err(e) = resp { if e.is_aborted() { /* re-read, retry */ } }

// Business binary
ComponentHost::new(angzarr_client::rules_version!())
    .with_handler(|| Order)
    .serve().await?;                        // serves ComponentService too
```

## Rejected alternatives

- **`enqueue` / `execute_projected`** (the first proposal): the owner chose
  the spec's RPC names.
- **`execute_with(Delivery)` returning one type**: ASYNC answers `Empty`.
- **Default trait methods for the new modes**: a fake falling back to
  `Handle` passes C-0197 for the wrong reason.
- **`at_head()` and a default expected sequence of 0**: unset already means
  "stamp the head".
- **`Router::with_rules_version` as author API, or a version per
  component**: the version is the binary's (decision 14.1), declared once on
  the host.
- **A push/Register call**: the coordinator pulls (decision 14.10).
