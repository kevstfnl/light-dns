# Configuration et administration

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

Depuis v0.2.0-beta.3, la CLI sélectionne la socket ainsi : `--socket` explicite, sinon `light-dns.sock` dans le dossier courant si présente, sinon `/run/light-dns/control.sock` si présente. Sans socket présente, le chemin local reste le défaut. Une erreur sur la socket choisie ne provoque aucun essai sur une autre instance.

Ainsi, `light-dns health`, `stats`, `reload` et `cache clear` trouvent automatiquement le service OpenRC. Les versions antérieures exigent encore `--socket`.

Pour un chemin personnalisé, préciser le chemin choisi au démarrage avec `--socket` :

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

Pour les chemins et commandes du service OpenRC, consulter le [guide Alpine](alpine.md). Pour les volumes et le réseau Compose, consulter le [guide Docker](docker.md).

## État du cache

```sh
light-dns cache status
light-dns cache clear
```

`cache status` affiche un résumé TOML : `enabled`, `entries`, `max_entries`, `estimated_bytes`, `max_memory_bytes`, `max_ttl_secs`, `hits` et `clears`. Les nombres d’entrées et la taille sont indicatifs après maintenance interne du cache. La mémoire pondérée n’est pas la RAM totale du processus. Hits et vidages sont cumulés depuis le démarrage, y compris après un reload ou un vidage ; aucun TTL ni contenu n’est modifié par cette lecture.

La socket est sélectionnée comme pour `stats` ; `--socket CHEMIN` reste prioritaire. Sous Docker : `docker compose exec light-dns light-dns cache status`. Disponible depuis v0.2.0-beta.3.
