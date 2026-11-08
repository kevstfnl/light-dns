use super::stats::{Counters, ServerStats};
use crate::{
    config::Config,
    dns::{cache::ResponseCache, forwarder::Forwarder, rewrites::Rewrites},
    filtering::{self, SharedFilters},
};
use arc_swap::ArcSwap;
use std::{io, net::SocketAddr, sync::Arc, time::Instant};
use tokio::{sync::Mutex, task::JoinHandle};

pub(super) struct Resolver {
    pub forwarder: Forwarder,
    pub rewrites: Rewrites,
    pub filters: SharedFilters,
    pub cache: ArcSwap<ResponseCache>,
}

impl Resolver {
    async fn prepare(
        config: Config,
        counters: Arc<Counters>,
        permits: Arc<tokio::sync::Semaphore>,
    ) -> io::Result<(Self, Option<Task>)> {
        let (filters, updater) = filtering::prepare(config.filtering).await?;
        let rewrites = tokio::task::spawn_blocking(move || Rewrites::new(config.rewrites))
            .await
            .map_err(io::Error::other)?;
        let resolver = Self {
            forwarder: Forwarder::new(
                config.dns.upstreams,
                config.dns.upstream_timeout,
                counters,
                permits,
            ),
            rewrites,
            filters,
            cache: ArcSwap::from_pointee(ResponseCache::new(config.cache)),
        };
        let updater = updater.map(|updater| Task(tokio::spawn(updater.run())));
        Ok((resolver, updater))
    }
}

struct Task(JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) struct Runtime {
    pub current: ArcSwap<Resolver>,
    pub counters: Arc<Counters>,
    started: Instant,
    listen: SocketAddr,
    updater: Mutex<Option<Task>>,
    permits: Arc<tokio::sync::Semaphore>,
}

impl Runtime {
    pub async fn prepare(config: Config) -> io::Result<Arc<Self>> {
        let listen = config.dns.listen;
        let counters = Arc::new(Counters::default());
        let permits = Arc::new(tokio::sync::Semaphore::new(256));
        let (resolver, updater) =
            Resolver::prepare(config, counters.clone(), permits.clone()).await?;
        Ok(Arc::new(Self {
            current: ArcSwap::from_pointee(resolver),
            counters,
            permits,
            started: Instant::now(),
            listen,
            updater: Mutex::new(updater),
        }))
    }

    pub async fn reload(&self, config: Config) -> io::Result<()> {
        if config.dns.listen != self.listen {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "modifier dns.listen nécessite un redémarrage",
            ));
        }
        let mut updater = self.updater.lock().await;
        let (resolver, next_updater) =
            Resolver::prepare(config, self.counters.clone(), self.permits.clone()).await?;
        // Aucun await entre la publication et la fin du commit : une annulation
        // conserve soit l'ancien état complet, soit le nouveau.
        self.current.load().forwarder.stop_probes();
        self.current.store(Arc::new(resolver));
        *updater = next_updater;
        Counters::increment(&self.counters.reloads);
        Ok(())
    }

    pub async fn clear_cache(&self) {
        let _update = self.updater.lock().await;
        let resolver = self.current.load();
        let empty = resolver.cache.load().empty();
        // Les requêtes en vol gardent leur ancien cache, désormais inaccessible.
        resolver.cache.store(Arc::new(empty));
        Counters::increment(&self.counters.cache_clears);
    }

    pub async fn stats(&self) -> ServerStats {
        let resolver = self.current.load_full();
        let cache = resolver.cache.load_full().stats().await;
        self.counters.snapshot(
            self.started.elapsed().as_secs(),
            resolver.filters.load().len(),
            cache,
        )
    }

    pub async fn stop_updater(&self) {
        self.current.load().forwarder.stop_probes();
        if let Some(mut task) = self.updater.lock().await.take() {
            task.0.abort();
            let _ = (&mut task.0).await;
        }
    }
}
