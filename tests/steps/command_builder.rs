//! Step definitions for `parity/client/command_builder.feature`.
//!
//! Every scenario drives the real [`CommandBuilder`] obtained from
//! [`CommandBuilderExt`]. Steps record only what the scenario sets; the
//! builder is then run with exactly those calls, so its own defaults and
//! validation decide the outcome.

use std::sync::Arc;

use angzarr_client::proto::{
    command_page, page_header::SequenceType, CommandBook, CommandResponse, MergeStrategy,
};
use angzarr_client::traits::GatewayClient;
use angzarr_client::{full_type_url, ClientError, CommandBuilderExt, CommandHandlerClient};
use cucumber::{given, then, when, World};
use prost::Message;
use uuid::Uuid;

use crate::common::backend::{root_for, Hidden};
use crate::common::fixtures::CreateOrder;
use angzarr_client::testing::RecordingGatewayClient;

/// Generic payload for "the command type and payload".
#[derive(Clone, PartialEq, Message)]
pub struct TestCommand {
    #[prost(string, tag = "1")]
    pub data: String,
}

const TEST_COMMAND_URL: &str = "type.googleapis.com/test.TestCommand";

/// What a scenario asked the builder to do.
#[derive(Debug, Default, Clone)]
struct Recipe {
    domain: String,
    /// `None` → `command_new` (auto-generated root).
    root: Option<Uuid>,
    correlation_id: Option<String>,
    sequence: Option<u32>,
    merge: Option<MergeStrategy>,
    /// Payload passed to `with_command`.
    command: Option<Payload>,
    /// Type URL set on its own via `with_type_url`.
    type_url: Option<String>,
    /// Encoded payload set on its own via `with_payload`.
    payload: Option<Vec<u8>>,
}

/// Typed payload a recipe passes to `with_command`.
#[derive(Debug, Clone)]
enum Payload {
    CreateOrder(CreateOrder),
    Test(TestCommand),
}

fn apply<'a, C: GatewayClient>(client: &'a C, r: &Recipe) -> angzarr_client::CommandBuilder<'a, C> {
    let mut b = match r.root {
        Some(root) => client.command(&r.domain, root),
        None => client.command_new(&r.domain),
    };
    if let Some(id) = &r.correlation_id {
        b = b.with_correlation_id(id);
    }
    if let Some(seq) = r.sequence {
        b = b.with_sequence(seq);
    }
    if let Some(m) = r.merge {
        b = b.with_merge_strategy(m);
    }
    match &r.command {
        Some(Payload::CreateOrder(m)) => b = b.with_command(full_type_url::<CreateOrder>(), m),
        Some(Payload::Test(m)) => b = b.with_command(TEST_COMMAND_URL, m),
        None => {}
    }
    if let Some(url) = &r.type_url {
        b = b.with_type_url(url.clone());
    }
    if let Some(bytes) = &r.payload {
        b = b.with_payload(bytes.clone());
    }
    b
}

#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct CommandBuilderWorld {
    mock: Arc<RecordingGatewayClient>,
    real: Hidden<CommandHandlerClient>,
    recipe: Recipe,
    built: Option<Result<CommandBook, ClientError>>,
    built_pair: Vec<CommandBook>,
    pair_roots: Vec<Uuid>,
    executed: Option<Result<CommandResponse, ClientError>>,
}

impl CommandBuilderWorld {
    fn new() -> Self {
        Self {
            mock: Arc::new(RecordingGatewayClient::new()),
            real: Hidden::default(),
            recipe: Recipe::default(),
            built: None,
            built_pair: Vec::new(),
            pair_roots: Vec::new(),
            executed: None,
        }
    }

    fn build(&mut self) {
        let result = if self.real.is_some() {
            apply(self.real.get(), &self.recipe).build()
        } else {
            apply(self.mock.as_ref(), &self.recipe).build()
        };
        self.built = Some(result);
    }

