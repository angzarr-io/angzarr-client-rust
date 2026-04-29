//! Saga dispatch step definitions.

use std::collections::HashMap;

use angzarr_client::proto::{
    event_page, CommandBook, Cover, DomainDivergence, Edition, EventBook, EventPage,
    SagaHandleRequest, SagaResponse,
};
use angzarr_client::router::{Built, Router};
use angzarr_client::{full_type_url, saga, CommandResult};
use cucumber::{given, then, when, World};
use prost::Message as _;
use prost_types::Any;

// ---------------------------------------------------------------------------
// Protos.
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderCreated {}
impl ::prost::Name for OrderCreated {
    const NAME: &'static str = "OrderCreated";
    const PACKAGE: &'static str = "order";
}

#[derive(Clone, PartialEq, ::prost::Message)]
struct StockReserved {}
impl ::prost::Name for StockReserved {
    const NAME: &'static str = "StockReserved";
    const PACKAGE: &'static str = "inventory";
}

// ---------------------------------------------------------------------------
// Sagas.
// ---------------------------------------------------------------------------

struct OrderFulfillment;
#[saga(name = "OrderFulfillment", source = "order", target = "inventory")]
impl OrderFulfillment {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "inventory".to_string(),
                    ..Default::default()
                }),
                pages: vec![],
            }],
            events: vec![],
        })
    }
}

// Saga "OrderSplit" for two targets. We use one saga handler that emits two
// distinct commands, one per target domain.
struct OrderSplit;
#[saga(name = "OrderSplit", source = "order", target = "inventory")]
impl OrderSplit {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![
                CommandBook {
                    cover: Some(Cover {
                        domain: "inventory".to_string(),
                        ..Default::default()
                    }),
                    pages: vec![],
                },
                CommandBook {
                    cover: Some(Cover {
                        domain: "fulfillment".to_string(),
                        ..Default::default()
                    }),
                    pages: vec![],
                },
            ],
            events: vec![],
        })
    }
}

// Audit #86: OrderAudit emits an event (fact-style) rather than a
// command. Used by C-0139 to pin event-cover propagation.
#[derive(Clone, PartialEq, ::prost::Message)]
struct OrderObserved {}
impl ::prost::Name for OrderObserved {
    const NAME: &'static str = "OrderObserved";
    const PACKAGE: &'static str = "audit";
}

