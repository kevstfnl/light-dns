# light-dns

Un serveur DNS léger et sans interface graphique, développé en Rust pour les homelabs et les environnements à ressources limitées.

Configuration simple, faible consommation de ressources et déploiement Docker.

> **Statut :** Serveur DNS UDP/TCP avec forwarding, rewrites locaux A/AAAA, filtrage, cache DNS, commandes locales d'administration et déploiement Docker. Validation ARM64 sur Raspberry Pi encore à effectuer.

## Fonctionnalités disponibles

- Écoute UDP et TCP sur la même adresse, en IPv4 ou IPv6.
- Forwarding vers les upstreams configurés, avec timeouts, bascule adaptative et repli TCP.
- Rewrites exacts et wildcards, avec réponses A/AAAA locales prioritaires.
- Filtrage de domaines et CNAME, règles DNS AdGuard courantes, exceptions et mises à jour atomiques.
- Cache des réponses upstream avec TTL décroissants, cache négatif SOA et budget mémoire configurable.
- Configuration exclusivement TOML et erreurs de démarrage explicites.
- Commandes CLI locales : purge du cache, rechargement TOML et statistiques.
- Dockerfile multi-stage et Compose avec utilisateur non privilégié.
- Arrêt sur `SIGINT` (Ctrl+C) et `SIGTERM` sous Linux.

## Fonctionnement

Le traitement des requêtes DNS suit cet ordre :

1. Recherche d'une règle de réécriture DNS locale.
2. Application des règles de filtrage et des réécritures de listes.
3. Recherche d'une réponse dans le cache.
4. Transmission aux serveurs DNS externes.

Les rewrites sont consultés en premier, puis les filtres, sans accès réseau. Un domaine bloqué reçoit `NXDOMAIN`, sans enregistrement, quel que soit le type demandé. Les exceptions autorisent la poursuite du traitement. Après une absence dans le cache, les requêtes récursives sont transmises aux upstreams dans l'ordre configuré. Une erreur réseau, un timeout, une réponse invalide, `SERVFAIL` ou `REFUSED` déclenche l'essai suivant. Les erreurs de transport écartent le serveur pendant 5 secondes pour le protocole concerné ; son retour est sondé en arrière-plan lorsque le trafic reprend. Les réponses `SERVFAIL`/`REFUSED` ne déclenchent pas cette mise à l'écart. Si tous échouent, le client reçoit `SERVFAIL`. Les réponses admissibles sont conservées en mémoire, avec diminution des TTL à chaque lecture et expiration sans prolongation.

Les clients TCP utilisent un upstream TCP. Pour UDP, une réponse upstream tronquée déclenche un nouvel échange TCP. Les réponses UDP respectent 512 octets sans EDNS, ou la taille annoncée par le client plafonnée à 1232 octets avec EDNS ; une réponse trop grande porte le drapeau `TC` pour permettre au client de réessayer en TCP. EDNS(0) et son indicateur DNSSEC `DO` sont pris en charge ; les autres options EDNS ne sont pas relayées. La validation DNSSEC reste du ressort de l'upstream.

Les rewrites ont un TTL de **60 secondes**. Une règle IPv4 répond aux requêtes A, une règle IPv6 aux requêtes AAAA ; ANY retourne l'enregistrement disponible. Un nom connu sans enregistrement du type demandé répond `NOERROR` avec une réponse vide (`NODATA`), sans rechercher une règle moins précise ni contacter un upstream. Le nom demandé est conservé dans la réponse, y compris pour un wildcard.

