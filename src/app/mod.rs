mod shutdown;

use crate::{config::Config, dns::server};
use shutdown::shutdown_signal;
use std::{io, time::Duration};

/// Charge la configuration et exécute le service jusqu'à son arrêt.
pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::load()?;
    tracing_subscriber::fmt()
        .with_env_filter("warn,light_dns=info")
        .with_ansi(false)
        .with_writer(io::stderr)
        .try_init()?;
    let signal = shutdown_signal()?;
    let (mut server, address) = server::bind(config).await?;
    tracing::info!(%address, "serveur DNS prêt en UDP et TCP");
    tokio::select! {
        result = server.block_until_done() => result?,
        signal = signal => {
            signal?;
            tracing::info!("arrêt du serveur DNS");
            tokio::time::timeout(Duration::from_secs(5), server.shutdown_gracefully()).await??;
        }
    }
    Ok(())
}
