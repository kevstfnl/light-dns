# light-dns

Un serveur DNS léger et sans interface graphique, développé en Rust pour les homelabs et les environnements à ressources limitées.

Configuration simple et faible consommation de ressources. Le déploiement Docker est prévu.

> **Statut :** Serveur DNS UDP/TCP avec forwarding, rewrites locaux A/AAAA et configuration TOML validée. Le filtrage et le cache restent à développer.

## Fonctionnalités disponibles

- Écoute UDP et TCP sur la même adresse, en IPv4 ou IPv6.
- Forwarding vers les upstreams configurés, avec timeouts, bascule et repli TCP.
- Rewrites exacts et wildcards, avec réponses A/AAAA locales prioritaires.
- Configuration exclusivement TOML et erreurs de démarrage explicites.
- Arrêt sur `SIGINT` (Ctrl+C) et `SIGTERM` sous Linux.

## Fonctionnalités prévues

- **DNS Filtering** — Blocage des publicités, trackers et domaines malveillants via des blocklists.
- **DNS Cache** — Mise en cache des réponses pour réduire la latence.
- **Docker** — Déploiement léger avec Docker Compose.

## Fonctionnement

Le traitement prévu des requêtes DNS suit cet ordre :

1. Recherche d'une règle de réécriture DNS locale.
2. Vérification des listes de blocage.
3. Recherche d'une réponse dans le cache.
4. Transmission aux serveurs DNS externes.

Actuellement, les règles locales sont consultées en premier, sans accès réseau. Les requêtes récursives restantes sont transmises aux upstreams dans l'ordre configuré. Une erreur réseau, un timeout, une réponse invalide, `SERVFAIL` ou `REFUSED` déclenche l'essai suivant. Si tous échouent, le client reçoit `SERVFAIL`. Les réponses négatives, les enregistrements et leurs TTL upstream sont conservés ; aucune réponse n'est mise en cache.

Les clients TCP utilisent un upstream TCP. Pour UDP, une réponse upstream tronquée déclenche un nouvel échange TCP. Les réponses UDP respectent 512 octets sans EDNS, ou la taille annoncée par le client plafonnée à 1232 octets avec EDNS ; une réponse trop grande porte le drapeau `TC` pour permettre au client de réessayer en TCP. EDNS(0) et son indicateur DNSSEC `DO` sont pris en charge ; les autres options EDNS ne sont pas relayées. La validation DNSSEC reste du ressort de l'upstream.

Les rewrites ont un TTL de **60 secondes**. Une règle IPv4 répond aux requêtes A, une règle IPv6 aux requêtes AAAA ; ANY retourne l'enregistrement disponible. Un nom connu sans enregistrement du type demandé répond `NOERROR` avec une réponse vide (`NODATA`), sans rechercher une règle moins précise ni contacter un upstream. Le nom demandé est conservé dans la réponse, y compris pour un wildcard.

