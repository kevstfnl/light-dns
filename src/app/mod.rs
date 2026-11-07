mod cli;
#[cfg(unix)]
mod control;
mod shutdown;

use crate::{config::Config, dns::server};
use cli::{Arguments, Command};
use shutdown::shutdown_signal;
use std::{io, time::Duration};

/// Charge la configuration et exécute le service jusqu'à son arrêt.
pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Arguments::parse(std::env::args_os().skip(1))?;
    if matches!(args.command, Command::Help) {
        print!("{}", cli::HELP);
        return Ok(());
    }
    if !matches!(args.command, Command::Serve) {
        #[cfg(unix)]
        {
            print!("{}", control::request(args.socket, args.command).await?);
            return Ok(());
        }
        #[cfg(not(unix))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "administration locale disponible sous Unix uniquement",
        )
        .into());
    }
    let config = Config::load_from(&args.config)?;
    tracing_subscriber::fmt()
        .with_env_filter("warn,light_dns=info")
        .with_ansi(false)
        .with_writer(io::stderr)
        .try_init()?;
    let signal = shutdown_signal()?;
    #[cfg(unix)]
    let mut commands = control::ControlServer::bind(args.socket)?;
    let (mut server, address) = server::bind(config).await?;
    tracing::info!(%address, "serveur DNS prêt en UDP et TCP");
    let controller = server.control();
    let admin = async {
        #[cfg(unix)]
        return commands.run(controller, args.config).await;
        #[cfg(not(unix))]
        std::future::pending::<io::Result<()>>().await
    };
    let result: Result<(), Box<dyn std::error::Error + Send + Sync>> = tokio::select! {
        result = server.block_until_done() => result.map_err(Into::into),
        result = signal => result.map_err(Into::into),
        result = admin => result.map_err(Into::into),
    };
    // Annuler les commandes avant d'arrêter le serveur évite qu'un reload
    // concurrent relance une tâche de filtrage pendant l'arrêt.
    #[cfg(unix)]
    commands.shutdown().await;
    tracing::info!("arrêt du serveur DNS");
    tokio::time::timeout(Duration::from_secs(5), server.shutdown_gracefully()).await??;
    result
}
