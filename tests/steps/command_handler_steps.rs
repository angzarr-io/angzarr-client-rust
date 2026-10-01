//! Step definitions for `features/client/command_handler.feature`.
//!
//! Every scenario dispatches a real `ContextualCommand` through a
//! `CommandHandlerRouter` built from `#[command_handler]` types. The
//! handler's behaviour (what it emits, which `cover.ext` it sets) comes from
//! a per-scenario [`OrderBehaviour`] captured by the factory closure, and
//! what the handler observed is recorded there for the Then steps.

use std::sync::{Arc, Mutex};

use angzarr_client::proto::{
    business_response, command_page, event_page, BusinessResponse, CommandBook, CommandPage,
    ContextualCommand, Cover, EventBook, EventPage, PageHeader,
};
use angzarr_client::router::CommandHandlerRouter;
use angzarr_client::router::{Built, Router};
use angzarr_client::{command_handler, ClientError, CommandResult};
use cucumber::{given, then, when, World};
use prost::{Message, Name};
use prost_types::Any;

use crate::common::fixtures::{CompleteOrder, CreateOrder, OrderCreated};

#[derive(Default)]
pub struct OrderState {
    created: bool,
}

/// What `handle_create` does when CreateOrder arrives.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Emit {
    /// Always emit one OrderCreated.
    #[default]
    Always,
    /// Emit nothing.
    Nothing,
    /// Emit OrderCreated only when the rebuilt state says created.
    OnlyIfCreated,
}

#[derive(Debug, Default)]
struct OrderBehaviour {
    emit: Mutex<Emit>,
    /// `cover.ext` the handler stamps on its own output, if any.
    handler_ext: Mutex<Option<Any>>,
    /// `state.created` as observed by the last `handle_create` call.
    observed_created: Mutex<Option<bool>>,
}

fn pack<M: Message + Name>(msg: &M) -> Any {
    Any {
        type_url: angzarr_client::full_type_url::<M>(),
        value: msg.encode_to_vec(),
    }
}

fn handle_create_body(behaviour: &OrderBehaviour, state: &OrderState) -> EventBook {
    *behaviour.observed_created.lock().unwrap() = Some(state.created);
    let emit = *behaviour.emit.lock().unwrap();
    let emits = match emit {
        Emit::Always => true,
        Emit::Nothing => false,
        Emit::OnlyIfCreated => state.created,
    };
    let pages = if emits {
        vec![EventPage {
            payload: Some(event_page::Payload::Event(pack(&OrderCreated {
                order_id: "o-1".into(),
                ..Default::default()
            }))),
            ..Default::default()
        }]
    } else {
        vec![]
    };
    let ext = behaviour.handler_ext.lock().unwrap().clone();
    EventBook {
        cover: ext.map(|ext| Cover {
            domain: "order".into(),
            ext: Some(ext),
            ..Default::default()
        }),
        pages,
        ..Default::default()
    }
}

/// Order aggregate built from the state type's default constructor.
pub struct Order {
    behaviour: Arc<OrderBehaviour>,
}

#[command_handler(domain = "order", state = OrderState)]
impl Order {
    #[applies(OrderCreated)]
    fn on_created(state: &mut OrderState, _evt: OrderCreated) {
        state.created = true;
    }

    #[handles(CreateOrder)]
    fn handle_create(
        &self,
        _cmd: CreateOrder,
        state: &OrderState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(handle_create_body(&self.behaviour, state))
    }
}

/// Order aggregate that supplies its own initial state (created = true).
pub struct SeededOrder {
    behaviour: Arc<OrderBehaviour>,
}

#[command_handler(domain = "order", state = OrderState)]
impl SeededOrder {
    #[state_factory]
    fn initial() -> OrderState {
        OrderState { created: true }
    }

    #[applies(OrderCreated)]
    fn on_created(state: &mut OrderState, _evt: OrderCreated) {
        state.created = true;
    }

