//! Parsed storage experiment, compiled only for tests. Never selected by the server.
use super::*;
use crate::offline::{self, Result};
use hickory_proto::{
    op::ResponseCode,
    rr::{
        Name, RData, Record, RecordType,
        rdata::{A, SOA},
    },
};
use hickory_server::net::xfer::Protocol;
use std::{hint::black_box, mem::size_of};

struct ParsedEntry {
    message: Message,
    inserted: Instant,
    lifetime: Duration,
    weight: u32,
}
struct ParsedExpiry;
impl Expiry<CacheKey, Arc<ParsedEntry>> for ParsedExpiry {
    fn expire_after_create(
        &self,
        _: &CacheKey,
        entry: &Arc<ParsedEntry>,
        _: Instant,
    ) -> Option<Duration> {
        Some(entry.lifetime.saturating_sub(entry.inserted.elapsed()))
    }
    fn expire_after_update(
        &self,
        key: &CacheKey,
        entry: &Arc<ParsedEntry>,
        now: Instant,
        _: Option<Duration>,
    ) -> Option<Duration> {
        self.expire_after_create(key, entry, now)
    }
}

fn parsed_entry(key: &CacheKey, response: &Message, minimum: u64) -> Option<ParsedEntry> {
    let inserted = Instant::now();
    let (message, lifetime) = policy::prepare(response, &key.query, Duration::from_secs(86400))?;
    let bytes = message.to_vec().ok()?;
    if Header::read(&mut BinDecoder::new(&bytes))
        .ok()?
        .metadata
        .truncation
    {
        return None;
    }
    // Conservative fixture-specific estimate, NOT a general-purpose Message heap estimator.
    // Include vector capacities, names, SOA heap data and table/key overhead.
    let mut heap = message.queries.capacity() * size_of::<Query>() + 2 * key.query.name().len();
    for section in [&message.answers, &message.authorities, &message.additionals] {
        heap += section.capacity() * size_of::<hickory_proto::rr::Record>();
        for record in section {
            heap += 2 * record.name.len();
            match &record.data {
                RData::A(_) => {}
                RData::SOA(soa) => {
                    heap += size_of::<SOA>() + 2 * (soa.mname.len() + soa.rname.len())
                }
                _ => return None, // The experimental estimator supports only these fixtures.
            }
        }
    }
    let weight = (size_of::<ParsedEntry>() + heap + 512) as u64;
    Some(ParsedEntry {
        message,
        inserted,
        lifetime,
        weight: u32::try_from(weight.max(minimum)).ok()?,
    })
}

fn parsed_cache(capacity: u64) -> Cache<CacheKey, Arc<ParsedEntry>> {
    Cache::builder()
        .max_capacity(capacity)
        .weigher(|_: &CacheKey, entry: &Arc<ParsedEntry>| entry.weight)
        .expire_after(ParsedExpiry)
        .build()
}
async fn parsed_get(cache: &Cache<CacheKey, Arc<ParsedEntry>>, key: &CacheKey) -> Option<Message> {
    let entry = cache.get(key).await?;
    let age = entry.inserted.elapsed();
    if age >= entry.lifetime {
        return None;
    }
    let seconds = u32::try_from(age.as_secs() + u64::from(age.subsec_nanos() != 0)).ok()?;
    let mut message = entry.message.clone();
    for record in message
        .answers
        .iter_mut()
        .chain(&mut message.authorities)
        .chain(&mut message.additionals)
    {
        record.ttl = record.ttl.saturating_sub(seconds);
    }
    Some(message)
}

fn fixture(index: usize, count: usize, negative: bool) -> Result<(CacheKey, Message)> {
    let query = Query::query(
        Name::from_ascii(format!("entry{index}.example."))?,
        RecordType::A,
    );
    let mut request = Message::query();
    request.queries.push(query.clone());
    let request = Request::from_bytes(request.to_vec()?, "127.0.0.1:5300".parse()?, Protocol::Udp)?;
    let key = CacheKey::new(&query, &request);
    let mut message = Message::response(0, request.metadata.op_code);
    message.queries.push(query);
    if negative {
        message.metadata.response_code = ResponseCode::NXDomain;
        message.authorities.push(Record::from_rdata(
            Name::from_ascii("example.")?,
            60,
            RData::SOA(SOA::new(
                Name::from_ascii("ns.example.")?,
                Name::from_ascii("hostmaster.example.")?,
                1,
                60,
                60,
                60,
                30,
            )),
        ));
    } else {
        message.answers = (0..count)
            .map(|_| {
                Record::from_rdata(key.query.name().clone(), 60, RData::A(A::new(192, 0, 2, 1)))
            })
            .collect();
    }
    Ok((key, message))
}

#[tokio::test]
async fn parsed_candidate_ages_expires_and_does_not_share_mutations() -> Result {
    for negative in [false, true] {
        let (key, message) = fixture(1, 1, negative)?;
        let cache = parsed_cache(8 * 1024 * 1024);
        let mut entry = parsed_entry(&key, &message, 2048).unwrap();
        entry.inserted -= Duration::from_millis(1100);
        let original = entry.message.clone();
        cache.insert(key.clone(), Arc::new(entry)).await;
        let mut first = parsed_get(&cache, &key).await.unwrap();
        first.metadata.id = 65535;
        first.answers.clear();
        let second = parsed_get(&cache, &key).await.unwrap();
        assert_eq!(second.metadata.id, 0);
        for (before, after) in original.all_sections().zip(second.all_sections()) {
            assert_eq!(after.ttl, before.ttl - 2);
        }
        assert_eq!(second.answers.len(), original.answers.len());
        let mut expired = parsed_entry(&key, &message, 2048).unwrap();
        expired.inserted -= expired.lifetime + Duration::from_secs(1);
        cache.insert(key.clone(), Arc::new(expired)).await;
        assert!(parsed_get(&cache, &key).await.is_none());
    }
    Ok(())
}

