use std::{ffi::OsString, io, path::PathBuf};

pub(super) const HELP: &str = "light-dns [--config CHEMIN] [--socket CHEMIN] [COMMANDE]

Sans commande : démarrer le serveur DNS.
  serve        Démarrer le serveur DNS
  cache clear  Vider le cache DNS du serveur actif
  reload       Recharger son fichier TOML, sans changer ses écoutes
  stats        Afficher les statistiques du serveur actif
  health       Vérifier UDP/TCP local (--upstream pour tester aussi les upstreams)\n  --help       Afficher cette aide

Défauts : --config config.toml ; --socket light-dns.sock.
Les commandes d'administration utilisent uniquement --socket.
";

pub(super) enum Command {
    Serve,
    ClearCache,
    Reload,
    Stats,
    Health(bool),
    Help,
}

pub(super) struct Arguments {
    pub config: PathBuf,
    pub socket: PathBuf,
    pub command: Command,
}

impl Arguments {
    pub fn parse(args: impl Iterator<Item = OsString>) -> io::Result<Self> {
        let mut config = PathBuf::from("config.toml");
        let mut socket = PathBuf::from("light-dns.sock");
        let mut words = Vec::new();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            if arg == "--help" || arg == "-h" {
                return Ok(Self {
                    config,
                    socket,
                    command: Command::Help,
                });
            }
            if arg == "--config" || arg == "--socket" {
                let value = args
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "chemin manquant après l'option",
                        )
                    })?;
                if arg == "--config" {
                    config = value.into();
                } else {
                    socket = value.into();
                }
            } else {
                words.push(arg);
            }
        }
        let command = match words.as_slice() {
            [] => Command::Serve,
            [one] if one == "serve" => Command::Serve,
            [one] if one == "reload" => Command::Reload,
            [one] if one == "stats" => Command::Stats,
            [one] if one == "health" => Command::Health(false),
            [one, two] if one == "health" && two == "--upstream" => Command::Health(true),
            [one, two] if one == "cache" && two == "clear" => Command::ClearCache,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "commande inconnue ; consulter light-dns --help",
                ));
            }
        };
        Ok(Self {
            config,
            socket,
            command,
        })
    }
}
