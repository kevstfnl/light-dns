use std::{ffi::OsString, io, path::PathBuf};

pub(super) const HELP: &str = "light-dns [--config CHEMIN] [--socket CHEMIN] [COMMANDE]

Gestion Alpine (root pour modifier le système) :\n  install\n  update [--check | --version VERSION]\n  uninstall [--purge]\n  version\n\nSans commande : démarrer le serveur DNS.
  serve        Démarrer le serveur DNS
  cache status Afficher l’état du cache DNS\n  cache clear  Vider le cache DNS du serveur actif
  reload       Recharger son fichier TOML, sans changer ses écoutes
  stats        Afficher les statistiques du serveur actif
  health       Vérifier UDP/TCP local (--upstream pour tester aussi les upstreams)\n  --help       Afficher cette aide

Défaut serveur : --config config.toml ; --socket light-dns.sock.
Administration : --socket explicite, sinon socket locale si présente,
sinon /run/light-dns/control.sock si présente.
Les commandes d’administration ne chargent pas le TOML.
";

pub(super) enum Command {
    Serve,
    ClearCache,
    CacheStatus,
    Reload,
    Stats,
    Health(bool),
    Help,
    Version,
    Install,
    Update {
        check: bool,
        version: Option<String>,
    },
    Uninstall(bool),
}

pub(super) struct Arguments {
    pub config: PathBuf,
    pub socket: PathBuf,
    pub command: Command,
}

impl Arguments {
    pub fn parse(args: impl Iterator<Item = OsString>) -> io::Result<Self> {
        let mut config = PathBuf::from("config.toml");
        let mut socket = None;
        let mut words = Vec::new();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            if arg == "--help" || arg == "-h" {
                return Ok(Self {
                    config,
                    socket: socket.unwrap_or_else(|| PathBuf::from("light-dns.sock")),
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
                    socket = Some(PathBuf::from(value));
                }
            } else {
                words.push(arg);
            }
        }
        let command = match words.as_slice() {
            [one] if one == "version" => Command::Version,
            [one] if one == "install" => Command::Install,
            [one] if one == "update" => Command::Update {
                check: false,
                version: None,
            },
            [one, two] if one == "update" && two == "--check" => Command::Update {
                check: true,
                version: None,
            },
            [one, two, version] if one == "update" && two == "--version" => Command::Update {
                check: false,
                version: Some(
                    version
                        .to_str()
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::InvalidInput, "version UTF-8 requise")
                        })?
                        .to_owned(),
                ),
            },
            [one] if one == "uninstall" => Command::Uninstall(false),
            [one, two] if one == "uninstall" && two == "--purge" => Command::Uninstall(true),
            [] => Command::Serve,
            [one] if one == "serve" => Command::Serve,
            [one] if one == "reload" => Command::Reload,
            [one] if one == "stats" => Command::Stats,
            [one] if one == "health" => Command::Health(false),
            [one, two] if one == "health" && two == "--upstream" => Command::Health(true),
            [one, two] if one == "cache" && two == "clear" => Command::ClearCache,
            [one, two] if one == "cache" && two == "status" => Command::CacheStatus,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "commande inconnue ; consulter light-dns --help",
                ));
            }
        };
        let socket = match socket {
            Some(path) => path,
            None if matches!(
                command,
                Command::Health(_)
                    | Command::Stats
                    | Command::Reload
                    | Command::ClearCache
                    | Command::CacheStatus
            ) =>
            {
                administration_socket(
                    std::path::Path::new("light-dns.sock"),
                    std::path::Path::new("/run/light-dns/control.sock"),
                )?
            }
            None => PathBuf::from("light-dns.sock"),
        };
        Ok(Self {
            config,
            socket,
            command,
        })
    }
}

// Select once before connecting: an unavailable local instance must never redirect
// an administrative command (especially reload/cache clear) to another daemon.
fn administration_socket(
    local: &std::path::Path,
    service: &std::path::Path,
) -> io::Result<PathBuf> {
    fn present(path: &std::path::Path) -> io::Result<bool> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
    if present(local)? || !present(service)? {
        Ok(local.to_owned())
    } else {
        Ok(service.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> io::Result<Self> {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "light-dns-cli-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn selects_local_then_service_and_retains_local_default_when_absent() -> io::Result<()> {
        let directory = Directory::new()?;
        let local = directory.0.join("light-dns.sock");
        let service = directory.0.join("control.sock");
        assert_eq!(administration_socket(&local, &service)?, local);
        fs::write(&service, "fixture")?;
        assert_eq!(administration_socket(&local, &service)?, service);
        fs::write(&local, "invalid local socket must still win")?;
        assert_eq!(administration_socket(&local, &service)?, local);
        // A lookup error must not redirect a destructive administrative command.
        assert!(administration_socket(&local.join("child"), &service).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn stale_socket_or_dangling_symlink_never_redirects_to_service() -> io::Result<()> {
        use std::os::unix::{fs::symlink, net::UnixListener};
        let directory = Directory::new()?;
        let local = directory.0.join("light-dns.sock");
        let service = directory.0.join("control.sock");
        let _service = UnixListener::bind(&service)?;
        drop(UnixListener::bind(&local)?);
        assert_eq!(administration_socket(&local, &service)?, local);
        fs::remove_file(&local)?;
        symlink(directory.0.join("missing"), &local)?;
        assert_eq!(administration_socket(&local, &service)?, local);
        Ok(())
    }

    #[test]
    fn explicit_socket_always_wins_and_serve_keeps_its_default() -> io::Result<()> {
        for words in [
            vec!["health"],
            vec!["health", "--upstream"],
            vec!["stats"],
            vec!["reload"],
            vec!["cache", "clear"],
            vec!["cache", "status"],
            vec!["serve"],
        ] {
            let mut words = words;
            words.extend(["--socket", "chosen.sock"]);
            let args = Arguments::parse(words.into_iter().map(OsString::from))?;
            assert_eq!(args.socket, Path::new("chosen.sock"));
        }
        for words in [vec![], vec!["serve"]] {
            let args = Arguments::parse(words.into_iter().map(OsString::from))?;
            assert_eq!(args.socket, Path::new("light-dns.sock"));
        }
        Ok(())
    }
}