#[test]
#[ignore = "Linux release cache A/B; synthetic fixtures, no DNS sockets"]
fn cache_storage() -> Result {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    let selected = std::env::var("LIGHT_DNS_CACHE_VARIANT").unwrap_or_default();
    if !matches!(selected.as_str(), "" | "current" | "parsed") {
        return Err(std::io::Error::other("expected current or parsed").into());
    }
    let mut csv = offline::HEADER.to_owned();
    let mut capacity_csv = "case,variant,entries,weighted_bytes,wire_bytes_per_entry,estimated_entry_weight,rss_before_fill_kib,rss_after_fill_kib,process_hwm_kib\n".to_owned();
    for (case, count, negative) in [
        ("small", 1, false),
        ("negative", 0, true),
        ("large", 100, false),
    ] {
        let rss_before = offline::usage()?.1;
        let config = CacheConfig::default();
        let current = ResponseCache::new(config.clone());
        let parsed = parsed_cache(config.max_memory_bytes);
        let (key, message) = fixture(0, count, negative)?;
        runtime.block_on(async {
            // Separate-process selection is necessary for interpretable RSS comparisons.
            for index in 0..4096 {
                let (key, message) = fixture(index, count, negative).unwrap();
                if selected != "parsed" {
                    current.insert(key.clone(), &message).await;
                }
                if selected != "current" {
                    let entry = parsed_entry(&key, &message, current.minimum_weight).unwrap();
                    parsed.insert(key, Arc::new(entry)).await;
                }
            }
            if let Some(cache) = &current.entries {
                cache.run_pending_tasks().await;
            }
            parsed.run_pending_tasks().await;
            if selected != "parsed" {
                current.insert(key.clone(), &message).await;
            }
            if selected != "current" {
                parsed
                    .insert(
                        key.clone(),
                        Arc::new(parsed_entry(&key, &message, current.minimum_weight).unwrap()),
                    )
                    .await;
            }
        });
        let usage = offline::usage()?;
        let wire_size = message.to_vec()?.len();
        for candidate in [false, true] {
            let variant = if candidate { "parsed" } else { "current" };
            if !selected.is_empty() && selected != variant {
                continue;
            }
            let (entries, weight, entry_weight) = if candidate {
                (
                    parsed.entry_count(),
                    parsed.weighted_size(),
                    parsed_entry(&key, &message, current.minimum_weight)
                        .unwrap()
                        .weight,
                )
            } else {
                let stats = runtime.block_on(current.stats());
                let weight = runtime
                    .block_on(current.entries.as_ref().unwrap().get(&key))
                    .unwrap()
                    .weight;
                (stats.entries, stats.estimated_bytes, weight)
            };
            use std::fmt::Write;
            writeln!(
                capacity_csv,
                "{case},{variant},{entries},{weight},{wire_size},{entry_weight},{rss_before},{},{}",
                usage.1, usage.2
            )?;
        }
        for trial in 0..5 {
            for candidate in if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let variant = if candidate { "parsed" } else { "current" };
                if !selected.is_empty() && selected != variant {
                    continue;
                }
                for clients in [1, 32, 256] {
                    offline::measure(
                        &mut csv,
                        &format!("{case}_read_{clients}"),
                        variant,
                        trial,
                        || {
                            runtime.block_on(async {
                                if clients == 1 {
                                    black_box(if candidate {
                                        parsed_get(&parsed, &key).await
                                    } else {
                                        current.get(&key).await
                                    })
                                    .unwrap();
                                } else {
                                    let mut tasks = tokio::task::JoinSet::new();
                                    for _ in 0..clients {
                                        let current = current.clone();
                                        let parsed = parsed.clone();
                                        let key = key.clone();
                                        tasks.spawn(async move {
                                            if candidate {
                                                parsed_get(&parsed, &key).await
                                            } else {
                                                current.get(&key).await
                                            }
                                        });
                                    }
                                    while let Some(result) = tasks.join_next().await {
                                        black_box(result.unwrap()).unwrap();
                                    }
                                }
                            })
                        },
                    )?;
                }
                offline::measure(&mut csv, &format!("{case}_insert"), variant, trial, || {
                    runtime.block_on(async {
                        if candidate {
                            parsed
                                .insert(
                                    key.clone(),
                                    Arc::new(
                                        parsed_entry(&key, &message, current.minimum_weight)
                                            .unwrap(),
                                    ),
                                )
                                .await;
                        } else {
                            current.insert(key.clone(), &message).await;
                        }
                    })
                })?;
            }
        }
    }
    offline::output(
        &format!(
            "cache-capacity-{}",
            if selected.is_empty() {
                "paired"
            } else {
                &selected
            }
        ),
        &capacity_csv,
    )?;
    offline::output(
        &format!(
            "cache-{}",
            if selected.is_empty() {
                "paired"
            } else {
                &selected
            }
        ),
        &csv,
    )
}