    fn built(&self) -> &CommandBook {
        match self.built.as_ref().expect("a command was built") {
            Ok(b) => b,
            Err(e) => panic!("build failed: {e:?}"),
        }
    }

    fn cover(&self) -> &angzarr_client::proto::Cover {
        self.built().cover.as_ref().expect("cover")
    }

    fn page(&self) -> &angzarr_client::proto::CommandPage {
        self.built().pages.first().expect("a command page")
    }
}

fn create_order_command() -> Payload {
    Payload::CreateOrder(CreateOrder {
        order_id: "o-1".into(),
        customer_id: "c-1".into(),
        items: vec![],
    })
}

fn test_command() -> Payload {
    Payload::Test(TestCommand {
        data: "test".into(),
    })
}

fn canned_response() -> CommandResponse {
    CommandResponse {
        events: Some(angzarr_client::proto::EventBook {
            next_sequence: 42,
            ..Default::default()
        }),
        projections: vec![],
        ..Default::default()
    }
}

fn root_bytes(cover: &angzarr_client::proto::Cover) -> [u8; 16] {
    cover
        .root
        .as_ref()
        .expect("root present")
        .value
        .as_slice()
        .try_into()
        .expect("16-byte UUID root")
}

// --------------------------------------------------------------------------
// Arrangement
// --------------------------------------------------------------------------

#[given("a mock CommandHandlerClient for testing")]
async fn given_mock(world: &mut CommandBuilderWorld) {
    world.mock = Arc::new(RecordingGatewayClient::new());
}

#[given("a CommandHandlerClient implementation")]
async fn given_real_client(world: &mut CommandBuilderWorld) {
    let channel = tonic::transport::Endpoint::from_static("http://127.0.0.1:1").connect_lazy();
    world.real.set(CommandHandlerClient::from_channel(channel));
}

#[given(expr = "a builder configured for domain {string}")]
async fn given_builder_for(world: &mut CommandBuilderWorld, domain: String) {
    world.recipe.domain = domain;
}

// --------------------------------------------------------------------------
// Recipe steps
// --------------------------------------------------------------------------

#[when(expr = "I build a command for domain {string} root {string}")]
async fn when_domain_root(world: &mut CommandBuilderWorld, domain: String, root: String) {
    world.recipe.domain = domain;
    world.recipe.root = Some(root_for(&root));
}

#[when(expr = "I build a command for domain {string}")]
async fn when_domain(world: &mut CommandBuilderWorld, domain: String) {
    world.recipe.domain = domain;
    world.recipe.root = Some(Uuid::new_v4());
}

#[when(expr = "I build a command for new aggregate in domain {string}")]
async fn when_new_aggregate(world: &mut CommandBuilderWorld, domain: String) {
    world.recipe.domain = domain;
    world.recipe.root = None;
}

#[when(expr = "I set the command type to {string}")]
async fn when_set_type(world: &mut CommandBuilderWorld, name: String) {
    assert_eq!(name, "CreateOrder", "fixtures provide CreateOrder");
    world.recipe.type_url = Some(full_type_url::<CreateOrder>());
}

#[when("I set the command payload")]
async fn when_set_payload(world: &mut CommandBuilderWorld) {
    let Payload::CreateOrder(cmd) = create_order_command() else {
        unreachable!("create_order_command builds a CreateOrder");
    };
    world.recipe.payload = Some(prost::Message::encode_to_vec(&cmd));
    world.build();
}

#[when("I set the command type and payload")]
async fn when_set_type_and_payload(world: &mut CommandBuilderWorld) {
    world.recipe.command = Some(test_command());
    world.build();
}

#[when(expr = "I set correlation ID to {string}")]
async fn when_set_correlation(world: &mut CommandBuilderWorld, id: String) {
    world.recipe.correlation_id = Some(id);
}

#[when(expr = "I set sequence to {int}")]
async fn when_set_sequence(world: &mut CommandBuilderWorld, seq: u32) {
    world.recipe.sequence = Some(seq);
}