Les règles suivent les priorités DNS de la [RFC 4592](https://www.rfc-editor.org/rfc/rfc4592.html#section-3.3.1) :

- Un nom exact est prioritaire. La casse et le point final ne changent pas la correspondance.
- Un wildcard `*.lan` couvre les descendants comme `nas.lan` ou `a.b.lan`, mais pas `lan` lui-même. Une règle explicite `"lan" = "…"` peut renseigner cet apex.
- Seul le wildcard de l'ancêtre existant le plus proche peut s'appliquer. Ainsi, avec `*.lan` et `*.dev.lan`, `app.dev.lan` utilise la seconde règle. Un nom exact `router.lan` empêche `*.lan` de répondre pour `child.router.lan`.
- Dans une zone locale déclarée par wildcard, les parents d'un nom configuré existent aussi : avec `*.lan` et `deep.host.lan`, `host.lan` est un nom sans adresse. Il répond `NODATA` ; ses descendants non configurés répondent `NXDOMAIN` si aucune règle wildcard plus précise ne les couvre.

Un wildcard déclare sa zone comme locale : son apex et tous ses descendants restent hors du forwarding, y compris en cas de réponse négative. Une règle exacte seule réserve uniquement son nom ; elle ne transforme pas sa zone parente en zone locale. Par exemple, `service.example.com` ne bloque pas `other.example.com`.

Ces réponses sont des substitutions locales : les drapeaux `AA` et `AD` restent désactivés, sans SOA ni validation DNSSEC locale. Les réponses négatives locales sans SOA ne fournissent pas de TTL de cache négatif ([RFC 2308](https://www.rfc-editor.org/rfc/rfc2308.html#section-5)). Les réponses locales et les réponses déjà en cache fonctionnent aussi sans récursion (`RD=0`) ; une absence dans le cache est alors refusée. Les classes locales autres que IN, les transferts de zone et les requêtes signées TSIG sont refusés ; les opérations autres que QUERY répondent `NOTIMP`.

Le nombre de requêtes simultanées vers les upstreams est limité à 256 ; un dépassement répond `SERVFAIL`. Les réponses locales et les lectures du cache restent disponibles à cette limite. Ce plafond reste commun aux générations de configuration et inclut les sondes de reprise. Les connexions TCP inactives sont fermées après 10 secondes. Les journaux vont sur stderr, sans journalisation de chaque requête par défaut.

## Configuration

Exemple de fichier `config.toml` :

```toml
[dns]
listen = "127.0.0.1:1053"
upstreams = ["1.1.1.1:53", "9.9.9.9:53"]
# Facultatif : délai par upstream, repli TCP compris.
upstream_timeout_ms = 2000

[rewrites]
"*.lan" = "192.168.1.10"
"router.lan" = "192.168.1.1"
"nas.lan" = "192.168.1.20"
"nas6.lan" = "fd00::20"

[cache]
enabled = true
max_entries = 4096
max_memory_bytes = 8388608
max_ttl_secs = 86400
```

Le fichier fourni écoute uniquement sur `127.0.0.1:1053`. Pour servir le LAN sans Docker, choisir explicitement une adresse LAN ou `0.0.0.0:1053` et autoriser le port UDP/TCP dans le pare-feu.

Le fichier `config.toml` est lu depuis le répertoire de travail, ou sélectionné avec `--config CHEMIN`. Les chemins relatifs des listes restent relatifs au répertoire de travail. Les sections `[dns]` et `[rewrites]`, ainsi que `listen` et `upstreams`, sont obligatoires ; `[rewrites]` peut être vide. Les champs inconnus sont refusés.

- **Adresses DNS :** IP et port obligatoires, avec un port entre 1 et 65535. IPv6 doit être entre crochets, par exemple `[::1]:5353`. Les noms d'hôtes ne sont pas acceptés comme endpoints.
- **Upstreams :** au moins une adresse ; les adresses non spécifiées (`0.0.0.0`, `::`), multicast et broadcast IPv4 global (`255.255.255.255`) sont refusées, y compris sous forme IPv6 mappée. Les adresses privées et loopback sont acceptées. L'écoute peut utiliser `0.0.0.0` ou `::`.
- **Timeout :** `upstream_timeout_ms` est facultatif, vaut 2000 ms par défaut et accepte 1 à 60000 ms. Le délai total peut atteindre ce délai multiplié par le nombre d'upstreams. Les boucles évidentes vers l'adresse d'écoute sont refusées ; un upstream doit désigner un autre service DNS.
- **Noms des rewrites :** ASCII ou punycode (`xn--…`), avec underscores autorisés. Le point final est facultatif. Les noms vides, la racine, les échappements DNS et les noms dépassant les limites DNS sont refusés. Hickory valide leur syntaxe.
- **Wildcards :** seul un préfixe `*.` suivi d'un domaine est accepté, par exemple `*.lan`. Les noms sont normalisés en minuscules avec un point final ; deux règles équivalentes, comme `NAS.LAN` et `nas.lan.`, sont refusées.
- **Destinations des rewrites :** une seule adresse IPv4 ou IPv6 par règle, sans port ni crochets. Dans cette section, un même nom ne peut pas publier les deux familles simultanément ; les règles `$dnsrewrite` des listes permettent de réunir A et AAAA.

La validation de configuration ne contacte pas les upstreams. Appliquer les changements avec `light-dns reload` ; une configuration invalide conserve l'état actif. Un changement de `dns.listen` nécessite un redémarrage.

## Filtrage et listes par défaut

Le filtrage est actif par défaut, avec **[AdGuard DNS filter](https://github.com/AdguardTeam/HostlistsRegistry/tree/main/filters/general/filter_1_DnsFilter)** (dont EasyList/EasyPrivacy adaptées au DNS) et **[Peter Lowe’s](https://pgl.yoyo.org/adservers/)** (également utilisée par uBlock Origin).

Les sources, fichiers locaux, exceptions et délais sont éditables dans `[filtering]` de `config.toml`. `enabled = false` désactive tout filtrage ; `sources = []` conserve uniquement les fichiers et exceptions locaux. Les formats domaines et hosts ainsi que les règles DNS AdGuard courantes sont reconnus : motifs avec jokers, exceptions, `$dnstype`, `$denyallow`, `$dnsrewrite`, `$important` et `$badfilter`. Les regex, règles par appareil et filtres de navigateur restent ignorés et comptabilisés. Les CNAME liés à la question sont contrôlés aussi sur les réponses du cache. Les `$dnsrewrite` des listes distantes sont ignorés par défaut, exceptions et `$badfilter` associés compris ; une source HTTPS de confiance peut les autoriser explicitement avec `allow_dnsrewrite = true`. Les rewrites TOML et ceux des fichiers locaux configurés restent autorisés.

Les fichiers et le cache disque sont préchargés. Les téléchargements et mises à jour se font en arrière-plan avec remplacement atomique de l'index. **Au premier démarrage sans cache, les listes distantes deviennent actives après leur chargement.** Une panne conserve la dernière version valide ; sans version disponible, le DNS continue avec les autres règles et un avertissement.

Voir la [configuration détaillée du filtrage](docs/filtrage.md) pour les formats, priorités, limites et exemples TOML.

## Cache DNS et performances

Le cache est actif par défaut et se règle dans `[cache]` : 4096 entrées, budget pondéré de 8 Mio et conservation plafonnée à 24 heures. Ce budget estime le poids des entrées ; il ne borne pas strictement la RAM totale du processus. `enabled = false` désactive le cache. Les filtres sont consultés avant chaque lecture, y compris après une mise à jour des listes.

Les réponses complètes peuvent être réutilisées entre UDP et TCP, avec adaptation à chaque client. Les réponses négatives exigent un SOA pertinent ; leur durée suit le minimum du TTL du SOA et de son champ MINIMUM. Les erreurs, TTL nuls et réponses contenant des signatures DNSSEC restent hors cache. Aucun historique de requêtes n'est écrit sur disque.

Voir les [règles et limites du cache](docs/cache.md) et le [benchmark local reproductible](docs/performance.md), incluant latence, CPU et mémoire.

## Commandes locales

Depuis le même répertoire et avec le même utilisateur que le serveur :

```bash
light-dns                 # Démarrer ; commande serve également disponible
light-dns cache clear     # Remplacer le cache actif par un cache vide
light-dns reload          # Relire le fichier TOML utilisé au démarrage
light-dns stats           # Compteurs et état du cache, au format TOML
light-dns health          # Échanges DNS UDP/TCP locaux, sans dépendre d’Internet
light-dns health --upstream # Ajouter une résolution directe de la racine DNS
```

Avec Cargo, utiliser par exemple `cargo run -- stats`. L'administration utilise une socket Unix `light-dns.sock` protégée en mode `0600`, sans port réseau. Un verrou compagnon `light-dns.sock.lock`, conservé sur disque, permet de récupérer une socket abandonnée après un crash sans retirer une socket active. Le dossier de contrôle doit appartenir au service ou à root et ne pas être modifiable par le groupe ou les autres ; ses ancêtres doivent rester maîtrisés. `--socket CHEMIN` sélectionne un autre chemin pour le serveur et ses clients. Aucun fichier TOML n'est nécessaire pour lancer `stats` ou demander un reload ; le serveur conserve le chemin du fichier à relire.

Le rechargement prépare puis remplace atomiquement rewrites, filtres, upstreams et paramètres de cache. Une réussite vide le cache DNS ; un échec conserve la configuration et le cache actifs. Les requêtes en cours terminent avec leur ancien état et les connexions TCP restent ouvertes. Les compteurs persistent jusqu'au redémarrage. Il n'y a pas de surveillance automatique du TOML ; l'actualisation périodique des listes de filtres reste active.

Voir le [détail des commandes et statistiques](docs/administration.md).

## Docker

Adapter `docker/config.toml`, puis démarrer :

```bash
docker compose up -d --build
docker compose exec light-dns light-dns stats
docker compose exec light-dns light-dns cache clear
docker compose exec light-dns light-dns reload
```

Compose publie le DNS sur `127.0.0.1:53` en UDP et TCP par défaut. Pour servir le LAN, définir `LIGHT_DNS_BIND_IP` sur l'adresse LAN de l'hôte. La configuration est montée en lecture seule, les listes téléchargées persistent dans un volume et le service utilise l'UID/GID 65532.

Voir le [déploiement Docker](docs/docker.md) pour les ports, les montages, les tests et ARM64.

La CI utilise Forgejo Actions et son registre de conteneurs pour les images. Elle vérifie les pushs et pull requests ; la construction et la publication AMD64 sont déclenchées manuellement. `LIGHT_DNS_IMAGE` permet à Compose d’utiliser l’image publiée. Voir la [configuration Forgejo](docs/docker.md#forgejo-et-registre-dimages).

## Développement

Nécessite une installation de Rust et Cargo.

```bash
cargo run
```

Le programme valide la configuration, ouvre UDP et TCP puis reste actif jusqu'à son arrêt. Une configuration invalide ou l'échec d'une écoute produit une erreur sur stderr et un code de sortie non nul.

Avec le `config.toml` fourni, depuis un autre terminal disposant de `dig` :

```bash
dig @127.0.0.1 -p 1053 example.org A
dig @127.0.0.1 -p 1053 example.org AAAA +tcp
dig @127.0.0.1 -p 1053 router.lan A
dig @127.0.0.1 -p 1053 nas6.lan AAAA +tcp
```

L'arrêt se fait avec Ctrl+C ou `SIGTERM`. Les tests automatisés utilisent uniquement des upstreams factices locaux et ne nécessitent pas Internet.

Vérifications :

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo check --locked
```

Les tests Docker et les benchmarks sont ignorés par défaut ; leurs commandes d'exécution figurent dans la documentation dédiée.

## Organisation du code

```text
src/
├── main.rs                  # Point d'entrée du binaire
├── lib.rs                   # Modules de l'application
├── app/
│   ├── mod.rs               # Démarrage et cycle de vie
│   ├── cli.rs               # Arguments et commandes du binaire
│   ├── control/
│   │   ├── mod.rs           # Exécution des commandes locales
│   │   └── socket.rs        # Socket Unix, permissions et messages bornés
│   └── shutdown.rs          # Signaux d'arrêt
├── config/
│   ├── mod.rs               # Chargement TOML et types validés
│   ├── cache.rs             # Paramètres et validation du cache DNS
│   ├── error.rs             # Erreurs de configuration
│   ├── filtering.rs         # Paramètres et validation du filtrage
│   └── validation.rs        # Validation des adresses et noms DNS
├── dns/
│   ├── mod.rs
│   ├── cache/
│   │   ├── mod.rs           # Stockage pondéré et vieillissement des TTL
│   │   └── policy.rs        # Admissibilité et expiration des réponses
│   ├── forwarder.rs         # Échanges avec les upstreams
│   ├── rewrites.rs          # Index des noms locaux et priorités wildcard
│   └── server/
│       ├── mod.rs           # Écoutes UDP/TCP et cycle de vie du serveur
│       ├── handler.rs       # Traitement des requêtes
│       ├── health.rs        # Sondes DNS locales et diagnostic upstream
│       ├── runtime.rs       # État partagé et rechargement atomique
│       ├── stats.rs         # Compteurs légers
│       └── response.rs      # Réponses DNS, EDNS et troncature
└── filtering/
    ├── mod.rs               # État partagé des filtres
    ├── parser.rs            # Formats DNS, hosts et sous-ensemble Adblock
    ├── index.rs             # Recherche des domaines et exceptions
    ├── download.rs          # HTTP borné et cache disque
    └── updater.rs           # Chargement et actualisation des listes
tests/
├── integration.rs           # Point d'entrée unique de la suite de tests
├── app/                     # CLI, contrôle local, démarrage et arrêt
├── config/                  # Chargement et validation
├── dns/                     # Cache, forwarding, rewrites et serveur
├── deployment/              # Test Docker explicite
├── filtering/               # Formats de listes, priorités, téléchargements et mises à jour
├── performance/             # Benchmark Linux de latence, CPU et mémoire
└── support/                 # Fixtures DNS et processus de test
```

Tous les tests fonctionnels résident dans `tests/` et suivent l'arborescence de `src/`. Ils utilisent la bibliothèque `light_dns` et les interfaces réseau du service ; les modules internes restent privés. Les fixtures communes sont regroupées dans `tests/support/`, et les mesures transversales dans `tests/performance/`.

Pour exécuter uniquement les tests du serveur DNS :

```bash
cargo test --test integration dns::server
```

## Licence

Ce projet est distribué sous licence [MIT](LICENSE).
