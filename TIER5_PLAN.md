# Tier 5 Phase 2 — Rust unified Router

Adaptation of the cross-language Tier 5 design for the Rust client. Reference: `client-python/main/TIER5_PLAN.md`.

## Design (agreed, Rust-adapted)

- One builder: `Router::new(name)` with `.with_handler::<H>(factory)` as the sole registration method. `factory: impl Fn() -> H + Send + Sync + 'static` is invoked per dispatch call to produce a fresh (or pooled) handler instance, keeping handler state isolated per request and making the built router safe to share across threads. Users can close over shared deps (`move || Player::new(db_pool.clone())`) or hand in a pool-checkout closure.
- Class-level proc macros declare the kind: `#[aggregate]`, `#[saga]`, `#[process_manager]`, `#[projector]`. No trait for the user to implement by hand.
- Method-level proc macros declare behavior: `#[handles]`, `#[applies]`, `#[rejected]`, `#[state_factory]`.
- Typed runtime routers returned from `Router::build()`: `CommandHandlerRouter<S>`, `SagaRouter`, `ProcessManagerRouter<S>`, `ProjectorRouter`. No public constructors.
- Multi-handler dispatch with output merging; `seq` increments across merged stream (A gets 5, emits 2, B gets 7).
- Mixed kinds at build → `Err(BuildError)`.
- **CloudEvents is out of scope.** Extracted to the separate `cloudevents` repo in a prior PR; `client-rust/main/src/router/cloudevents.rs` is already deleted.
- Scope: `client-rust/main/src/` + `client-rust/main/angzarr-macros/` only. Tests in `tests/` and `src/**/*.test.rs`.

## Target surface

```rust
// Builder
pub struct Router { /* private */ }
impl Router {
    pub fn new(name: impl Into<String>) -> Self;
    pub fn with_handler<H, F>(self, factory: F) -> Self
    where
        H: Handler + 'static,
        F: Fn() -> H + Send + Sync + 'static;
    pub fn build(self) -> Result<Built, BuildError>;
}

// Typed output of build() — one variant per kind
pub enum Built {
    CommandHandler(CommandHandlerRouter),
    Saga(SagaRouter),
    ProcessManager(ProcessManagerRouter),
    Projector(ProjectorRouter),
}
// Plus typed accessors: .into_command_handler() -> Result<CommandHandlerRouter, BuildError>, etc.

// Minimal trait produced by proc macros — users never implement it by hand
pub trait Handler: Send + Sync {
    fn config(&self) -> HandlerConfig;
    fn dispatch(&self, request: HandlerRequest) -> Result<HandlerResponse, ClientError>;
}

// Proc macros (angzarr-macros crate)
#[aggregate(domain = "player", state = PlayerState)]
#[saga(name = "saga-order-fulfillment", source = "order", target = "inventory")]
#[process_manager(name = "...", pm_domain = "...", sources = [...], targets = [...], state = S)]
#[projector(name = "...", domains = [...])]

#[handles(Cmd)]
#[applies(Evt)]
#[rejected(domain = "...", command = "...")]
#[state_factory]
```

User-facing example:
```rust
#[aggregate(domain = "player", state = PlayerState)]
impl Player {
    pub fn new(db_pool: DbPool) -> Self { Self { db_pool } }

    #[state_factory]
    fn empty() -> PlayerState { PlayerState::default() }

    #[applies(PlayerRegistered)]
    fn apply_registered(state: &mut PlayerState, evt: PlayerRegistered) { ... }

    #[handles(RegisterPlayer)]
    fn register(&self, cmd: RegisterPlayer, state: &PlayerState, seq: u32)
        -> CommandResult<EventBook> { ... }

    #[rejected(domain = "payment", command = "ProcessPayment")]
    fn on_payment_rejected(&self, notif: &Notification, state: &PlayerState)
        -> CommandResult<BusinessResponse> { ... }
}

let router: CommandHandlerRouter = Router::new("agg-service")
    .with_handler({
        let pool = db_pool.clone();
        move || Player::new(pool.clone())
    })
    .with_handler({
        let rng = rng.clone();
        move || Hand::new(rng.clone())
    })
    .build()?
    .into_command_handler()?;
```