#[when("I do NOT set the command type")]
async fn when_no_type(world: &mut CommandBuilderWorld) {
    world.recipe.command = None;
    world.build();
}

/// `with_command(type_url, msg)` is the only way to set either value, so
/// "type without payload" cannot be expressed: the recipe keeps no command
/// and the builder reports what is missing.
#[when("I do NOT set the payload")]
async fn when_no_payload(world: &mut CommandBuilderWorld) {
    world.recipe.payload = None;
    world.recipe.command = None;
    world.build();
}

#[when("I build a command without specifying merge strategy")]
async fn when_default_merge(world: &mut CommandBuilderWorld) {
    world.recipe = Recipe {
        domain: "orders".into(),
        root: Some(Uuid::new_v4()),
        command: Some(test_command()),
        ..Default::default()
    };
    world.build();
}

#[when("I build a command with merge strategy STRICT")]
async fn when_strict_merge(world: &mut CommandBuilderWorld) {
    world.recipe = Recipe {
        domain: "orders".into(),
        root: Some(Uuid::new_v4()),
        merge: Some(MergeStrategy::MergeStrict),
        command: Some(test_command()),
        ..Default::default()
    };
    world.build();
}

#[when("I build a command using fluent chaining:")]
async fn when_fluent(world: &mut CommandBuilderWorld) {
    world.recipe = Recipe {
        domain: "orders".into(),
        root: Some(root_for("order-chained")),
        correlation_id: Some("trace-456".into()),
        sequence: Some(3),
        command: Some(create_order_command()),
        ..Default::default()
    };
    world.build();
}

#[when(expr = "I build and execute a command for domain {string}")]
async fn when_build_execute(world: &mut CommandBuilderWorld, domain: String) {
    world.recipe = Recipe {
        domain,
        root: Some(Uuid::new_v4()),
        command: Some(test_command()),
        ..Default::default()
    };
    world.mock.respond(canned_response());
    world.built = Some(apply(world.mock.as_ref(), &world.recipe).build());
    world.executed = Some(apply(world.mock.as_ref(), &world.recipe).execute().await);
}

#[when("I use the builder to execute directly:")]
async fn when_execute_directly(world: &mut CommandBuilderWorld) {
    world.recipe = Recipe {
        domain: "orders".into(),
        root: Some(Uuid::new_v4()),
        command: Some(create_order_command()),
        ..Default::default()
    };
    world.executed = Some(apply(world.mock.as_ref(), &world.recipe).execute().await);
}

#[when("I create two commands with different roots")]
async fn when_two_roots(world: &mut CommandBuilderWorld) {
    let roots = [root_for("pair-a"), root_for("pair-b")];
    for root in roots {
        let mut recipe = world.recipe.clone();
        recipe.root = Some(root);
        recipe.command = Some(test_command());
        let book = apply(world.mock.as_ref(), &recipe)
            .build()
            .expect("builder builds each command");
        world.built_pair.push(book);
    }
    world.pair_roots = roots.to_vec();
}

#[when(expr = "I call client.command\\({string}, root\\)")]
async fn when_call_command(world: &mut CommandBuilderWorld, domain: String) {
    world.recipe = Recipe {
        domain,
        root: Some(root_for("shortcut-root")),
        command: Some(test_command()),
        ..Default::default()
    };
    world.build();
}

#[when(expr = "I call client.command_new\\({string}\\)")]
async fn when_call_command_new(world: &mut CommandBuilderWorld, domain: String) {
    world.recipe = Recipe {
        domain,
        root: None,
        command: Some(test_command()),
        ..Default::default()
    };
    world.build();
}

// --------------------------------------------------------------------------
// Outcomes
// --------------------------------------------------------------------------

#[then(expr = "the built command should have domain {string}")]
async fn then_domain(world: &mut CommandBuilderWorld, domain: String) {
    assert_eq!(world.cover().domain, domain);
}

#[then(expr = "the built command should have root {string}")]
async fn then_root(world: &mut CommandBuilderWorld, root: String) {
    assert_eq!(root_bytes(world.cover()), *root_for(&root).as_bytes());
}

