//! Release-candidate website compatibility coverage.
//!
//! The ranking snapshot is deliberately checked into the repository so the RC
//! suite does not change when a traffic provider updates its page. The live
//! smoke test is opt-in because third-party sites are rate-limited and may
//! change their markup without notice.

#![cfg(feature = "net")]

use olive_html::net::{DocumentLoader, Location};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Site {
    domain: &'static str,
    region_rank: u8,
    worldwide_rank: Option<u8>,
}

// Semrush, July 2026, all categories. The Israel and worldwide lists overlap
// on six domains, giving the suite fourteen unique targets.
const ISRAEL: [Site; 10] = [
    Site {
        domain: "google.com",
        region_rank: 1,
        worldwide_rank: Some(1),
    },
    Site {
        domain: "youtube.com",
        region_rank: 2,
        worldwide_rank: Some(2),
    },
    Site {
        domain: "ynet.co.il",
        region_rank: 3,
        worldwide_rank: None,
    },
    Site {
        domain: "facebook.com",
        region_rank: 4,
        worldwide_rank: Some(3),
    },
    Site {
        domain: "mako.co.il",
        region_rank: 5,
        worldwide_rank: None,
    },
    Site {
        domain: "walla.co.il",
        region_rank: 6,
        worldwide_rank: None,
    },
    Site {
        domain: "wikipedia.org",
        region_rank: 7,
        worldwide_rank: Some(7),
    },
    Site {
        domain: "instagram.com",
        region_rank: 8,
        worldwide_rank: Some(4),
    },
    Site {
        domain: "sport5.co.il",
        region_rank: 9,
        worldwide_rank: None,
    },
    Site {
        domain: "chatgpt.com",
        region_rank: 10,
        worldwide_rank: Some(5),
    },
];

const WORLDWIDE: [Site; 10] = [
    Site {
        domain: "google.com",
        region_rank: 0,
        worldwide_rank: Some(1),
    },
    Site {
        domain: "youtube.com",
        region_rank: 0,
        worldwide_rank: Some(2),
    },
    Site {
        domain: "facebook.com",
        region_rank: 0,
        worldwide_rank: Some(3),
    },
    Site {
        domain: "instagram.com",
        region_rank: 0,
        worldwide_rank: Some(4),
    },
    Site {
        domain: "chatgpt.com",
        region_rank: 0,
        worldwide_rank: Some(5),
    },
    Site {
        domain: "reddit.com",
        region_rank: 0,
        worldwide_rank: Some(6),
    },
    Site {
        domain: "wikipedia.org",
        region_rank: 0,
        worldwide_rank: Some(7),
    },
    Site {
        domain: "x.com",
        region_rank: 0,
        worldwide_rank: Some(8),
    },
    Site {
        domain: "pornhub.com",
        region_rank: 0,
        worldwide_rank: Some(9),
    },
    Site {
        domain: "whatsapp.com",
        region_rank: 0,
        worldwide_rank: Some(10),
    },
];

fn all_sites() -> impl Iterator<Item = Site> {
    ISRAEL.into_iter().chain(WORLDWIDE)
}

#[test]
fn ranking_snapshot_has_ten_per_region_and_expected_overlap() {
    assert_eq!(ISRAEL.len(), 10);
    assert_eq!(WORLDWIDE.len(), 10);
    assert!(
        ISRAEL
            .windows(2)
            .all(|pair| pair[0].region_rank < pair[1].region_rank)
    );
    assert!(
        WORLDWIDE
            .iter()
            .all(|site| site.worldwide_rank.is_some_and(|rank| rank > 0))
    );
    let unique = all_sites()
        .map(|site| site.domain)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), 14);
}

#[test]
fn every_target_is_a_safe_http_address() {
    for site in all_sites() {
        let location = Location::from_input(&format!("https://{}/", site.domain)).unwrap();
        assert!(location.is_remote());
        assert!(location.as_str().starts_with("https://"));
    }
}

/// Run with `OLIVE_LIVE_COMPAT=1 cargo test --all-features --test compatibility`.
/// Each response is parsed with scripts disabled, matching the default web
/// scripting boundary. A missing/blocked site is reported with its domain.
#[test]
fn live_top_sites_smoke_test() {
    if std::env::var_os("OLIVE_LIVE_COMPAT").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    let loader = DocumentLoader::new().expect("HTTP client");
    for site in all_sites() {
        let location = Location::from_input(&format!("https://{}/", site.domain)).unwrap();
        let loaded = loader
            .load(location)
            .unwrap_or_else(|error| panic!("{} failed to load: {}", site.domain, error));
        let parsed = loaded
            .parse(false)
            .unwrap_or_else(|error| panic!("{} failed to parse: {}", site.domain, error));
        assert!(
            parsed
                .document
                .descendants(parsed.document.root())
                .next()
                .is_some(),
            "{} returned an empty document",
            site.domain
        );
    }
}
