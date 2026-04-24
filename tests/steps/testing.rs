//! Step definitions for features/client/testing.feature.

use angzarr_client::{
    make_cover, make_event_book, uuid_for, uuid_obj_for, uuid_str_for, ScenarioContext,
    DEFAULT_TEST_NAMESPACE,
};
use cucumber::{given, then, when, World};
use uuid::Uuid;

#[derive(Debug, Default, World)]
pub struct TestingWorld {
    last_bytes: Option<[u8; 16]>,
    last_str: Option<String>,
    last_obj: Option<Uuid>,
    derived_root: Option<[u8; 16]>,
    cover: Option<angzarr_client::proto::Cover>,
    event_book: Option<angzarr_client::proto::EventBook>,
    context: Option<ScenarioContext>,
}

// --- Given -----------------------------------------------------------------

#[given(expr = "a 16-byte root derived from the name {string}")]
async fn given_derived_root(world: &mut TestingWorld, name: String) {
    world.derived_root = Some(uuid_for(&name, DEFAULT_TEST_NAMESPACE));
}

#[given(expr = "a cover with domain {string} for that root")]
async fn given_cover_for_root(world: &mut TestingWorld, domain: String) {
    let root = world.derived_root.expect("no derived root");
    world.cover = Some(make_cover(domain, root, ""));
}

#[given(expr = "a ScenarioContext with domain {string} and root bytes for {string}")]
async fn given_scenario_context(world: &mut TestingWorld, domain: String, name: String) {
    let mut ctx = ScenarioContext::new();
    ctx.domain = domain;
    ctx.root = uuid_for(&name, DEFAULT_TEST_NAMESPACE);
    ctx.events
        .push(prost_types::Any::default()); // something to prove reset clears it
    world.context = Some(ctx);
}

// --- When ------------------------------------------------------------------

#[when(expr = "I call uuid_for with the name {string}")]
async fn when_uuid_for(world: &mut TestingWorld, name: String) {
    world.last_bytes = Some(uuid_for(&name, DEFAULT_TEST_NAMESPACE));
}

#[when(expr = "I call uuid_str_for with the name {string}")]
async fn when_uuid_str_for(world: &mut TestingWorld, name: String) {
    world.last_str = Some(uuid_str_for(&name, DEFAULT_TEST_NAMESPACE));
}

#[when(expr = "I call uuid_obj_for with the name {string}")]
async fn when_uuid_obj_for(world: &mut TestingWorld, name: String) {
    world.last_obj = Some(uuid_obj_for(&name, DEFAULT_TEST_NAMESPACE));
}

#[when(expr = "I call make_cover with domain {string} and correlation_id {string}")]
async fn when_make_cover(world: &mut TestingWorld, domain: String, correlation_id: String) {
    let root = world.derived_root.expect("no derived root");
    world.cover = Some(make_cover(domain, root, correlation_id));
}

#[when("I call make_event_book with that cover, no pages, and no explicit next_sequence")]
async fn when_make_event_book_empty(world: &mut TestingWorld) {
    let cover = world.cover.clone().expect("no cover");
    world.event_book = Some(make_event_book(cover, vec![], None));
}

#[when("I reset the scenario context")]
async fn when_reset_context(world: &mut TestingWorld) {
    world.context.as_mut().expect("no context").reset();
}

// --- Then ------------------------------------------------------------------

#[then(expr = "DEFAULT_TEST_NAMESPACE equals the UUID {string}")]
async fn then_default_namespace(_world: &mut TestingWorld, expected: String) {
    assert_eq!(DEFAULT_TEST_NAMESPACE.to_string(), expected);
}

#[then(expr = "the 16 returned bytes match the hex {string}")]
async fn then_bytes_hex(world: &mut TestingWorld, expected: String) {
    let bytes = world.last_bytes.expect("no uuid_for called");
    assert_eq!(hex::encode(bytes), expected);
}

#[then("the bytes, string, and object all encode the same UUID")]
async fn then_all_agree(world: &mut TestingWorld) {
    let bytes = world.last_bytes.expect("no bytes");
    let s = world.last_str.as_ref().expect("no str");
    let obj = world.last_obj.expect("no obj");
    assert_eq!(obj.as_bytes(), &bytes);
    assert_eq!(obj.to_string(), *s);
}

#[then(expr = "the cover's domain is {string}")]
async fn then_cover_domain(world: &mut TestingWorld, expected: String) {
    let cover = world.cover.as_ref().expect("no cover");
    assert_eq!(cover.domain, expected);
}

#[then(expr = "the cover's correlation_id is {string}")]
async fn then_cover_correlation_id(world: &mut TestingWorld, expected: String) {
    let cover = world.cover.as_ref().expect("no cover");
    assert_eq!(cover.correlation_id, expected);
}

#[then("the cover's root bytes match the derived root")]
async fn then_cover_root_matches(world: &mut TestingWorld) {
    let cover = world.cover.as_ref().expect("no cover");
    let root = world.derived_root.expect("no derived root");
    assert_eq!(
        cover.root.as_ref().expect("no root").value,
        root.to_vec()
    );
}

#[then(expr = "the resulting event book has next_sequence equal to {int}")]
async fn then_event_book_next_sequence(world: &mut TestingWorld, expected: u32) {
    let book = world.event_book.as_ref().expect("no event book");
    assert_eq!(book.next_sequence, expected);
}

#[then("the context's domain is empty")]
async fn then_context_domain_empty(world: &mut TestingWorld) {
    assert!(world.context.as_ref().expect("no context").domain.is_empty());
}

#[then("the context's root is empty")]
async fn then_context_root_empty(world: &mut TestingWorld) {
    assert_eq!(
        world.context.as_ref().expect("no context").root,
        [0u8; 16]
    );
}

#[then("the context's events list is empty")]
async fn then_context_events_empty(world: &mut TestingWorld) {
    assert!(world.context.as_ref().expect("no context").events.is_empty());
}