#[then("the built command should have an auto-generated UUID root")]
async fn then_auto_root(world: &mut CommandBuilderWorld) {
    assert!(world.recipe.root.is_none(), "scenario used command_new");
    assert_ne!(root_bytes(world.cover()), [0u8; 16]);
}

#[then("the auto-generated root should be a valid UUID")]
async fn then_auto_root_v4(world: &mut CommandBuilderWorld) {
    let uuid = Uuid::from_bytes(root_bytes(world.cover()));
    assert_eq!(uuid.get_version_num(), 4);
}

#[then(expr = "the built command should have type URL containing {string}")]
async fn then_type_url(world: &mut CommandBuilderWorld, part: String) {
    match &world.page().payload {
        Some(command_page::Payload::Command(any)) => {
            assert!(any.type_url.contains(&part), "type_url: {}", any.type_url)
        }
        other => panic!("expected command payload, got {other:?}"),
    }
}

#[then("the built command should have a non-empty correlation ID")]
async fn then_nonempty_correlation(world: &mut CommandBuilderWorld) {
    assert!(!world.cover().correlation_id.is_empty());
}

#[then("the correlation ID should be a valid UUID")]
async fn then_correlation_uuid(world: &mut CommandBuilderWorld) {
    Uuid::parse_str(&world.cover().correlation_id).expect("correlation id is a UUID");
}

#[then(expr = "the built command should have correlation ID {string}")]
async fn then_correlation(world: &mut CommandBuilderWorld, id: String) {
    assert_eq!(world.cover().correlation_id, id);
}

#[then(expr = "the built command should have sequence {int}")]
async fn then_sequence(world: &mut CommandBuilderWorld, seq: u32) {
    let header = world.page().header.as_ref().expect("page header");
    assert_eq!(header.sequence_type, Some(SequenceType::Sequence(seq)));
}

#[then("building should fail")]
async fn then_build_fails(world: &mut CommandBuilderWorld) {
    let built = world.built.as_ref().expect("a build was attempted");
    assert!(built.is_err(), "build unexpectedly succeeded: {built:?}");
}

#[then("the error should indicate missing type URL")]
async fn then_missing_type(world: &mut CommandBuilderWorld) {
    let err = world
        .built
        .as_ref()
        .expect("built")
        .as_ref()
        .expect_err("build failed");
    assert_eq!(
        err.code(),
        angzarr_client::error_codes::codes::COMMAND_TYPE_URL_MISSING
    );
}

#[then("the error should indicate missing payload")]
async fn then_missing_payload(world: &mut CommandBuilderWorld) {
    let err = world
        .built
        .as_ref()
        .expect("built")
        .as_ref()
        .expect_err("build failed");
    assert_eq!(
        err.code(),
        angzarr_client::error_codes::codes::COMMAND_PAYLOAD_MISSING
    );
}

#[then("the build should succeed")]
async fn then_build_ok(world: &mut CommandBuilderWorld) {
    world.built();
}

#[then("all chained values should be preserved")]
async fn then_chained(world: &mut CommandBuilderWorld) {
    let cover = world.cover();
    assert_eq!(cover.domain, "orders");
    assert_eq!(root_bytes(cover), *root_for("order-chained").as_bytes());
    assert_eq!(cover.correlation_id, "trace-456");
    let header = world.page().header.as_ref().expect("header");
    assert_eq!(header.sequence_type, Some(SequenceType::Sequence(3)));
    match &world.page().payload {
        Some(command_page::Payload::Command(any)) => {
            assert_eq!(any.type_url, full_type_url::<CreateOrder>());
            let cmd = CreateOrder::decode(any.value.as_slice()).expect("decodes");
            assert_eq!(cmd.customer_id, "c-1");
        }
        other => panic!("expected command payload, got {other:?}"),
    }
}

