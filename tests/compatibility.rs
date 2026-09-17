//! Checked-in ranking coverage. Live response availability is not conformance.
#![cfg(feature = "net")]

use olive_html::net::Location;
use std::collections::{BTreeMap, BTreeSet};

fn rankings() -> Vec<(&'static str, u8, &'static str)> {
    include_str!("fixtures/compatibility/sites.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.starts_with("region"))
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 3);
            (fields[0], fields[1].parse().unwrap(), fields[2])
        })
        .collect()
}

#[test]
fn ranking_snapshot_has_fifty_contiguous_unique_targets_per_region() {
    let mut regions: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for (region, rank, domain) in rankings() {
        regions.entry(region).or_default().push((rank, domain));
    }
    assert_eq!(
        regions.keys().copied().collect::<Vec<_>>(),
        ["global", "il"]
    );
    for sites in regions.values() {
        assert_eq!(sites.len(), 50);
        assert_eq!(
            sites.iter().map(|(r, _)| *r).collect::<Vec<_>>(),
            (1..=50).collect::<Vec<_>>()
        );
        assert_eq!(
            sites.iter().map(|(_, d)| d).collect::<BTreeSet<_>>().len(),
            50
        );
    }
    assert_eq!(
        rankings()
            .iter()
            .map(|(_, _, d)| d)
            .collect::<BTreeSet<_>>()
            .len(),
        77
    );
}

#[test]
fn every_target_is_a_safe_https_address() {
    for (_, _, domain) in rankings() {
        let location = Location::from_input(&format!("https://{domain}/")).unwrap();
        assert!(location.is_remote());
        assert_eq!(location.url().scheme(), "https");
        assert_eq!(location.url().host_str(), Some(domain));
        assert_eq!(location.url().path(), "/");
    }
}

// Use scripts/run-compatibility.sh for the opt-in live audit. It records every
// unique target, including HTTP blocks and transport failures, without stopping
// at the first third-party outage or mistaking a challenge page for support.