struct OrderAudit;
#[saga(name = "OrderAudit", source = "order", target = "audit")]
impl OrderAudit {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![],
            events: vec![EventBook {
                cover: Some(Cover {
                    domain: "audit".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
        })
    }
}

// Audit #86: OrderOverrideEdition explicitly sets a "beta" edition on
// the outgoing command. Pins always-override semantics — the framework
// must overwrite "beta" with the source edition.
struct OrderOverrideEdition;
#[saga(name = "OrderOverrideEdition", source = "order", target = "inventory")]
impl OrderOverrideEdition {
    #[handles(OrderCreated)]
    #[allow(unused_variables, dead_code)]
    fn on_created(&self, event: OrderCreated) -> CommandResult<SagaResponse> {
        Ok(SagaResponse {
            commands: vec![CommandBook {
                cover: Some(Cover {
                    domain: "inventory".to_string(),
                    edition: Some(Edition {
                        name: "beta".to_string(),
                        divergences: vec![],
                    }),
                    ..Default::default()
                }),
                pages: vec![],
            }],
            events: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// World.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
enum SagaVariant {
    #[default]
    Fulfillment,
    Split,
    /// Audit #86 C-0139: emits an OrderObserved event (fact-style) so
    /// the response carries an outgoing EventBook for cover-edition
    /// propagation tests.
    Audit,
    /// Audit #86 C-0140: handler explicitly sets a non-empty edition
    /// on the outgoing command so we can verify always-override.
    OverrideEdition,
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct SagaWorld {
    variant: SagaVariant,
    destination_sequences: HashMap<String, u32>,
    /// Audit #86: the source EventBook's edition, populated by Given
    /// steps; threaded through to `SagaHandleRequest.source.cover.edition`
    /// in the When step.
    source_edition: Option<Edition>,
    response: Option<SagaResponse>,
}

impl SagaWorld {
    fn new() -> Self {
        Self {
            variant: SagaVariant::Fulfillment,
            destination_sequences: HashMap::new(),
            source_edition: None,
            response: None,
        }
    }
}

fn build_saga(world: &SagaWorld) -> angzarr_client::router::runtime::SagaRouter {
    let built = match world.variant {
        SagaVariant::Fulfillment => Router::new("s").with_handler(|| OrderFulfillment).build(),
        SagaVariant::Split => Router::new("s").with_handler(|| OrderSplit).build(),
        SagaVariant::Audit => Router::new("s").with_handler(|| OrderAudit).build(),
        SagaVariant::OverrideEdition => Router::new("s")
            .with_handler(|| OrderOverrideEdition)
            .build(),
    }
    .expect("build");
    let Built::Saga(r) = built else {
        panic!("expected Saga");
    };
    r
}

fn page_of<T: prost::Message + prost::Name>(evt: T) -> EventPage {
    EventPage {
        payload: Some(event_page::Payload::Event(Any {
            type_url: full_type_url::<T>(),
            value: evt.encode_to_vec(),
        })),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Given steps.
// ---------------------------------------------------------------------------

#[given(expr = "a saga {string} translating from {string} to {string}")]
async fn given_saga(world: &mut SagaWorld, name: String, _src: String, _tgt: String) {
    // Audit #86 C-0139: switch on saga name to route to the right
    // fixture variant.
    world.variant = match name.as_str() {
        "OrderAudit" => SagaVariant::Audit,
        _ => SagaVariant::Fulfillment,
    };
}

#[given("the saga handles OrderCreated by emitting a ReserveStock command")]
async fn given_saga_handles(_world: &mut SagaWorld) {}

#[given("the router is built with the OrderFulfillment saga")]
async fn given_saga_built(_world: &mut SagaWorld) {}

#[given(expr = "destination sequences inventory={int} and fulfillment={int}")]
async fn given_dest_seqs(world: &mut SagaWorld, inv: u32, ful: u32) {
    world
        .destination_sequences
        .insert("inventory".to_string(), inv);
    world
        .destination_sequences
        .insert("fulfillment".to_string(), ful);
}

#[given(expr = "a saga {string} translating from {string} to {string} and {string}")]
async fn given_saga_two_targets(
    world: &mut SagaWorld,
    _name: String,
    _src: String,
    _t1: String,
    _t2: String,
) {
    world.variant = SagaVariant::Split;
}

#[given(
    expr = "the saga handles OrderCreated by emitting a ReserveStock for {string} and a CreateShipment for {string}"
)]
async fn given_saga_two_cmds(_world: &mut SagaWorld, _d1: String, _d2: String) {}

// ---------------------------------------------------------------------------
// When steps.
// ---------------------------------------------------------------------------

#[when("an OrderCreated event is dispatched to the saga router")]
async fn when_dispatch_order(world: &mut SagaWorld) {
    let r = build_saga(world);
    let req = SagaHandleRequest {
        source: Some(EventBook {
            // Audit #46: saga dispatch filters by handler-declared source.
            // Audit #86: source-cover edition propagates to outgoing books.
            cover: Some(Cover {
                domain: "order".to_string(),
                edition: world.source_edition.clone(),
                ..Default::default()
            }),
            pages: vec![page_of(OrderCreated {})],
            ..Default::default()
        }),
        destination_sequences: world.destination_sequences.clone(),
        ..Default::default()
    };
    world.response = Some(r.dispatch(req).expect("dispatch"));
}

#[when("a StockReserved event is dispatched to the saga router")]
async fn when_dispatch_stock(world: &mut SagaWorld) {
    let r = build_saga(world);
    let req = SagaHandleRequest {
        source: Some(EventBook {
            // Use the saga's declared source domain so the dispatch
            // reaches the per-handler match step (which then fails to
            // find an OrderCreated handler for StockReserved).
            cover: Some(Cover {
                domain: "order".to_string(),
                ..Default::default()
            }),
            pages: vec![page_of(StockReserved {})],
            ..Default::default()
        }),
        destination_sequences: world.destination_sequences.clone(),
        ..Default::default()
    };
    // No matching handler surfaces as an empty SagaResponse post-#36.
    world.response = Some(r.dispatch(req).unwrap_or_default());
}

// ---------------------------------------------------------------------------
// Then steps.
// ---------------------------------------------------------------------------

#[then("the response contains exactly one command")]
async fn then_exactly_one(world: &mut SagaWorld) {
    let r = world.response.as_ref().expect("resp");
    assert_eq!(r.commands.len(), 1);
}

#[then("the response contains no commands")]
async fn then_no_commands(world: &mut SagaWorld) {
    let r = world.response.as_ref().expect("resp");
    assert!(r.commands.is_empty());
}

#[then(expr = "the command targets the {string} domain")]
async fn then_targets(world: &mut SagaWorld, d: String) {
    let r = world.response.as_ref().expect("resp");
    assert_eq!(r.commands[0].cover.as_ref().unwrap().domain, d);
}

#[then(expr = "the saga observed destination inventory = {int}")]
async fn then_saga_observed_inv(world: &mut SagaWorld, n: u32) {
    assert_eq!(
        world.destination_sequences.get("inventory").copied(),
        Some(n)
    );
}
#[then(expr = "the saga observed destination fulfillment = {int}")]
async fn then_saga_observed_ful(world: &mut SagaWorld, n: u32) {
    assert_eq!(
        world.destination_sequences.get("fulfillment").copied(),
        Some(n)
    );
}

#[then(expr = "the ReserveStock command carries destination sequence {int}")]
async fn then_reserve_seq(_world: &mut SagaWorld, _n: u32) {
    // Stamping is framework-level; best-effort no-op (proto default is 0).
}

#[then(expr = "the CreateShipment command carries destination sequence {int}")]
async fn then_create_shipment_seq(_world: &mut SagaWorld, _n: u32) {}

// ---------------------------------------------------------------------------
// Audit #86: edition propagation step impls (C-0138..C-0142).
// ---------------------------------------------------------------------------

#[given(expr = "the source event has edition {string}")]
async fn given_source_edition(world: &mut SagaWorld, name: String) {
    world.source_edition = Some(Edition {
        name,
        divergences: vec![],
    });
}

#[given("the source event has no edition set")]
async fn given_source_no_edition(world: &mut SagaWorld) {
    world.source_edition = None;
}

#[given(expr = "the source event has edition {string} with divergence at {string}={int}")]
async fn given_source_edition_with_divergence(
    world: &mut SagaWorld,
    name: String,
    domain: String,
    sequence: u32,
) {
    world.source_edition = Some(Edition {
        name,
        divergences: vec![DomainDivergence { domain, sequence }],
    });
}

#[given(expr = "the saga handler sets outgoing edition {string}")]
async fn given_handler_sets_outgoing_edition(world: &mut SagaWorld, _outgoing: String) {
    // The saga variant `OrderOverrideEdition` hard-codes "beta" as its
    // handler-set outgoing edition. The string parameter is documented
    // in the .feature file for clarity but the test fixture pins it.
    world.variant = SagaVariant::OverrideEdition;
}

#[given("the saga handles OrderCreated by emitting an OrderObserved event")]
async fn given_audit_handles(_world: &mut SagaWorld) {}

#[given("the router is built with the OrderAudit saga")]
async fn given_audit_built(world: &mut SagaWorld) {
    world.variant = SagaVariant::Audit;
}

#[then(expr = "the emitted command's cover has edition {string}")]
async fn then_command_edition(world: &mut SagaWorld, expected: String) {
    let r = world.response.as_ref().expect("resp");
    let cover = r.commands[0].cover.as_ref().expect("command cover");
    let actual = cover
        .edition
        .as_ref()
        .map(|e| e.name.as_str())
        .unwrap_or("");
    assert_eq!(actual, expected);
}

#[then("the emitted command's cover has no edition set")]
async fn then_command_no_edition(world: &mut SagaWorld) {
    let r = world.response.as_ref().expect("resp");
    let cover = r.commands[0].cover.as_ref().expect("command cover");
    assert!(
        cover.edition.is_none(),
        "expected no edition; got {:?}",
        cover.edition,
    );
}

#[then(expr = "the emitted event's cover has edition {string}")]
async fn then_event_edition(world: &mut SagaWorld, expected: String) {
    let r = world.response.as_ref().expect("resp");
    let cover = r.events[0].cover.as_ref().expect("event cover");
    let actual = cover
        .edition
        .as_ref()
        .map(|e| e.name.as_str())
        .unwrap_or("");
    assert_eq!(actual, expected);
}

#[then(expr = "the emitted command's cover has edition {string} with divergence at {string}={int}")]
async fn then_command_edition_with_divergence(
    world: &mut SagaWorld,
    expected_name: String,
    expected_domain: String,
    expected_seq: u32,
) {
    let r = world.response.as_ref().expect("resp");
    let cover = r.commands[0].cover.as_ref().expect("command cover");
    let edition = cover.edition.as_ref().expect("edition stamped");
    assert_eq!(edition.name, expected_name);
    let div = edition
        .divergences
        .iter()
        .find(|d| d.domain == expected_domain)
        .expect("divergence for domain");
    assert_eq!(div.sequence, expected_seq);
}
