# light-dns

light-dns est un serveur DNS headless léger, écrit en Rust pour Linux.
Il fournit le filtrage de domaines, les rewrites locaux, un cache à TTL et le forwarding UDP/TCP, en IPv4 et IPv6.

Pour une première installation sur **Raspberry Pi 5 sous Alpine Linux 64 bits**, utiliser l'archive native avec OpenRC : suivre le [guide Alpine](docs/alpine.md). Aucun compilateur, Docker ni Python n'est nécessaire sur le Pi. La première release reste en brouillon jusqu'à validation matérielle par le mainteneur ; le guide explique comment la récupérer avant publication.

## Docker Compose

Prérequis : Git, Docker et Docker Compose.

```bash
git clone https://github.com/kevstfnl/light-dns.git
cd light-dns
```

Modifier `docker/config.toml` : choisir les IP des upstreams dans `[dns]` et les adresses locales dans `[rewrites]`. Garder l’écoute interne `0.0.0.0:1053`. Les exemples complets se trouvent dans les fichiers TOML.

Compose publie **UDP et TCP sur `127.0.0.1:53` par défaut** : les autres appareils du LAN ne peuvent pas y accéder. Pour les desservir, créer un fichier `.env` contenant l’adresse LAN de cet hôte, par exemple :

```dotenv
LIGHT_DNS_BIND_IP=192.168.1.10
LIGHT_DNS_PORT=53
```

Choisir une adresse réellement présente sur l’hôte et un port libre. Autoriser UDP/TCP sur ce port dans le pare-feu du LAN ; ne pas exposer le résolveur à Internet. La publication fournie est IPv4 ; adapter les publications Compose pour desservir aussi les clients IPv6.

```bash
docker compose config --quiet
docker compose up -d --build
docker compose logs -f light-dns
docker compose exec light-dns light-dns health
```

Configurer ensuite le routeur/DHCP ou les clients pour utiliser l’adresse LAN du serveur comme DNS.

Le conteneur utilise l’UID/GID `65532:65532`, sans capacités privilégiées. Le dossier `docker/` est monté en lecture seule : ses fichiers doivent être lisibles et ses dossiers traversables par cet utilisateur. Les listes téléchargées persistent dans le volume `filter-data` ; la socket d’administration reste dans un `tmpfs` privé.

Après modification du TOML :

```bash
docker compose exec light-dns light-dns reload
```

Pour mettre à jour le code et reconstruire, en conservant la configuration locale et le fichier `.env` :

```bash
git pull --ff-only
docker compose up -d --build
```

Sauvegarder ses modifications locales avant la mise à jour et résoudre tout conflit de configuration avant de redémarrer. `docker compose restart light-dns` redémarre le service sans reconstruire ; `docker compose stop` l’arrête sans supprimer les listes persistées.

## Installation native Linux

## Installation rapide — Alpine Linux / Raspberry Pi 5

Sur **Alpine AArch64 en installation sys avec OpenRC**, depuis un shell **root** :

```sh
(umask 077; f=$(mktemp) && trap 'rm -f "$f"' EXIT && wget -T 60 -qO "$f" https://github.com/kevstfnl/light-dns/releases/download/v0.2.0-beta.2/bootstrap.sh && sh "$f")
```

Cette commande exige la publication de **v0.2.0-beta.2**. Elle télécharge un binaire musl statique, vérifie SHA-256, crée le compte système `light-dns`, active OpenRC et démarre le DNS sur **127.0.0.1:53**. Aucun Rust, Cargo, Docker, Git ou GitHub CLI sur le Pi. `libcap-setcap` est installé seulement si nécessaire pour le port privilégié ; les certificats CA système sont nécessaires à HTTPS. Aucun réglage réseau ni DNS système n’est modifié.

