# light-dns

Serveur DNS léger pour Linux, écrit en Rust : filtrage de domaines, réécritures locales, cache TTL et forwarding UDP/TCP, en IPv4 et IPv6.

## Installation native — Alpine Linux / Raspberry Pi 5

Sur **Alpine AArch64, installation sys avec OpenRC**, depuis un shell **root** :

```sh
(umask 077; f=$(mktemp) && trap 'rm -f "$f"' EXIT && wget -T 60 -qO "$f" https://github.com/kevstfnl/light-dns/releases/download/v0.2.0-beta.2/bootstrap.sh && sh "$f")
```

Installe le binaire et démarre le service sur `127.0.0.1:53`, sans compilateur ni Docker. Configuration : `/etc/light-dns/config.toml`.

Le script distant s’exécute en root. [Installation manuelle, mises à jour et désinstallation](docs/alpine.md). **Bêta : validation sur Pi physique encore nécessaire.**

## Installation Docker

Prérequis : Git, Docker et Docker Compose.

```sh
git clone https://github.com/kevstfnl/light-dns.git
cd light-dns
# Adapter docker/config.toml avant de démarrer.
docker compose up -d --build
```

Le DNS écoute sur `127.0.0.1:53` par défaut. [Accès LAN, configuration et gestion Docker](docs/docker.md).

## Développer

Prérequis : Git, Rust/Cargo, compilateur C, make, CMake et certificats CA.

```sh
git clone https://github.com/kevstfnl/light-dns.git
cd light-dns
cargo run --locked -- --config config.toml
```

Le fichier fourni écoute sur `127.0.0.1:1053`. Pour vérifier ses changements :

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked
```

[Développement et compilation](docs/developpement.md) · [Configuration et administration](docs/utilisation.md) · [Maintenance](docs/maintenance.md) · [Règles de contribution](AGENTS.md)