#[then("the command should be sent to the gateway")]
async fn then_sent(world: &mut CommandBuilderWorld) {
    let call = world
        .mock
        .last_call("execute")
        .expect("gateway received execute");
    let built = world.built();
    // build() and execute() each mint a fresh correlation id, so compare
    // the addressing and the command itself.
    let sent = call.command.cover.as_ref().expect("sent cover");
    let expected = built.cover.as_ref().expect("built cover");
    assert_eq!(
        (&sent.domain, &sent.root),
        (&expected.domain, &expected.root)
    );
    assert!(!sent.correlation_id.is_empty());
    // execute() additionally stamps its sync mode into the page header.
    assert_eq!(call.command.pages.len(), 1);
    assert_eq!(call.command.pages[0].payload, built.pages[0].payload);
    assert_eq!(
        call.command.pages[0]
            .header
            .as_ref()
            .map(|h| h.sequence_type.clone()),
        built.pages[0]
            .header
            .as_ref()
            .map(|h| h.sequence_type.clone())
    );
}

#[then("the response should be returned")]
async fn then_response(world: &mut CommandBuilderWorld) {
    let resp = world
        .executed
        .as_ref()
        .expect("executed")
        .as_ref()
        .expect("execute succeeds");
    assert_eq!(resp, &canned_response());
}

#[then("the command should be built and executed in one call")]
async fn then_built_and_executed(world: &mut CommandBuilderWorld) {
    world
        .executed
        .as_ref()
        .expect("executed")
        .as_ref()
        .expect("execute succeeds");
    assert_eq!(world.mock.call_count("execute"), 1);
    let call = world.mock.last_call("execute").expect("recorded");
    match &call.command.pages[0].payload {
        Some(command_page::Payload::Command(any)) => {
            assert_eq!(any.type_url, full_type_url::<CreateOrder>())
        }
        other => panic!("expected command payload, got {other:?}"),
    }
}

#[then("the command page should have MERGE_COMMUTATIVE strategy")]
async fn then_commutative(world: &mut CommandBuilderWorld) {
    assert_eq!(
        world.page().merge_strategy,
        MergeStrategy::MergeCommutative as i32
    );
}

#[then("the command page should have MERGE_STRICT strategy")]
async fn then_strict(world: &mut CommandBuilderWorld) {
    assert_eq!(
        world.page().merge_strategy,
        MergeStrategy::MergeStrict as i32
    );
}

#[then("each command should have its own root")]
async fn then_own_roots(world: &mut CommandBuilderWorld) {
    let roots: Vec<[u8; 16]> = world
        .built_pair
        .iter()
        .map(|b| root_bytes(b.cover.as_ref().expect("cover")))
        .collect();
    let expected: Vec<[u8; 16]> = world.pair_roots.iter().map(|u| *u.as_bytes()).collect();
    assert_eq!(roots, expected);
}

#[then("builder reuse should not cause cross-contamination")]
async fn then_no_contamination(world: &mut CommandBuilderWorld) {
    assert_eq!(world.built_pair.len(), 2);
    let a = world.built_pair[0].cover.as_ref().expect("cover");
    let b = world.built_pair[1].cover.as_ref().expect("cover");
    assert_eq!(a.domain, world.recipe.domain);
    assert_eq!(b.domain, world.recipe.domain);
    assert_ne!(
        a.correlation_id, b.correlation_id,
        "each build gets its own correlation id"
    );
}

#[then("I should receive a CommandBuilder for that domain and root")]
async fn then_builder_domain_root(world: &mut CommandBuilderWorld) {
    let cover = world.cover();
    assert_eq!(cover.domain, world.recipe.domain);
    assert_eq!(root_bytes(cover), *root_for("shortcut-root").as_bytes());
}

#[then("I should receive a CommandBuilder for that domain and an auto-generated root")]
async fn then_builder_domain_auto_root(world: &mut CommandBuilderWorld) {
    let cover = world.cover();
    assert_eq!(cover.domain, world.recipe.domain);
    assert_eq!(Uuid::from_bytes(root_bytes(cover)).get_version_num(), 4);
}
