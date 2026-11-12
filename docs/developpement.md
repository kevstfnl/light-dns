# Développement et installation depuis les sources


Prérequis : Git, Rust/Cargo (la CI utilise Rust **1.99.1**), un compilateur C, make, CMake et les certificats CA du système.

```bash
git clone https://github.com/kevstfnl/light-dns.git
cd light-dns
cargo build --release --locked
# Installation du binaire, puis exécution sans root :
sudo install -m755 target/release/light-dns /usr/local/bin/light-dns
install -d -m700 "$HOME/.local/state/light-dns"
light-dns --config "$PWD/config.toml" --socket "$HOME/.local/state/light-dns/control.sock"
```

Modifier auparavant `config.toml` : l’exemple écoute sur `127.0.0.1:1053`. Pour le LAN, choisir l’IP LAN et le port voulus. Le port 1053 ne demande aucun privilège particulier ; les clients DNS ordinaires attendent le port 53.

Si Linux exige un privilège pour le port 53, installer `setcap` (paquet libcap de la distribution), puis donner uniquement la capacité d’écoute au binaire :

```bash
sudo setcap 'cap_net_bind_service=+ep' /usr/local/bin/light-dns
```

Cette capacité doit être réappliquée après remplacement du binaire. Exécuter le service avec un utilisateur dédié ou son utilisateur habituel, sans root. Le dossier de la socket doit appartenir à cet utilisateur ou à root, être accessible en écriture au service et ne pas être modifiable par le groupe ou les autres ; ses ancêtres doivent rester sous contrôle. Le cache des listes doit également être accessible en écriture.

Le serveur reste au premier plan : `Ctrl+C`, `SIGINT` ou `SIGTERM` l’arrête proprement. Un service OpenRC est fourni dans l’archive Alpine ; aucun service systemd n’est fourni. Pour mettre à jour une installation depuis les sources : arrêter, récupérer le code, recompiler avec le lockfile, réinstaller le binaire puis relancer la même commande.

## Vérifier les changements

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked
```

Lire les [règles de contribution](../AGENTS.md). Pour les fixtures, mesures et releases, consulter le [guide de maintenance](maintenance.md).
