//! Baselines and isolated allocation experiments; no global scratch buffer.
use super::*;
use crate::offline::{self, Result};
use std::{fmt::Write, hint::black_box};

#[test]
fn inplace_ascii_conversion_is_equivalent() -> Result {
    for text in ["Mixed.Example.", "long-name.Mixed.Example."] {
        let name = Name::from_ascii(text)?;
        let mut candidate = name.to_ascii();
        candidate.make_ascii_lowercase();
        assert_eq!(candidate, name.to_ascii().to_ascii_lowercase());
    }
    Ok(())
}

#[test]
#[ignore = "Linux release filtering baselines and isolated scratch experiments"]
fn filtering_costs() -> Result {
    let mut text = String::new();
    for id in 0..100_000 {
        writeln!(text, "block{id}.example")?;
    }
    let simple_index = FilterIndex::compile(
        &[FilterList {
            text: text.clone(),
            allow: false,
            allow_dnsrewrite: false,
        }],
        &[],
    )?;
    text.push_str("@@||safe.example^\n||typed.example^$dnstype=AAAA\n||rewrite.example^$dnsrewrite=192.0.2.8\n");
    for id in 0..1000 {
        writeln!(text, "||ad{id}*.example^$dnstype=AAAA")?;
    }
    let index = FilterIndex::compile(
        &[FilterList {
            text,
            allow: false,
            allow_dnsrewrite: true,
        }],
        &[],
    )?;
    let mut csv = offline::HEADER.to_owned();
    for (case, text, kind) in [
        ("absent", "clean.example.", RecordType::A),
        ("simple", "block50000.example.", RecordType::A),
        ("pattern", "ad500x.example.", RecordType::AAAA),
        ("exception", "safe.example.", RecordType::A),
        ("dnstype", "typed.example.", RecordType::AAAA),
        ("rewrite", "rewrite.example.", RecordType::A),
    ] {
        let name = Name::from_ascii(text)?;
        match case {
            "simple" | "pattern" | "dnstype" => {
                assert!(matches!(index.decide(&name, kind), Decision::Block))
            }
            "exception" => assert!(matches!(index.decide(&name, kind), Decision::Allow)),
            "rewrite" => assert!(matches!(
                index.decide(&name, kind),
                Decision::Rewrite { .. }
            )),
            _ => assert!(matches!(index.decide(&name, kind), Decision::Pass)),
        }
        for trial in 0..5 {
            offline::measure(&mut csv, case, "current", trial, || {
                index.decide(black_box(&name), kind)
            })?;
        }
    }
    for text in [
        "clean.example.",
        "block50000.example.",
        "deep.child.block50000.example.",
    ] {
        let name = Name::from_ascii(text)?;
        assert_eq!(
            simple_lookup(&simple_index, &name, false),
            simple_lookup(&simple_index, &name, true)
        );
        for trial in 0..5 {
            for candidate in if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                offline::measure(
                    &mut csv,
                    &format!("simple_ancestors_{text}"),
                    if candidate { "borrow_first" } else { "current" },
                    trial,
                    || simple_lookup(&simple_index, black_box(&name), candidate),
                )?;
            }
        }
    }
    // Same candidate identifiers and stable source order. Vec::collect may already
    // recycle its input allocation; measure rather than assume a second allocation.
    for count in [0, 1, 8, 128] {
        let ids: Vec<_> = (0..count)
            .chain(0..count)
            .map(|id| id % index.advanced.len())
            .collect();
        let name = Name::from_ascii("ad500x.example.")?;
        for kind in [RecordType::A, RecordType::AAAA] {
            let expected: Vec<_> = sorted_ids(ids.clone())
                .into_iter()
                .map(|id| &index.advanced[id])
                .filter(|rule| rule.applies(&name, kind))
                .collect();
            let actual = retained_ids(&index, &name, kind, ids.clone());
            assert!(
                expected
                    == actual
                        .iter()
                        .map(|id| &index.advanced[*id])
                        .collect::<Vec<_>>()
            );
        }
        for trial in 0..5 {
            for candidate in if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                offline::measure(
                    &mut csv,
                    &format!("matched_ids_{count}"),
                    if candidate {
                        "retain_ids"
                    } else {
                        "collect_refs"
                    },
                    trial,
                    || {
                        if candidate {
                            black_box(retained_ids(
                                &index,
                                &name,
                                RecordType::AAAA,
                                black_box(&ids).clone(),
                            ))
                            .len()
                        } else {
                            let matched: Vec<_> = sorted_ids(black_box(&ids).clone())
                                .into_iter()
                                .map(|id| &index.advanced[id])
                                .filter(|rule| rule.applies(&name, RecordType::AAAA))
                                .collect();
                            black_box(matched).len()
                        }
                    },
                )?;
            }
        }
    }
    let name = Name::from_ascii("Sub.Mixed.Example.")?;
    for trial in 0..5 {
        for candidate in if trial % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            offline::measure(
                &mut csv,
                "ascii",
                if candidate { "inplace" } else { "current" },
                trial,
                || {
                    if candidate {
                        let mut text = black_box(&name).to_ascii();
                        text.make_ascii_lowercase();
                        text
                    } else {
                        black_box(&name).to_ascii().to_ascii_lowercase()
                    }
                },
            )?;
        }
    }
    let masks = index.masks.as_ref().unwrap();
    let input = Input::new(b"ad500x.example");
    let mut scratch = PatternSet::new(masks.pattern_len());
    for trial in 0..5 {
        for candidate in if trial % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            offline::measure(
                &mut csv,
                "pattern_set",
                if candidate {
                    "local_scratch"
                } else {
                    "current"
                },
                trial,
                || {
                    if candidate {
                        scratch.clear();
                        masks.which_overlapping_matches(&input, &mut scratch);
                        scratch.len()
                    } else {
                        let mut hits = PatternSet::new(masks.pattern_len());
                        masks.which_overlapping_matches(&input, &mut hits);
                        hits.len()
                    }
                },
            )?;
        }
    }
    let name = Name::from_ascii("alias.example.")?;
    let mut response = Message::query();
    response.answers.push(hickory_proto::rr::Record::from_rdata(
        name.clone(),
        60,
        RData::CNAME(hickory_proto::rr::rdata::CNAME(Name::from_ascii(
            "block50000.example.",
        )?)),
    ));
    assert_eq!(index.blocks_cname(&name, &response, false), Ok(true));
    for trial in 0..5 {
        offline::measure(&mut csv, "cname", "current", trial, || {
            index.blocks_cname(&name, &response, false)
        })?;
    }
    offline::output("filtering-offline", &csv)
}

// Isolate only the simple ancestor walk. Advanced rules never use this candidate.
fn simple_lookup(index: &FilterIndex, name: &Name, candidate: bool) -> Option<Action> {
    let mut strongest = if candidate {
        index.rules.get(name).copied()
    } else {
        None
    };
    if strongest == Some(Action::UserAllow) {
        return strongest;
    }
    let mut ancestor = if candidate {
        name.base_name()
    } else {
        name.clone()
    };
    while !ancestor.is_root() {
        strongest = strongest.max(index.rules.get(&ancestor).copied());
        if strongest == Some(Action::UserAllow) {
            return strongest;
        }
        ancestor = ancestor.base_name();
    }
    strongest
}

fn sorted_ids(mut ids: Vec<usize>) -> Vec<usize> {
    ids.sort_unstable();
    ids.dedup();
    ids
}
fn retained_ids(index: &FilterIndex, name: &Name, kind: RecordType, ids: Vec<usize>) -> Vec<usize> {
    let mut ids = sorted_ids(ids);
    ids.retain(|id| index.advanced[*id].applies(name, kind));
    ids
}
