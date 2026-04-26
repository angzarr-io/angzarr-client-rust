//! Step defs for features/client/destinations.feature.
//!
//! Pins the canonical query surface on Destinations across languages:
//! `has_domain(domain) -> bool` and `domains() -> impl Iterator<Item =
//! &str>`. The Python sibling at
//! `client-python/main/tests/client/steps/test_destinations.py`
//! exercises the same scenarios against the same canonical names.

use std::collections::HashMap;

use cucumber::{given, then, World};

use angzarr_client::router::Destinations;

#[derive(Debug, Default, World)]
#[world(init = Self::default)]
pub struct DestinationsWorld {
    destinations: Option<Destinations>,
}

#[given(regex = r"^a Destinations built from sequences mapping (.+)$")]
async fn given_destinations(world: &mut DestinationsWorld, spec: String) {
    let mut seqs: HashMap<String, u32> = HashMap::new();
    for part in spec.split(" and ") {
        let (name, seq) = part
            .split_once(" to ")
            .expect("expected `\"name\" to N` form");
        let name = name.trim().trim_matches('"').to_string();
        let seq: u32 = seq.trim().parse().expect("seq must parse as u32");
        seqs.insert(name, seq);
    }
    world.destinations = Some(Destinations::from_sequences(seqs));
}

#[given(regex = r"^a Destinations built from an ordered sequence list (.+)$")]
async fn given_destinations_ordered(world: &mut DestinationsWorld, spec: String) {
    // Build from an ordered Vec so the insertion sequence matches the
    // spec literally. Each named destination gets sequence 0; the
    // order is what's under test.
    let pairs: Vec<(String, u32)> = spec
        .split(" then ")
        .map(|s| (s.trim().trim_matches('"').to_string(), 0u32))
        .collect();
    world.destinations = Some(Destinations::from_sequences(pairs));
}

#[then(regex = r#"^has_domain "([^"]*)" returns (true|false)$"#)]
async fn then_has_domain(world: &mut DestinationsWorld, domain: String, expected: String) {
    let dest = world.destinations.as_ref().expect("Destinations must be set");
    let actual = dest.has_domain(&domain);
    let expected: bool = expected == "true";
    assert_eq!(
        actual, expected,
        "has_domain({:?}) = {}, expected {}",
        domain, actual, expected
    );
}

#[then(regex = r#"^domains contains "([^"]+)"$"#)]
async fn then_domains_contains(world: &mut DestinationsWorld, domain: String) {
    let dest = world.destinations.as_ref().expect("Destinations must be set");
    let found: Vec<&str> = dest.domains().collect();
    assert!(
        found.iter().any(|d| *d == domain.as_str()),
        "domain {:?} not in {:?}",
        domain,
        found
    );
}

#[then(regex = r"^domains has (\d+) entries$")]
async fn then_domains_count(world: &mut DestinationsWorld, count: usize) {
    let dest = world.destinations.as_ref().expect("Destinations must be set");
    let n = dest.domains().count();
    assert_eq!(n, count, "domains count = {}, expected {}", n, count);
}

#[then(regex = r"^domains in order are (.+)$")]
async fn then_domains_in_order(world: &mut DestinationsWorld, spec: String) {
    let dest = world.destinations.as_ref().expect("Destinations must be set");
    let expected: Vec<String> = spec
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect();
    let actual: Vec<String> = dest.domains().map(|s| s.to_string()).collect();
    assert_eq!(
        actual, expected,
        "insertion order drift: got {:?}, expected {:?}",
        actual, expected
    );
}
