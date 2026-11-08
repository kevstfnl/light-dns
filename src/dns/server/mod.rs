mod handler;
mod health;
mod response;
mod runtime;
pub(super) mod stats;

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
    address: SocketAddr,
}

impl DnsControl {
    /// Vérifie les transports locaux, puis éventuellement une résolution directe upstream.
    pub async fn health(&self, upstream: bool) -> io::Result<()> {
        health::check(self.address, &self.runtime, upstream).await
    }

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
    let (udp, tcp, address) = bind_transports(config.dns.listen).await?;
    let runtime = Runtime::prepare(config).await?;
    let mut server = Server::new(Handler::new(runtime.clone()));
    server.register_socket(udp);
    server.register_listener(tcp, TCP_IDLE_TIMEOUT, 1);
    Ok((
        DnsServer {
            inner: server,
            control: DnsControl { runtime, address },
        },
        address,
    ))
}

// Un port choisi automatiquement doit être libre dans les deux espaces UDP/TCP.
async fn bind_transports(listen: SocketAddr) -> io::Result<(UdpSocket, TcpListener, SocketAddr)> {
    for attempt in 0..16 {
        let tcp = TcpListener::bind(listen).await?;
        let address = tcp.local_addr()?;
        match UdpSocket::bind(address).await {
            Ok(udp) => return Ok((udp, tcp, address)),
            Err(error)
                if listen.port() == 0
                    && error.kind() == io::ErrorKind::AddrInUse
                    && attempt < 15 => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("aucun port UDP/TCP commun disponible"))
}