**Sécurité : le script distant s’exécute en root.** Faire confiance au dépôt GitHub et à sa chaîne de release est indispensable. SHA-256 détecte la corruption mais ne fournit pas une signature indépendante de GitHub. Pour inspecter le script et vérifier manuellement les fichiers avant exécution, suivre la [méthode manuelle](docs/alpine.md#installation-manuelle-avec-vérification). Ne jamais désactiver la vérification TLS.

```sh
rc-service light-dns status
rc-service light-dns stop
rc-service light-dns start
rc-service light-dns restart
rc-service light-dns reload
rc-update add light-dns default
light-dns --socket /run/light-dns/control.sock health
light-dns version
light-dns update --check
light-dns update
light-dns update --version v0.2.0-beta.2
light-dns uninstall
light-dns uninstall --purge
```

`update` inclut les bêtas, conserve le TOML et les données et restaure l’ancienne installation si le redémarrage ou la santé locale échoue. Un service déjà arrêté reste arrêté. Seul `--version` permet une rétrogradation explicite. `uninstall` conserve configuration, données et compte ; `--purge` exige de taper `PURGE` sur un terminal et ne supprime que les comptes créés par cet installateur. Aucun paquet partagé n’est désinstallé.

| Usage | Emplacement |
| --- | --- |
| Binaire | `/usr/local/bin/light-dns` |
| Configuration | `/etc/light-dns/config.toml` |
| Données | `/var/lib/light-dns/` |
| Service OpenRC | `/etc/init.d/light-dns` |
| Socket et fichiers temporaires du service | `/run/light-dns/` |

La bêta peut être publiée après CI ; **l’exécution sur Raspberry Pi physique reste à valider par le mainteneur**. Le [guide Alpine](docs/alpine.md) détaille les permissions, les erreurs et la validation matérielle.

### Compilation depuis les sources

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

## Configuration minimale

```toml
[dns]
listen = "127.0.0.1:1053"
upstreams = ["1.1.1.1:53", "9.9.9.9:53"]

[rewrites]
"router.home.arpa" = "192.168.1.1"
"*.home.arpa" = "192.168.1.10"

[filtering]
enabled = true
cache_dir = "data/filters"
```

Le cache DNS est actif par défaut. Le filtrage télécharge par défaut les listes AdGuard DNS et Peter Lowe ; ajouter `sources = []` dans `[filtering]` pour utiliser seulement ses fichiers locaux. Au premier démarrage sans cache disque, les listes distantes deviennent actives après leur téléchargement. Le dossier `data/filters` est relatif au répertoire de lancement ; en Docker, utiliser le chemin `/var/lib/light-dns/filters` de l’exemple fourni.

Les rewrites exacts sont prioritaires. Un wildcard réserve sa zone locale sans forwarding, même en cas de panne des upstreams ; il ne fournit pas d’adresse à son apex. Les réécritures des listes distantes sont interdites par défaut ; `allow_dnsrewrite = true` accorde cette confiance explicitement à une source HTTPS.

## Administration

En natif, préciser le chemin choisi au démarrage avec `--socket` :

```bash
light-dns --socket "$HOME/.local/state/light-dns/control.sock" stats
light-dns --socket "$HOME/.local/state/light-dns/control.sock" reload
light-dns --socket "$HOME/.local/state/light-dns/control.sock" cache clear
light-dns --socket "$HOME/.local/state/light-dns/control.sock" health
light-dns --socket "$HOME/.local/state/light-dns/control.sock" health --upstream
```

Dans Compose, utiliser la socket par défaut du conteneur : par exemple `docker compose exec light-dns light-dns stats`, ou remplacer `stats` par les autres commandes ci-dessus, sans l’option `--socket` native.

`stats` affiche du TOML ; `health` vérifie le contrôle et le DNS local UDP/TCP, et `--upstream` ajoute une résolution externe. Un échec renvoie un code non nul. Seuls l’utilisateur du service et root peuvent administrer la socket.

Sans options, le serveur utilise `config.toml` et `light-dns.sock` dans le répertoire courant. Les commandes administratives utilisent uniquement la socket. `reload` relit le TOML du serveur, vide le cache DNS en cas de succès et conserve l’état actif en cas d’erreur. Un changement d’écoute exige un redémarrage ; aucune surveillance automatique du TOML n’est effectuée.

Tests, mesures reproductibles et maintenance : [guide court](docs/maintenance.md). Licence [MIT](LICENSE).
