//! Régressions du cache de recherche, indépendantes de l'allocateur système.
use super::*;
use std::{
    fmt::Write,
    sync::{Arc, Barrier},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn lists(count: usize) -> Result<Vec<FilterList>> {
    let mut text = String::new();
    for id in 0..count {
        writeln!(text, "||ad{id}*.example^$dnstype=AAAA")?;
    }
    Ok(vec![FilterList {
        text,
        allow: false,
        allow_dnsrewrite: true,
    }])
}

#[test]
fn pikevm_cache_is_bounded_and_preserves_all_pattern_ids() -> Result {
    // Disable both DFA engines so every search exercises the otherwise lazy fallback.
    let index = FilterIndex::compile_with_config(
        &lists(1000)?,
        &[],
        FilterIndex::mask_config().dfa(false).hybrid(false),
    )?;
    let regex = index.masks.as_ref().ok_or("missing masks")?;
    assert_eq!(regex.pattern_len(), 2000);
    let mut cache = regex.create_cache();
    let mut hits = PatternSet::new(regex.pattern_len());
    for id in 0..1000 {
        for prefix in ["", "child."] {
            let text = format!("{prefix}ad{id}x.example");
            hits.clear();
            regex.which_overlapping_matches_with(&mut cache, &Input::new(&text), &mut hits);
            // ad1*, ad10*, ad100* can all match: never accept only the first hit.
            let expected: Vec<_> = index
                .mask_rules
                .iter()
                .enumerate()
                .filter(|(pattern, rule)| {
                    let Selector::Mask(mask) = &index.advanced[**rule].selector else {
                        return false;
                    };
                    let stem = mask.trim_start_matches("||").trim_end_matches("*.example");
                    format!("ad{id}").starts_with(stem)
                        && (*pattern % 2 == usize::from(!prefix.is_empty()))
                })
                .map(|(pattern, _)| pattern)
                .collect();
            assert_eq!(
                hits.iter().map(|p| p.as_usize()).collect::<Vec<_>>(),
                expected
            );
            assert!(
                cache.memory_usage() < 16 * 1024 * 1024,
                "cache grew to {} bytes",
                cache.memory_usage()
            );
        }
    }
    let warmed = cache.memory_usage();
    for id in 0..10_000 {
        hits.clear();
        regex.which_overlapping_matches_with(
            &mut cache,
            &Input::new(&format!("unique{id}.example")),
            &mut hits,
        );
        assert!(hits.is_empty());
    }
    assert_eq!(cache.memory_usage(), warmed);
    Ok(())
}

#[test]
fn masks_keep_priorities_conditions_rewrites_and_cnames_without_captures() -> Result {
    let text = "simple.example\n||ad*.example^\n@@||adsafe*.example^\n||adsafeimportant*.example^$important\n@@||adsafeimportantallow*.example^$important\n||disabled*.example^\n||disabled*.example^$badfilter\n||typed*.example^$dnstype=AAAA,denyallow=typed-safe.example\n||rewrite*.example^$dnsrewrite=192.0.2.8\n@@||rewriteexcept*.example^$dnsrewrite\n||alias*.example^$dnsrewrite=simple.example\n";
    let lists = [FilterList {
        text: text.into(),
        allow: false,
        allow_dnsrewrite: true,
    }];
    for config in [
        FilterIndex::mask_config(),
        FilterIndex::mask_config().dfa(false).hybrid(false),
    ] {
        let index = FilterIndex::compile_with_config(
            &lists,
            &[Name::from_ascii("aduser.example.")?],
            config,
        )?;
        for (name, kind, expected) in [
            ("simple.example.", RecordType::A, "block"),
            ("ad1.example.", RecordType::A, "block"),
            ("child.ad1.example.", RecordType::A, "block"),
            ("adsafe.example.", RecordType::A, "allow"),
            ("adsafeimportant.example.", RecordType::A, "block"),
            ("adsafeimportantallow.example.", RecordType::A, "allow"),
            ("aduser.example.", RecordType::A, "allow"),
            ("disabled1.example.", RecordType::A, "pass"),
            ("typed1.example.", RecordType::A, "pass"),
            ("typed1.example.", RecordType::AAAA, "block"),
            ("typed-safe.example.", RecordType::AAAA, "pass"),
            ("rewrite1.example.", RecordType::A, "rewrite"),
            ("rewriteexcept1.example.", RecordType::A, "pass"),
            ("alias1.example.", RecordType::A, "alias"),
        ] {
            let actual = match index.decide(&Name::from_ascii(name)?, kind) {
                Decision::Pass => "pass",
                Decision::Allow => "allow",
                Decision::Block => "block",
                Decision::Rewrite { code, data } => {
                    assert_eq!(code, ResponseCode::NoError);
                    assert_eq!(data.len(), 1);
                    if matches!(data[0], RData::CNAME(_)) {
                        assert_eq!(data[0].to_string(), "simple.example.");
                        "alias"
                    } else {
                        assert_eq!(data[0].to_string(), "192.0.2.8");
                        "rewrite"
                    }
                }
            };
            assert_eq!(actual, expected, "{name} {kind}");
        }
        let name = Name::from_ascii("original.example.")?;
        let mut response = Message::query();
        response.answers.push(hickory_proto::rr::Record::from_rdata(
            name.clone(),
            60,
            RData::CNAME(hickory_proto::rr::rdata::CNAME(Name::from_ascii(
                "ad1.example.",
            )?)),
        ));
        assert_eq!(index.blocks_cname(&name, &response, false), Ok(true));
        assert_eq!(index.blocks_cname(&name, &response, true), Ok(false));
    }
    Ok(())
}

#[test]
fn concurrent_readers_release_replaced_generations() -> Result {
    let lists = lists(128)?;
    let build = || {
        FilterIndex::compile_with_config(
            &lists,
            &[],
            FilterIndex::mask_config().dfa(false).hybrid(false),
        )
    };
    let shared = Arc::new(arc_swap::ArcSwap::from_pointee(build()?));
    for _ in 0..5 {
        let old = Arc::downgrade(&shared.load_full());
        let barrier = Arc::new(Barrier::new(5));
        let mut readers = Vec::new();
        for worker in 0..4 {
            let snapshot = shared.load_full();
            let barrier = barrier.clone();
            readers.push(std::thread::spawn(move || -> Result {
                barrier.wait();
                for id in 0..100 {
                    let name = Name::from_ascii(format!("ad{worker}{id}x.example."))?;
                    assert!(matches!(
                        snapshot.decide(&name, RecordType::AAAA),
                        Decision::Block
                    ));
                }
                Ok(())
            }));
        }
        shared.store(Arc::new(build()?));
        assert!(
            old.upgrade().is_some(),
            "readers must retain the old generation"
        );
        barrier.wait();
        for reader in readers {
            reader.join().map_err(|_| "reader panicked")??;
        }
        assert!(
            old.upgrade().is_none(),
            "old index and its regex pool retained"
        );
    }
    Ok(())
}
