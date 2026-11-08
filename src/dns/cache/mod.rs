mod policy;

use crate::config::CacheConfig;
use hickory_proto::{
    op::{Header, Message, Query},
    serialize::binary::{BinDecodable, BinDecoder},
};
use hickory_server::server::Request;
use moka::{future::Cache, policy::Expiry};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Eq, Hash, PartialEq)]
pub(crate) struct CacheKey {
    query: Query,
    edns: bool,
    dnssec_ok: bool,
    checking_disabled: bool,
    authentic_data: bool,
}

impl CacheKey {
    pub(crate) fn new(query: &Query, request: &Request) -> Self {
        Self {
            query: query.clone(),
            edns: request.edns.is_some(),
            dnssec_ok: request
                .edns
                .as_ref()
                .is_some_and(|edns| edns.flags().dnssec_ok),
            checking_disabled: request.metadata.checking_disabled,
            authentic_data: request.metadata.authentic_data,
        }
    }
}

struct Entry {
    bytes: Box<[u8]>,
    inserted: Instant,
    lifetime: Duration,
    weight: u32,
}
struct TtlExpiry;

impl Expiry<CacheKey, Arc<Entry>> for TtlExpiry {
    fn expire_after_create(
        &self,
        _: &CacheKey,
        entry: &Arc<Entry>,
        _: Instant,
    ) -> Option<Duration> {
        Some(entry.lifetime.saturating_sub(entry.inserted.elapsed()))
    }
    fn expire_after_update(
        &self,
        key: &CacheKey,
        entry: &Arc<Entry>,
        now: Instant,
        _: Option<Duration>,
    ) -> Option<Duration> {
        self.expire_after_create(key, entry, now)
    }
}

#[derive(Clone)]
pub(crate) struct ResponseCache {
    entries: Option<Cache<CacheKey, Arc<Entry>>>,
    config: CacheConfig,
    minimum_weight: u64,
}

/// Taille pondérée indicative du cache, après sa maintenance interne.
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheStats {
    pub entries: u64,
    pub estimated_bytes: u64,
}

impl ResponseCache {
    pub(crate) fn empty(&self) -> Self {
        Self::new(self.config.clone())
    }

    pub(crate) fn new(config: CacheConfig) -> Self {
        let minimum_weight = config.max_memory_bytes.div_ceil(config.max_entries.max(1));
        let entries = (config.enabled
            && config.max_entries != 0
            && config.max_memory_bytes != 0
            && !config.max_ttl.is_zero())
        .then(|| {
            Cache::builder()
                .name("dns-responses")
                .max_capacity(config.max_memory_bytes)
                .weigher(|_: &CacheKey, entry: &Arc<Entry>| entry.weight)
                .expire_after(TtlExpiry)
                .build()
        });
        Self {
            entries,
            config,
            minimum_weight,
        }
    }

    pub(crate) async fn get(&self, key: &CacheKey) -> Option<Message> {
        let cache = self.entries.as_ref()?;
        let entry = cache.get(key).await?;
        let age = entry.inserted.elapsed();
        if age >= entry.lifetime {
            return None;
        }
        let mut message = Message::from_vec(&entry.bytes).ok()?;
        // Arrondir l'âge vers le haut évite de rallonger un TTL d'une fraction de seconde.
        let seconds = u32::try_from(age.as_secs() + u64::from(age.subsec_nanos() != 0)).ok()?;
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

    pub(crate) async fn insert(&self, key: CacheKey, response: &Message) {
        let Some(cache) = &self.entries else {
            return;
        };
        let inserted = Instant::now();
        let Some((message, lifetime)) = policy::prepare(response, &key.query, self.config.max_ttl)
        else {
            return;
        };
        let Ok(bytes) = message.to_vec() else {
            return;
        };
        // L'encodeur peut tronquer une réponse trop grande : ne jamais conserver ce résultat.
        if !matches!(Header::read(&mut BinDecoder::new(&bytes)), Ok(header) if !header.metadata.truncation)
        {
            return;
        }
        let weight =
            ((bytes.len() + 512 + 2 * key.query.name().len()) as u64).max(self.minimum_weight);
        if weight > self.config.max_memory_bytes {
            return;
        }
        let Ok(weight) = u32::try_from(weight) else {
            return;
        };
        cache
            .insert(
                key,
                Arc::new(Entry {
                    bytes: bytes.into_boxed_slice(),
                    inserted,
                    lifetime,
                    weight,
                }),
            )
            .await;
    }

    pub(crate) async fn stats(&self) -> CacheStats {
        if let Some(cache) = &self.entries {
            cache.run_pending_tasks().await;
            CacheStats {
                entries: cache.entry_count(),
                estimated_bytes: cache.weighted_size(),
            }
        } else {
            CacheStats::default()
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "../../../tests/performance/cache_offline.rs"]
mod offline;