    #[handles(CreateOrder)]
    fn handle_create(
        &self,
        _cmd: CreateOrder,
        state: &OrderState,
        _seq: u32,
    ) -> CommandResult<EventBook> {
        Ok(handle_create_body(&self.behaviour, state))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum InitialState {
    #[default]
    Default,
    Supplied,
}

#[derive(Debug, Default, World)]
pub struct CommandHandlerWorld {
    behaviour: Arc<OrderBehaviour>,
    initial: InitialState,
    prior: Option<EventBook>,
    command_ext: Option<Any>,
    response: Option<BusinessResponse>,
    error: Option<ClientError>,
}

impl CommandHandlerWorld {
    fn router(&self) -> CommandHandlerRouter {
        let behaviour = Arc::clone(&self.behaviour);
        let built = match self.initial {
            InitialState::Default => Router::new("order")
                .with_handler(move || Order {
                    behaviour: Arc::clone(&behaviour),
                })
                .build(),
            InitialState::Supplied => Router::new("order")
                .with_handler(move || SeededOrder {
                    behaviour: Arc::clone(&behaviour),
                })
                .build(),
        }
        .expect("router builds");
        match built {
            Built::CommandHandler(r) => r,
            other => panic!("expected a command-handler router, got {other:?}"),
        }
    }

    fn dispatch<C: Message + Name>(&mut self, cmd: &C) {
        let ctx = ContextualCommand {
            command: Some(CommandBook {
                cover: Some(Cover {
                    domain: "order".into(),
                    ext: self.command_ext.clone(),
                    ..Default::default()
                }),
                pages: vec![CommandPage {
                    header: Some(PageHeader::default()),
                    payload: Some(command_page::Payload::Command(pack(cmd))),
                    ..Default::default()
                }],
            }),
            events: self.prior.clone(),
        };
        match self.router().dispatch(ctx) {
            Ok(r) => self.response = Some(r),
            Err(e) => self.error = Some(e),
        }
    }

    fn events(&self) -> &EventBook {
        match self.response.as_ref().map(|r| &r.result) {
            Some(Some(business_response::Result::Events(book))) => book,
            other => panic!(
                "expected an Events response, got {other:?} / {:?}",
                self.error
            ),
        }
    }
}

fn parent_cover_ext(domain: &str) -> Any {
    pack(&Cover {
        domain: domain.into(),
        correlation_id: format!("{domain}-corr"),
        ..Default::default()
    })
}

// --- Given -----------------------------------------------------------------

#[given(expr = "a command handler {string} for domain {string} with order state")]
fn given_command_handler(world: &mut CommandHandlerWorld, name: String, domain: String) {
    assert_eq!(name, "Order");
    assert_eq!(domain, "order");
    world.initial = InitialState::Default;
}

#[given("OrderCreated marks the order as created")]
fn given_applier(_world: &mut CommandHandlerWorld) {
    // `#[applies(OrderCreated)]` on both aggregate types sets created = true.
}

#[given("CreateOrder emits OrderCreated")]
fn given_emits(world: &mut CommandHandlerWorld) {
    *world.behaviour.emit.lock().unwrap() = Emit::Always;
}

#[given("Order is the active aggregate handler")]
fn given_active(world: &mut CommandHandlerWorld) {
    let router = world.router();
    assert_eq!(router.name(), "order");
    assert_eq!(router.handler_count(), 1);
}

#[given(expr = "a prior history with an OrderCreated event at sequence {int}")]
fn given_prior(world: &mut CommandHandlerWorld, seq: u32) {
    world.prior = Some(EventBook {
        cover: Some(Cover {
            domain: "order".into(),
            ..Default::default()
        }),
        pages: vec![EventPage {
            header: Some(PageHeader {
                sequence_type: Some(angzarr_client::proto::page_header::SequenceType::Sequence(
                    seq,
                )),
                sync_mode: None,
            }),
            payload: Some(event_page::Payload::Event(pack(&OrderCreated::default()))),
            ..Default::default()
        }],
        next_sequence: seq + 1,
        ..Default::default()
    });
}

#[given("a command handler whose handler returns None for CreateOrder")]
fn given_emits_nothing(world: &mut CommandHandlerWorld) {
    *world.behaviour.emit.lock().unwrap() = Emit::Nothing;
}

#[given("the aggregate supplies its own initial state with created = true")]
fn given_supplied_state(world: &mut CommandHandlerWorld) {
    world.initial = InitialState::Supplied;
}

#[given("the aggregate does not supply its own initial state")]
fn given_default_state(world: &mut CommandHandlerWorld) {
    world.initial = InitialState::Default;
}

#[given(
    "Order handles CreateOrder by emitting OrderCreated only when the order is already created"
)]
fn given_emit_if_created(world: &mut CommandHandlerWorld) {
    *world.behaviour.emit.lock().unwrap() = Emit::OnlyIfCreated;
}

#[given("Order handles CreateOrder by reading whether the order is created")]
fn given_reads_state(world: &mut CommandHandlerWorld) {
    *world.behaviour.emit.lock().unwrap() = Emit::Always;
}

#[given("no prior events in the incoming ContextualCommand")]
fn given_no_prior(world: &mut CommandHandlerWorld) {
    world.prior = None;
}

