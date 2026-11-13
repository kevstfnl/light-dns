# light-dns

> Un serveur DNS léger pour Linux, écrit en Rust.

**Filtrage de domaines** · **Réécritures locales** · **Cache TTL** · **UDP/TCP, IPv4 et IPv6**

[Installation native](#installation-native) · [Docker](#docker-compose) · [Développement](#développement) · [Documentation](docs/utilisation.md)

---

## Installation native

**Alpine Linux AArch64 · Raspberry Pi 5 · OpenRC · installation sys**

Depuis un shell **root** :

```sh
(s=$(wget -T 60 -qO- https://github.com/kevstfnl/light-dns/releases/download/v0.2.0-beta.3/bootstrap.sh) && sh -c "$s")
```

Installe le binaire et démarre le service sur `127.0.0.1:53`, sans compilateur ni Docker. Configuration : `/etc/light-dns/config.toml`.

Le script distant s’exécute en root. [Installation manuelle, mises à jour et désinstallation](docs/alpine.md). **Bêta : validation sur Pi physique encore nécessaire.**

---

## Docker Compose

Prérequis : Git, Docker et Docker Compose.

```sh
git clone https://github.com/kevstfnl/light-dns.git
cd light-dns
# Adapter docker/config.toml avant de démarrer.
docker compose up -d --build
```

Le DNS écoute sur `127.0.0.1:53` par défaut. [Accès LAN, configuration et gestion Docker](docs/docker.md).

---

## Développement

Prérequis : Git, Rust/Cargo, compilateur C, make, CMake et certificats CA.

```sh
git clone https://github.com/kevstfnl/light-dns.git
cd light-dns
cargo run --locked -- --config config.toml
```

Le fichier fourni écoute sur `127.0.0.1:1053`. Avant de proposer un changement :

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked
```

[Développement et compilation](docs/developpement.md) · [Configuration et administration](docs/utilisation.md) · [Maintenance](docs/maintenance.md) · [Règles de contribution](AGENTS.md)
