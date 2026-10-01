//! Step defs for parity/client/wire_parity.feature.
//!
//! Cross-language wire-format parity check — verifies that the Rust client
//! produces byte-identical proto-encoded output as the Python client for
//! the same logical input. The Python sibling at
//! `client-python/main/tests/client/steps/test_wire_parity.py` exercises
//! the same scenarios against the same expected SHA-256 values.

use cucumber::{given, then, when, World};
use prost::Message;
use prost_types::Any as ProtoAny;
use sha2::{Digest, Sha256};

use angzarr_client::proto::{
    command_page::Payload as CmdPayload, CommandBook, CommandPage, Cover, Uuid as ProtoUuid,
};
use angzarr_client::router::Destinations;

#[derive(Debug, Default, World)]
#[world(init = Self::default)]
pub struct WireParityWorld {
    source: Option<Cover>,
    book: Option<CommandBook>,
    output_domains: Vec<String>,
}

#[given(
    expr = "a source cover with domain {string}, root bytes {word} and correlation_id {string}"
)]
async fn given_source_cover(
    world: &mut WireParityWorld,
    domain: String,
    root_hex: String,
    correlation_id: String,
) {
    world.source = Some(Cover {
        domain,
        root: Some(ProtoUuid {
            value: parse_hex_range(&root_hex),
        }),
        correlation_id,
        ..Default::default()
    });
}

#[given(
    expr = "a CommandBook with cover.domain {string}, cover.root bytes {word} and correlation_id {string}"
)]
async fn given_commandbook(
    world: &mut WireParityWorld,
    domain: String,
    root_hex: String,
    correlation_id: String,
) {
    let root_bytes = parse_hex_range(&root_hex);
    world.book = Some(CommandBook {
        cover: Some(Cover {
            domain,
            root: Some(ProtoUuid { value: root_bytes }),
            correlation_id,
            ..Default::default()
        }),
        pages: Vec::new(),
    });
}

#[given(expr = "a single CommandPage with command type_url {string} and payload bytes {word}")]
async fn given_single_page(world: &mut WireParityWorld, type_url: String, payload_hex: String) {
    let payload = parse_hex_bytes(&payload_hex);
    let book = world.book.as_mut().expect("CommandBook must be set first");
    book.pages.push(CommandPage {
        header: None,
        merge_strategy: 0,
        payload: Some(CmdPayload::Command(ProtoAny {
            type_url,
            value: payload,
        })),
    });
}

#[given(expr = "a Destinations for a component declaring output domain {string}")]
async fn given_destinations(world: &mut WireParityWorld, domain: String) {
    world.output_domains.push(domain);
}

#[when(
    expr = "I stamp the command for domain {string} from source event sequence {int} at command index {int}"
)]
async fn when_stamp(world: &mut WireParityWorld, domain: String, source_seq: u32, index: u32) {
    let dest = Destinations::new(world.output_domains.clone());
    let source = world.source.clone().expect("source cover must be set");
    let book = world.book.as_mut().expect("CommandBook must be set");
    dest.stamp_command(book, &domain, &source, source_seq, index)
        .expect("stamp must succeed");
}

#[then(expr = "the deterministically-encoded CommandBook hashes to SHA-256 {string}")]
async fn then_hash(world: &mut WireParityWorld, expected: String) {
    let book = world.book.as_ref().expect("CommandBook must be set");
    let raw = book.encode_to_vec();
    let actual = format!("{:x}", Sha256::digest(&raw));
    assert_eq!(
        actual, expected,
        "Wire-format drift from expected golden.\n  actual:   {}\n  expected: {}",
        actual, expected
    );
}

/// Parse a hex string of the form `00..0f` (range) into the byte sequence
/// `[0x00, 0x01, ..., 0x0f]`. Inclusive bounds.
fn parse_hex_range(s: &str) -> Vec<u8> {
    let parts: Vec<&str> = s.split("..").collect();
    assert_eq!(
        parts.len(),
        2,
        "expected hex range form 'NN..MM', got {:?}",
        s
    );
    let lo = u8::from_str_radix(parts[0], 16).expect("invalid lo");
    let hi = u8::from_str_radix(parts[1], 16).expect("invalid hi");
    (lo..=hi).collect()
}

/// Parse a contiguous hex string like `01020304` into bytes.
fn parse_hex_bytes(s: &str) -> Vec<u8> {
    assert!(
        s.len().is_multiple_of(2),
        "hex string must be even length: {:?}",
        s
    );
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("invalid hex"))
        .collect()
}