#[given("the incoming command has cover.ext set to a packed parent Cover")]
fn given_command_ext(world: &mut CommandHandlerWorld) {
    world.command_ext = Some(parent_cover_ext("tournament"));
}

#[given("a command handler whose emit step sets EventBook cover.ext explicitly")]
fn given_handler_ext(world: &mut CommandHandlerWorld) {
    *world.behaviour.handler_ext.lock().unwrap() = Some(parent_cover_ext("handler-set"));
}

#[given("the incoming command also has a different cover.ext set")]
fn given_different_command_ext(world: &mut CommandHandlerWorld) {
    world.command_ext = Some(parent_cover_ext("tournament"));
}

#[given("the incoming command's cover has no ext field set")]
fn given_no_command_ext(world: &mut CommandHandlerWorld) {
    world.command_ext = None;
}

// --- When ------------------------------------------------------------------

#[when(expr = "CreateOrder\\(order_id={string}\\) is dispatched")]
fn when_create(world: &mut CommandHandlerWorld, order_id: String) {
    world.dispatch(&CreateOrder {
        order_id,
        ..Default::default()
    });
}

#[when(expr = "CompleteOrder\\(order_id={string}\\) is dispatched")]
fn when_complete(world: &mut CommandHandlerWorld, order_id: String) {
    world.dispatch(&CompleteOrder { order_id });
}

#[when("a command is dispatched against the aggregate")]
fn when_any_command(world: &mut CommandHandlerWorld) {
    world.dispatch(&CreateOrder::default());
}

// --- Then ------------------------------------------------------------------

#[then("the response emits an OrderCreated event")]
fn then_emits_created(world: &mut CommandHandlerWorld) {
    let book = world.events();
    assert_eq!(book.pages.len(), 1, "pages: {:?}", book.pages);
    match &book.pages[0].payload {
        Some(event_page::Payload::Event(any)) => {
            assert_eq!(
                any.type_url,
                angzarr_client::full_type_url::<OrderCreated>()
            )
        }
        other => panic!("expected an event payload, got {other:?}"),
    }
}

#[then(expr = "the emitted event sequence is {int}")]
fn then_sequence(world: &mut CommandHandlerWorld, seq: u32) {
    let book = world.events();
    let header = book.pages[0].header.as_ref().expect("page header");
    assert_eq!(
        header.sequence_type,
        Some(angzarr_client::proto::page_header::SequenceType::Sequence(
            seq
        ))
    );
}

#[then("the order is treated as already created")]
fn then_already_created(world: &mut CommandHandlerWorld) {
    assert_eq!(
        *world.behaviour.observed_created.lock().unwrap(),
        Some(true)
    );
}

#[then("the handler observes that the order is not created")]
fn then_not_created(world: &mut CommandHandlerWorld) {
    assert_eq!(
        *world.behaviour.observed_created.lock().unwrap(),
        Some(false)
    );
}

#[then("the unknown command is rejected as invalid input")]
fn then_unknown_rejected(world: &mut CommandHandlerWorld) {
    assert!(
        world.response.is_none(),
        "unexpected response {:?}",
        world.response
    );
    let err = world.error.as_ref().expect("dispatch error");
    assert!(
        matches!(err, ClientError::InvalidArgument(_)),
        "got {err:?}"
    );
    assert_eq!(
        err.code(),
        angzarr_client::error_codes::codes::NO_HANDLER_REGISTERED
    );
}

#[then("when the handler emits nothing, no events are produced")]
fn then_nothing(world: &mut CommandHandlerWorld) {
    assert!(world.events().pages.is_empty());
    assert_eq!(
        *world.behaviour.observed_created.lock().unwrap(),
        Some(false)
    );
}

#[then("the response's EventBook cover.ext is the same packed parent Cover")]
fn then_ext_propagated(world: &mut CommandHandlerWorld) {
    let ext = world.events().cover.as_ref().and_then(|c| c.ext.clone());
    assert_eq!(ext, Some(parent_cover_ext("tournament")));
}

#[then("the response's EventBook cover.ext is the handler-set value")]
fn then_handler_ext_kept(world: &mut CommandHandlerWorld) {
    let ext = world.events().cover.as_ref().and_then(|c| c.ext.clone());
    assert_eq!(ext, Some(parent_cover_ext("handler-set")));
}

#[then("the response's EventBook cover has no ext field set")]
fn then_no_ext(world: &mut CommandHandlerWorld) {
    let book = world.events();
    assert_eq!(book.pages.len(), 1);
    assert_eq!(book.cover.as_ref().and_then(|c| c.ext.clone()), None);
}