## File layout (end state)

```
client-rust/main/src/
  router/
    mod.rs                     Router + Built + mode markers deleted; becomes slim entry
    builder.rs                 NEW — Router builder impl
    runtime.rs                 NEW — CommandHandlerRouter, SagaRouter, ProcessManagerRouter, ProjectorRouter
    dispatch.rs                REWRITE — multi-handler dispatch, merge logic
    validation.rs              NEW — build-time invariants
    state.rs                   KEEP — StateRouter/Destinations
    saga_context.rs            KEEP
    helpers.rs                 KEEP
    upcaster.rs                KEEP (separate system)
    traits.rs                  DELETE — *DomainHandler traits obsolete
  handler.rs                   REWRITE — gRPC server wrappers accept new runtime types
  lib.rs                       UPDATE exports, delete stale mode-markers + SingleDomainRouter doc
  retry.rs                     NO CHANGE (already done in earlier tier)
  transport.rs                 NO CHANGE

client-rust/main/angzarr-macros/src/
  lib.rs                       REWRITE aggregate!/saga!/process_manager!/projector! macros
                               to generate `impl Handler for T { ... }` rather than per-component traits.
                               Add #[state_factory] support.

client-rust/main/tests/
  router/                      NEW subdirectory
    builder.rs
    decorators.rs              Rust tests for proc-macro expansion
    dispatch_command_handler.rs
    multi_handler.rs
    rejection.rs
    dispatch_saga.rs
    dispatch_pm.rs
    dispatch_projector.rs
```

## TDD rounds (Rust adaptation)

### R1 — supporting types + proc macros stash metadata
Tests: `#[aggregate(domain = "player", state = PlayerState)]` on a struct-impl produces an `impl Handler` with `config()` returning `HandlerConfig::CommandHandler { domain: "player", ... }`. Stacking two kind-macros → `compile_error!`.
Implement:
- Introduce the supporting types referenced by the `Handler` trait (`HandlerConfig`, `HandlerRequest`, `HandlerResponse`, `BuildError`) plus the `Handler` trait itself in a new `router/handler.rs` (or equivalent). Types remain empty-shell enough that later rounds can extend variants without breaking R1.
- Minimal macro rewrite in `angzarr-macros` that emits the `Handler` trait impl for the struct the `#[aggregate]` attribute is applied to.

### R2 — method macros stash metadata
Tests: `#[handles(Cmd)]` on a method produces registration metadata recoverable from `Handler::config()`. Same for `#[applies]`, `#[rejected]`, `#[state_factory]`.
Implement: method-level macro expansion → per-method registration entries in the generated `Handler` impl.

### R3 — builder collects factories
Tests: `Router::new("x").with_handler(|| P::new()).with_handler(|| H::new()).build()` stores both factories. Empty build → `BuildError`. Factory whose return type isn't `Handler` → compile error (trait bound). Factories are not invoked at registration or build time — only during dispatch.
Implement: `router/builder.rs` — `Router`, `Vec<Box<dyn Fn() -> Box<dyn Handler> + Send + Sync>>` (or an equivalent type-erased factory shape), build dispatch.

### R4 — mode inference
Tests: homogeneous → correct `Built::*` variant. Mixed → `BuildError("cannot mix ...")`.
Implement: mode inference + stub runtime router types.

### R5 — build-time validation per kind
Tests (table-driven):
- `#[aggregate]` without `state` → compile error (macro side)
- `#[saga]` without `target` → compile error
- `#[process_manager]` missing any of `pm_domain`/`sources`/`targets`/`state` → compile error
- `#[projector]` without `domains` → compile error
- Duplicate `(domain, type_url)` across handlers → allowed (call-both)
Implement: compile-time attribute parsing in macros; runtime invariants in `validation.rs`.

