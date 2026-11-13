use super::CacheStats;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

/// Compteurs cumulés depuis le démarrage ; les lectures concurrentes sont indicatives.
#[derive(Debug, Serialize)]
pub struct ServerStats {
    pub uptime_secs: u64,
    pub requests: u64,
    pub local_answers: u64,
    pub blocked: u64,
    pub cache_hits: u64,
    pub forwarded: u64,
    pub errors: u64,
    pub upstream_failures: u64,
    pub upstream_skipped: u64,
    pub upstream_probes: u64,
    pub cname_blocked: u64,
    pub rewritten: u64,
    pub reloads: u64,
    pub cache_clears: u64,
    pub filter_rules: usize,
    pub cache_entries: u64,
    pub cache_estimated_bytes: u64,
}

#[derive(Default)]
pub(crate) struct Counters {
    pub requests: AtomicU64,
    pub local_answers: AtomicU64,
    pub blocked: AtomicU64,
    pub cache_hits: AtomicU64,
    pub forwarded: AtomicU64,
    pub errors: AtomicU64,
    pub upstream_failures: AtomicU64,
    pub upstream_skipped: AtomicU64,
    pub upstream_probes: AtomicU64,
    pub cname_blocked: AtomicU64,
    pub rewritten: AtomicU64,
    pub reloads: AtomicU64,
    pub cache_clears: AtomicU64,
}

impl Counters {
    pub fn increment(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(
        &self,
        uptime_secs: u64,
        filter_rules: usize,
        cache: CacheStats,
    ) -> ServerStats {
        ServerStats {
            uptime_secs,
            requests: self.requests.load(Ordering::Relaxed),
            local_answers: self.local_answers.load(Ordering::Relaxed),
            blocked: self.blocked.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            forwarded: self.forwarded.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            upstream_failures: self.upstream_failures.load(Ordering::Relaxed),
            upstream_skipped: self.upstream_skipped.load(Ordering::Relaxed),
            upstream_probes: self.upstream_probes.load(Ordering::Relaxed),
            cname_blocked: self.cname_blocked.load(Ordering::Relaxed),
            rewritten: self.rewritten.load(Ordering::Relaxed),
            reloads: self.reloads.load(Ordering::Relaxed),
            cache_clears: self.cache_clears.load(Ordering::Relaxed),
            filter_rules,
            cache_entries: cache.entries,
            cache_estimated_bytes: cache.estimated_bytes,
        }
    }
}

/// État indicatif de la génération active ; compteurs cumulés depuis le démarrage.
#[derive(Debug, Serialize)]
pub struct CacheStatus {
    pub enabled: bool,
    pub entries: u64,
    pub max_entries: u64,
    pub estimated_bytes: u64,
    pub max_memory_bytes: u64,
    pub max_ttl_secs: u64,
    pub hits: u64,
    pub clears: u64,
}
