mod handler;
mod response;
mod runtime;
mod stats;

pub use super::cache::CacheStats;
use crate::config::Config;
use handler::Handler;
use hickory_server::{Server, net::NetError};
use runtime::Runtime;
pub use stats::ServerStats;
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::{TcpListener, UdpSocket};

const TCP_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Serveur DNS UDP/TCP et gestion de son cycle de vie.
pub struct DnsServer {
    inner: Server<Handler>,
    control: DnsControl,
}

/// Contrôle local du serveur ; ne modifie pas ses écoutes réseau.
#[derive(Clone)]
pub struct DnsControl {
    runtime: Arc<Runtime>,
}

impl DnsControl {
    pub async fn reload(&self, config: Config) -> io::Result<()> {
        self.runtime.reload(config).await
    }

    pub async fn clear_cache(&self) {
        self.runtime.clear_cache().await;
    }

    pub async fn stats(&self) -> ServerStats {
        self.runtime.stats().await
    }
}

impl DnsServer {
    pub fn control(&self) -> DnsControl {
        self.control.clone()
    }

    /// Statistiques locales du cache, sans interface d'administration réseau.
    pub async fn cache_stats(&self) -> CacheStats {
        self.control
            .runtime
            .current
            .load_full()
            .cache
            .load_full()
            .stats()
            .await
    }

    /// Attend la fin des tâches d'écoute.
    pub async fn block_until_done(&mut self) -> Result<(), NetError> {
        self.inner.block_until_done().await
    }

    /// Ferme les écoutes et leurs tâches associées.
    pub async fn shutdown_gracefully(&mut self) -> Result<(), NetError> {
        self.control.runtime.stop_updater().await;
        self.inner.shutdown_gracefully().await
    }
}

/// Ouvre UDP et TCP sur la même adresse et retourne le serveur et l'adresse liée.
pub async fn bind(config: Config) -> io::Result<(DnsServer, SocketAddr)> {
    // Les deux binds doivent réussir avant d'enregistrer les tâches d'écoute.
    let udp = UdpSocket::bind(config.dns.listen).await?;
    let address = udp.local_addr()?;
    let tcp = TcpListener::bind(address).await?;
    let runtime = Runtime::prepare(config).await?;
    let mut server = Server::new(Handler::new(runtime.clone()));
    server.register_socket(udp);
    server.register_listener(tcp, TCP_IDLE_TIMEOUT, 1);
    Ok((
        DnsServer {
            inner: server,
            control: DnsControl { runtime },
        },
        address,
    ))
}