### R6 — single-handler command dispatch
Tests: one aggregate with one `#[handles(Cmd)]` → `CommandHandlerRouter::dispatch` routes and returns `BusinessResponse`.
Implement: `CommandHandlerRouter::dispatch` in `dispatch.rs`.

### R7 — state rebuild via #[applies]
Tests: prior EventBook's events replayed through the instance's `#[applies]` methods before `#[handles]` invocation. `#[state_factory]` overrides `Default::default()`.
Implement: state rebuild loop per instance.

### R8 — multi-handler merge
Tests: two aggregate factories in the same domain each produce a type carrying `#[handles(Cmd)]`; both invoked in registration order (one factory call per matched handler per dispatch); events concatenated. Each handler rebuilds its own state.
Implement: multi-handler fan-out in dispatch.

### R9 — sequence increments
Tests: A at seq=5 emits 2 → B at seq=7. Merged stream has monotonic seqs.
Implement: framework-driven `seq` tracking.

### R10 — rejection handler routing
Tests: `#[rejected(domain = "payment", command = "ProcessPayment")]` receives Notification. Multiple handlers for same key → all invoked, compensation merged.
Implement: notification branch across all runtime routers.

### R11 — saga dispatch
Tests: `#[saga]` with `#[handles]` emits commands/facts; `SagaRouter::dispatch` wraps into `SagaResponse`. Multi-handler merge.
Implement: `SagaRouter::dispatch`.

### R12 — process-manager dispatch
Tests: multi-source `#[process_manager]` with state. Destination-sequence stamping via `Destinations`. Rejection path.
Implement: `ProcessManagerRouter::dispatch`.

### R13 — projector dispatch
Tests: `#[projector]` side-effects via `&self`; multi-handler fan-out; no merge (side-effect semantics). One projector instance per matching factory per dispatch, reused across every event in the book so projectors can batch writes within a single projection run.
Implement: `ProjectorRouter::dispatch`.

### R14 — gRPC server wrappers + Gherkin features stay green
Tests: `angzarr-project/features/client/*.feature` (run via `tests/features.rs`) and `tests/steps/*.rs` pass unchanged after `handler.rs` wrappers are adapted.
Implement: `CommandHandlerGrpc`, `SagaHandler`, `ProcessManagerGrpcHandler`, `ProjectorHandler` in `src/handler.rs` take the new typed runtime routers.

### R15 — cleanup
- Delete `src/router/traits.rs` (`*DomainHandler` traits).
- Delete mode markers (`CommandHandlerMode`, `SagaMode`, `ProcessManagerMode`, `ProjectorMode`) and the stale `SingleDomainRouter<S, Mode>` doc in `router/mod.rs`.
- Update `lib.rs` exports.
- Run `cargo test --lib && cargo test --test '*'` — all green.

## Doneness gate per round

1. New round's tests green (`cargo test -p angzarr-client <filter>`).
2. All prior rounds still green (`cargo test --lib --tests`).
3. `cargo clippy --all-targets -- -D warnings` clean.
4. No new `unsafe`, no new panics in library code paths.

## Risks + mitigations

- **Proc-macro bugs are hard to debug.** Use `cargo expand` per round to verify expansion. Keep macros thin — generate trait impls that call hand-written helpers in the client crate.
- **Trait-object dispatch (`Box<dyn Handler>`) has overhead.** Acceptable for the scale of command routing; don't optimize until measured.
- **Factory latency on the request path.** Factories run inside `dispatch()`; document that handlers with expensive construction (I/O, connection setup, config reads) should pool or close over a pre-built instance (`move || shared.clone()`) rather than constructing fresh per call.
- **Mode markers were re-exported from `lib.rs`.** Deleting them is a public-API break. Will show in semver; flag in CHANGELOG.

## Out of scope for Phase 2

- `examples-rust` migration (follow-up).
- Cross-language behavior parity (handled per-language).
- Performance benchmarks.
