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

`stats` et `cache status` affichent des tableaux en terminal, et du TOML lorsque la sortie est redirigée ; `health` vérifie le contrôle et le DNS local UDP/TCP, et `--upstream` ajoute une résolution externe. Un échec renvoie un code non nul. Seuls l’utilisateur du service et root peuvent administrer la socket.

Sans options, le serveur utilise `config.toml` et `light-dns.sock` dans le répertoire courant. Les commandes administratives utilisent uniquement la socket. `reload` relit le TOML du serveur, vide le cache DNS en cas de succès et conserve l’état actif en cas d’erreur. Un changement d’écoute exige un redémarrage ; aucune surveillance automatique du TOML n’est effectuée.

Pour les chemins et commandes du service OpenRC, consulter le [guide Alpine](alpine.md). Pour les volumes et le réseau Compose, consulter le [guide Docker](docker.md).

## État du cache

```sh
light-dns cache status
light-dns cache clear
```

`cache status` fournit les champs TOML suivants (ou leur présentation en tableau) : `enabled`, `entries`, `max_entries`, `estimated_bytes`, `max_memory_bytes`, `max_ttl_secs`, `hits` et `clears`. Les nombres d’entrées et la taille sont indicatifs après maintenance interne du cache. La mémoire pondérée n’est pas la RAM totale du processus. Hits et vidages sont cumulés depuis le démarrage, y compris après un reload ou un vidage ; aucun TTL ni contenu n’est modifié par cette lecture.

La socket est sélectionnée comme pour `stats` ; `--socket CHEMIN` reste prioritaire. Sous Docker : `docker compose exec light-dns light-dns cache status`. Disponible depuis v0.2.0-beta.3.

## Affichage et diagnostic

Les titres sont colorés en terminal, les succès en vert et les points à vérifier
en jaune. Les erreurs de commande apparaissent en rouge
sur stderr. Les couleurs sont désactivées si la sortie n’est pas un terminal,
si `TERM=dumb`, ou avec `--no-color`/`NO_COLOR` (même vide). Les logs du serveur
conservent leur format sans couleur.

```sh
light-dns stats                         # tableau en terminal
light-dns cache status --format table   # tableau même dans une redirection
light-dns stats --format toml > stats.toml
light-dns --no-color stats
NO_COLOR=1 light-dns health
```

`--format auto` est le défaut ; `table` et `toml` sont réservés à `stats` et
`cache status`. Le format TOML n’ajoute ni décoration ni code ANSI. Les champs
existants et le protocole de contrôle `OK`/`ERR` sont conservés. Un ancien serveur
reste administrable ; les nouveaux champs absents sont marqués indisponibles
par l’affichage humain. Une erreur garde un code de sortie non nul.

`stats` regroupe les compteurs DNS, les échecs/évitements/sondes upstream, les
règles actives et l’occupation du cache. Il affiche la durée de fonctionnement,
la part des requêtes bloquées, la part servie depuis le cache par rapport à
**toutes les requêtes reçues**, et le débit moyen depuis le démarrage. Ce débit
n’est pas une mesure instantanée ; le pourcentage cache ne mesure pas seulement
les requêtes admissibles au cache. Sans dénominateur, le résultat est indisponible.

Le nouveau tableau TOML `[process]` contient `pid`, `version`, `os`, `arch`, et,
sous Linux quand `/proc/self/status` est lisible, `rss_bytes`, `peak_rss_bytes`,
`data_bytes` et `threads`. Il décrit le **serveur interrogé**, pas le client CLI.
Les champs mémoire sont en octets ; le tableau humain présente aussi Kio/Mio/Gio.
La lecture est effectuée uniquement lors de `stats`, sans surveillance automatique
ni travail supplémentaire pour les requêtes DNS. Les mesures absentes sont
omises du TOML et `measurement_error` explique une collecte indisponible ou
incomplète ; elles ne sont jamais remplacées par zéro.

Pour diagnostiquer une hausse mémoire, comparer RSS actuelle, pic RSS et poids
estimé du cache sur plusieurs relevés. VmData est la taille des données virtuelles,
**pas la RAM résidente** ; le pic RSS est cumulé depuis le démarrage. Le budget
Moka ne plafonne pas la mémoire totale du processus. Voir aussi le
[protocole de mesure du filtrage](filter-memory.md).

Les avertissements d’erreurs DNS/upstream portent sur des compteurs cumulés :
ils ne prouvent pas une panne actuelle. Vérifier `health`, puis `health --upstream`
et les journaux du service. Zéro règle peut correspondre à un filtrage désactivé
ou à des listes en attente. Un cache proche de 90 % de son budget est signalé,
en précisant que les évictions à la limite sont normales. Un cache activé avec
une capacité ou un TTL nul est signalé comme ne conservant aucune réponse.

Ces informations n’entraînent aucun téléchargement, rechargement de TOML ou
changement de configuration. Seul `health --upstream` ajoute explicitement le
contrôle de résolution externe déjà prévu par cette commande.
