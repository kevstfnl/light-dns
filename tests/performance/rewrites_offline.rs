//! Small lookup candidate; deliberately retains the existing table and DNS semantics.
use super::*;
use crate::offline::{self, Result};
use std::hint::black_box;

fn parent_lookup(rewrites: &Rewrites, name: &Name) -> Option<Lookup> {
    if rewrites.nodes.is_empty() {
        return None;
    }
    let node = rewrites.nodes.get(name);
    if let Some(address) = node.and_then(|node| node.address) {
        return Some(Lookup::Address(address));
    }
    let mut closest = node;
    if node.is_some_and(|node| node.managed_zone) {
        return Some(Lookup::NoData);
    }
    let mut candidate = name.base_name();
    while !candidate.is_root() {
        if let Some(ancestor) = rewrites.nodes.get(&candidate) {
            closest = closest.or(Some(ancestor));
            if ancestor.managed_zone {
                return Some(if node.is_some() {
                    Lookup::NoData
                } else if let Some(address) = closest.and_then(|node| node.wildcard) {
                    Lookup::Address(address)
                } else {
                    Lookup::NxDomain
                });
            }
        }
        candidate = candidate.base_name();
    }
    None
}
fn tag(lookup: Option<Lookup>) -> String {
    match lookup {
        Some(Lookup::Address(ip)) => ip.to_string(),
        Some(Lookup::NoData) => "nodata".into(),
        Some(Lookup::NxDomain) => "nxdomain".into(),
        None => "forward".into(),
    }
}
fn fixture() -> Result<Rewrites> {
    Ok(Rewrites::new(
        [
            ("*.home.arpa.", "192.0.2.1"),
            ("router.home.arpa.", "192.0.2.2"),
            ("v6.home.arpa.", "2001:db8::1"),
            ("deep.child.home.arpa.", "192.0.2.3"),
            ("*.inner.home.arpa.", "192.0.2.4"),
        ]
        .into_iter()
        .map(|(name, ip)| Ok((Name::from_ascii(name)?, ip.parse()?)))
        .collect::<Result<HashMap<_, _>>>()?,
    ))
}
#[test]
fn parent_candidate_keeps_closest_encloser() -> Result {
    let rewrites = fixture()?;
    for text in [
        "router.home.arpa.",
        "v6.home.arpa.",
        "home.arpa.",
        "host.home.arpa.",
        "x.router.home.arpa.",
        "child.home.arpa.",
        "x.child.home.arpa.",
        "deep.child.home.arpa.",
        "inner.home.arpa.",
        "x.inner.home.arpa.",
        "elsewhere.",
        ".",
    ] {
        let name = Name::from_ascii(text)?;
        assert_eq!(
            tag(rewrites.lookup(&name)),
            tag(parent_lookup(&rewrites, &name)),
            "{text}"
        );
    }
    Ok(())
}
#[test]
#[ignore = "Linux release rewrite lookup microbenchmark"]
fn rewrite_lookup() -> Result {
    let rewrites = fixture()?;
    let mut csv = offline::HEADER.to_owned();
    for text in [
        "router.home.arpa.",
        "host.home.arpa.",
        "x.child.home.arpa.",
        "elsewhere.",
    ] {
        let name = Name::from_ascii(text)?;
        for trial in 0..5 {
            for candidate in if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                offline::measure(
                    &mut csv,
                    text,
                    if candidate { "parent" } else { "current" },
                    trial,
                    || {
                        if candidate {
                            parent_lookup(&rewrites, black_box(&name))
                        } else {
                            rewrites.lookup(black_box(&name))
                        }
                    },
                )?;
            }
        }
    }
    offline::output("rewrites-offline", &csv)
}
