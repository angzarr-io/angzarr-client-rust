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