Les règles suivent les priorités DNS de la [RFC 4592](https://www.rfc-editor.org/rfc/rfc4592.html#section-3.3.1) :

- Un nom exact est prioritaire. La casse et le point final ne changent pas la correspondance.
- Un wildcard `*.lan` couvre les descendants comme `nas.lan` ou `a.b.lan`, mais pas `lan` lui-même. Une règle explicite `"lan" = "…"` peut renseigner cet apex.
- Seul le wildcard de l'ancêtre existant le plus proche peut s'appliquer. Ainsi, avec `*.lan` et `*.dev.lan`, `app.dev.lan` utilise la seconde règle. Un nom exact `router.lan` empêche `*.lan` de répondre pour `child.router.lan`.
- Dans une zone locale déclarée par wildcard, les parents d'un nom configuré existent aussi : avec `*.lan` et `deep.host.lan`, `host.lan` est un nom sans adresse. Il répond `NODATA` ; ses descendants non configurés répondent `NXDOMAIN` si aucune règle wildcard plus précise ne les couvre.

Un wildcard déclare sa zone comme locale : son apex et tous ses descendants restent hors du forwarding, y compris en cas de réponse négative. Une règle exacte seule réserve uniquement son nom ; elle ne transforme pas sa zone parente en zone locale. Par exemple, `service.example.com` ne bloque pas `other.example.com`.

Ces réponses sont des substitutions locales : les drapeaux `AA` et `AD` restent désactivés, sans SOA ni validation DNSSEC locale. Les réponses négatives locales sans SOA ne fournissent pas de TTL de cache négatif ([RFC 2308](https://www.rfc-editor.org/rfc/rfc2308.html#section-5)). Les réponses locales fonctionnent aussi sans récursion (`RD=0`). Les autres requêtes sans récursion, les classes locales autres que IN, les transferts de zone et les requêtes signées TSIG sont refusés ; les opérations autres que QUERY répondent `NOTIMP`.

Le nombre de requêtes simultanées vers les upstreams est limité à 256 ; un dépassement répond `SERVFAIL`. Les réponses locales restent disponibles à cette limite. Les connexions TCP inactives sont fermées après 10 secondes. Les journaux vont sur stderr, sans journalisation de chaque requête par défaut.

## Configuration

Exemple de fichier `config.toml` :

```toml
[dns]
listen = "0.0.0.0:5353"
upstreams = ["1.1.1.1:53", "9.9.9.9:53"]
# Facultatif : délai par upstream, repli TCP compris.
upstream_timeout_ms = 2000

[rewrites]
"*.lan" = "192.168.1.10"
"router.lan" = "192.168.1.1"
"nas.lan" = "192.168.1.20"
"nas6.lan" = "fd00::20"
```

Le fichier `config.toml` est lu depuis le répertoire de travail. Les sections `[dns]` et `[rewrites]`, ainsi que `listen` et `upstreams`, sont obligatoires ; `[rewrites]` peut être vide. Les champs inconnus sont refusés.

- **Adresses DNS :** IP et port obligatoires, avec un port entre 1 et 65535. IPv6 doit être entre crochets, par exemple `[::1]:5353`. Les noms d'hôtes ne sont pas acceptés comme endpoints.
- **Upstreams :** au moins une adresse ; les adresses non spécifiées (`0.0.0.0`, `::`), multicast et broadcast IPv4 global (`255.255.255.255`) sont refusées, y compris sous forme IPv6 mappée. Les adresses privées et loopback sont acceptées. L'écoute peut utiliser `0.0.0.0` ou `::`.
- **Timeout :** `upstream_timeout_ms` est facultatif, vaut 2000 ms par défaut et accepte 1 à 60000 ms. Le délai total peut atteindre ce délai multiplié par le nombre d'upstreams. Les boucles évidentes vers l'adresse d'écoute sont refusées ; un upstream doit désigner un autre service DNS.
- **Noms des rewrites :** ASCII ou punycode (`xn--…`), avec underscores autorisés. Le point final est facultatif. Les noms vides, la racine, les échappements DNS et les noms dépassant les limites DNS sont refusés. Hickory valide leur syntaxe.
- **Wildcards :** seul un préfixe `*.` suivi d'un domaine est accepté, par exemple `*.lan`. Les noms sont normalisés en minuscules avec un point final ; deux règles équivalentes, comme `NAS.LAN` et `nas.lan.`, sont refusées.
- **Destinations des rewrites :** une seule adresse IPv4 ou IPv6 par règle, sans port ni crochets. Un même nom ne peut pas encore publier les deux familles simultanément.

La validation de configuration ne contacte pas les upstreams. Les changements de configuration nécessitent un redémarrage.

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
cargo test
cargo check
```

Le support Docker sera ajouté dans une prochaine version.

## Organisation du code

```text
src/
├── main.rs                  # Point d'entrée du binaire
├── lib.rs                   # Modules de l'application
├── app/
│   ├── mod.rs               # Démarrage et cycle de vie
│   └── shutdown.rs          # Signaux d'arrêt
├── config/
│   ├── mod.rs               # Chargement TOML et types validés
│   ├── error.rs             # Erreurs de configuration
│   └── validation.rs        # Validation des adresses et noms DNS
└── dns/
    ├── mod.rs
    ├── forwarder.rs         # Échanges avec les upstreams
    ├── rewrites.rs          # Index des noms locaux et priorités wildcard
    └── server/
        ├── mod.rs           # Écoutes UDP/TCP et cycle de vie du serveur
        ├── handler.rs       # Traitement des requêtes
        └── response.rs      # Réponses DNS, EDNS et troncature
tests/
├── integration.rs           # Point d'entrée unique de la suite de tests
├── app/                     # Démarrage du binaire et arrêt
├── config/                  # Chargement et validation
├── dns/                     # Forwarding, rewrites et server/{handler,response}
└── support/                 # Fixtures DNS et processus de test
```

Tous les tests résident dans `tests/` et suivent l'arborescence fonctionnelle de `src/`. Ils utilisent la bibliothèque `light_dns` et les interfaces réseau du service ; les modules internes restent privés. Les fixtures communes sont regroupées dans `tests/support/`.

Pour exécuter uniquement les tests du serveur DNS :

```bash
cargo test --test integration dns::server
```

## Licence

Ce projet est distribué sous licence [MIT](https://opensource.org/licenses/MIT).
