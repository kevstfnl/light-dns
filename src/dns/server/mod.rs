mod handler;
mod response;

use crate::config::Config;
use handler::Handler;
use hickory_server::{Server, net::NetError};
use std::{io, net::SocketAddr, time::Duration};
use tokio::net::{TcpListener, UdpSocket};

const TCP_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Serveur DNS UDP/TCP et gestion de son cycle de vie.
pub struct DnsServer {
    inner: Server<Handler>,
}

impl DnsServer {
    /// Attend la fin des tâches d'écoute.
    pub async fn block_until_done(&mut self) -> Result<(), NetError> {
        self.inner.block_until_done().await
    }

    /// Ferme les écoutes et leurs tâches associées.
    pub async fn shutdown_gracefully(&mut self) -> Result<(), NetError> {
        self.inner.shutdown_gracefully().await
    }
}

/// Ouvre UDP et TCP sur la même adresse et retourne le serveur et l'adresse liée.
pub async fn bind(config: Config) -> io::Result<(DnsServer, SocketAddr)> {
    // Les deux binds doivent réussir avant d'enregistrer les tâches d'écoute.
    let udp = UdpSocket::bind(config.dns.listen).await?;
    let address = udp.local_addr()?;
    let tcp = TcpListener::bind(address).await?;
    let mut server = Server::new(Handler::new(config));
    server.register_socket(udp);
    server.register_listener(tcp, TCP_IDLE_TIMEOUT, 1);
    Ok((DnsServer { inner: server }, address))
}
