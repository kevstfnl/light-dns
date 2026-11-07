mod socket;

use super::cli::Command;
use crate::{config::Config, dns::server::DnsControl};
use socket::{Listener, read_frame};
use std::{io, path::PathBuf, time::Duration};
use tokio::{io::AsyncWriteExt, net::UnixStream, task::JoinSet, time::timeout};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CLIENTS: usize = 8;

pub(super) struct ControlServer {
    listener: Listener,
    clients: JoinSet<()>,
}

impl ControlServer {
    pub fn bind(path: PathBuf) -> io::Result<Self> {
        Ok(Self {
            listener: Listener::bind(path)?,
            clients: JoinSet::new(),
        })
    }

    pub async fn run(&mut self, control: DnsControl, config: PathBuf) -> io::Result<()> {
        loop {
            tokio::select! {
                accepted = self.listener.accept(), if self.clients.len() < MAX_CLIENTS => {
                    let mut stream = accepted?;
                    let control = control.clone();
                    let config = config.clone();
                    self.clients.spawn(async move {
                        let result = timeout(COMMAND_TIMEOUT, execute(&mut stream, &control, config)).await;
                        let result = match result {
                            Ok(result) => result,
                            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "commande expirée")),
                        };
                        let response = match result {
                            Ok(body) => format!("OK\n{body}"),
                            Err(error) => format!("ERR\n{error}\n"),
                        };
                        let _ = timeout(IO_TIMEOUT, stream.write_all(response.as_bytes())).await;
                    });
                }
                _ = self.clients.join_next(), if !self.clients.is_empty() => {}
            }
        }
    }

    pub async fn shutdown(&mut self) {
        self.clients.abort_all();
        while self.clients.join_next().await.is_some() {}
    }
}

async fn execute(
    stream: &mut UnixStream,
    control: &DnsControl,
    config: PathBuf,
) -> io::Result<String> {
    let command = timeout(IO_TIMEOUT, read_frame(stream, 64)).await??;
    match command.as_slice() {
        b"cache clear\n" => {
            control.clear_cache().await;
            Ok("cache DNS vidé\n".into())
        }
        b"reload\n" => {
            // La lecture TOML et son parsing ne bloquent pas les workers DNS.
            let config = tokio::task::spawn_blocking(move || Config::load_from(&config))
                .await
                .map_err(io::Error::other)?
                .map_err(io::Error::other)?;
            control.reload(config).await?;
            tracing::info!("configuration rechargée ; cache DNS remplacé");
            Ok("configuration rechargée\n".into())
        }
        b"stats\n" => toml::to_string(&control.stats().await).map_err(io::Error::other),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "commande de contrôle inconnue",
        )),
    }
}

pub(super) async fn request(path: PathBuf, command: Command) -> io::Result<String> {
    let text = match command {
        Command::ClearCache => b"cache clear\n".as_slice(),
        Command::Reload => b"reload\n".as_slice(),
        Command::Stats => b"stats\n".as_slice(),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "commande non administrative",
            ));
        }
    };
    timeout(COMMAND_TIMEOUT + IO_TIMEOUT, async {
        let mut stream = UnixStream::connect(path).await?;
        stream.write_all(text).await?;
        let response = socket::read_response(&mut stream, 16384).await?;
        if let Some(body) = response.strip_prefix("OK\n") {
            Ok(body.to_owned())
        } else if let Some(error) = response.strip_prefix("ERR\n") {
            Err(io::Error::other(error.trim_end().to_owned()))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "réponse de contrôle invalide",
            ))
        }
    })
    .await?
}
