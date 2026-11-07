mod handler;
mod response;

pub use super::cache::CacheStats;
use super::cache::ResponseCache;
use crate::config::Config;
use crate::filtering;
use handler::Handler;
use hickory_server::{Server, net::NetError};
use std::{io, net::SocketAddr, time::Duration};
use tokio::net::{TcpListener, UdpSocket};

const TCP_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Serveur DNS UDP/TCP et gestion de son cycle de vie.
pub struct DnsServer {
    inner: Server<Handler>,
    updater: Option<tokio::task::JoinHandle<()>>,
    cache: ResponseCache,
}

impl DnsServer {
    /// Statistiques locales du cache, sans interface d'administration réseau.
    pub async fn cache_stats(&self) -> CacheStats {
        self.cache.stats().await
    }

    /// Attend la fin des tâches d'écoute.
    pub async fn block_until_done(&mut self) -> Result<(), NetError> {
        self.inner.block_until_done().await
    }

    /// Ferme les écoutes et leurs tâches associées.
    pub async fn shutdown_gracefully(&mut self) -> Result<(), NetError> {
        if let Some(updater) = self.updater.take() {
            updater.abort();
            let _ = updater.await;
        }
        self.inner.shutdown_gracefully().await
    }
}

impl Drop for DnsServer {
    fn drop(&mut self) {
        if let Some(updater) = &self.updater {
            updater.abort();
        }
    }
}

/// Ouvre UDP et TCP sur la même adresse et retourne le serveur et l'adresse liée.
pub async fn bind(config: Config) -> io::Result<(DnsServer, SocketAddr)> {
    let (filters, updater) = filtering::prepare(config.filtering.clone()).await?;
    // Les deux binds doivent réussir avant d'enregistrer les tâches d'écoute.
    let udp = UdpSocket::bind(config.dns.listen).await?;
    let address = udp.local_addr()?;
    let tcp = TcpListener::bind(address).await?;
    let cache = ResponseCache::new(config.cache.clone());
    let mut server = Server::new(Handler::new(config, filters, cache.clone()));
    server.register_socket(udp);
    server.register_listener(tcp, TCP_IDLE_TIMEOUT, 1);
    let updater = updater.map(|updater| tokio::spawn(updater.run()));
    Ok((
        DnsServer {
            inner: server,
            updater,
            cache,
        },
        address,
    ))
}
