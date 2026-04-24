//! Step definitions for features/client/identity.feature.

use angzarr_client::{
    cart_root, compute_root, customer_root, fulfillment_root, inventory_product_root,
    inventory_root, order_root, product_root, to_proto_bytes, INVENTORY_PRODUCT_NAMESPACE,
};
use cucumber::{given, then, when, World};
use uuid::Uuid;

#[derive(Debug, Default, World)]
pub struct IdentityWorld {
    calls: Vec<Uuid>,
    proto_bytes: Option<[u8; 16]>,
}

// --- compute_root direct calls --------------------------------------------

#[when(expr = "I call compute_root with domain {string} and key {string}")]
async fn when_compute_root(world: &mut IdentityWorld, domain: String, key: String) {
    world.calls.push(compute_root(&domain, &key));
}

#[when(expr = "I call compute_root with domain {string} and key {string} a second time")]
async fn when_compute_root_second(world: &mut IdentityWorld, domain: String, key: String) {
    world.calls.push(compute_root(&domain, &key));
}

// --- per-domain helper dispatch -------------------------------------------

#[when(expr = "I call {string} with {string}")]
async fn when_call_helper(world: &mut IdentityWorld, helper: String, input: String) {
    let id = match helper.as_str() {
        "customer_root" => customer_root(&input),
        "product_root" => product_root(&input),
        "order_root" => order_root(&input),
        "inventory_root" => inventory_root(&input),
        "cart_root" => cart_root(&input),
        "fulfillment_root" => fulfillment_root(&input),
        "inventory_product_root" => inventory_product_root(&input),
        other => panic!("unknown helper: {}", other),
    };
    world.calls.push(id);
}

#[when(expr = "I call inventory_product_root with {string}")]
async fn when_inventory_product_root(world: &mut IdentityWorld, input: String) {
    world.calls.push(inventory_product_root(&input));
}

#[when(expr = "I call customer_root with {string}")]
async fn when_customer_root(world: &mut IdentityWorld, input: String) {
    world.calls.push(customer_root(&input));
}

#[when("I pass the resulting UUID through to_proto_bytes")]
async fn when_pass_through_to_proto_bytes(world: &mut IdentityWorld) {
    let id = world.calls.last().copied().expect("no prior UUID");
    world.proto_bytes = Some(to_proto_bytes(id));
}

// --- assertions ------------------------------------------------------------

#[then("both calls return the same UUID")]
async fn then_same_uuid(world: &mut IdentityWorld) {
    assert!(world.calls.len() >= 2);
    let first = world.calls[world.calls.len() - 2];
    let second = world.calls[world.calls.len() - 1];
    assert_eq!(first, second);
}

#[then("the two UUIDs differ")]
async fn then_uuids_differ(world: &mut IdentityWorld) {
    assert!(world.calls.len() >= 2);
    let first = world.calls[world.calls.len() - 2];
    let second = world.calls[world.calls.len() - 1];
    assert_ne!(first, second);
}

#[then(expr = "the resulting UUID equals {string}")]
async fn then_uuid_equals(world: &mut IdentityWorld, expected: String) {
    let last = world.calls.last().expect("no UUID computed");
    assert_eq!(last.to_string(), expected);
}

#[then(expr = "INVENTORY_PRODUCT_NAMESPACE equals the UUID {string}")]
async fn then_namespace_equals(_world: &mut IdentityWorld, expected: String) {
    assert_eq!(INVENTORY_PRODUCT_NAMESPACE.to_string(), expected);
}

#[then(expr = "the byte length is {int}")]
async fn then_byte_length(world: &mut IdentityWorld, expected: usize) {
    let bytes = world.proto_bytes.expect("no bytes captured");
    assert_eq!(bytes.len(), expected);
}

#[then(expr = "the bytes match the hex {string}")]
async fn then_bytes_match_hex(world: &mut IdentityWorld, expected: String) {
    let bytes = world.proto_bytes.expect("no bytes captured");
    assert_eq!(hex::encode(bytes), expected);
}
