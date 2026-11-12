# Maintenance

## Vérifications

Lire les [règles de contribution](../AGENTS.md). Conserver les fixtures réseau locales, les interdictions d’`unsafe`, la CLI et le format TOML.

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked
cargo build --release --locked
```

Les tests de configuration valident les TOML natif, Docker et Alpine, ainsi que le petit exemple du README. Les tests réseau exigent des sockets UDP/TCP loopback, IPv6 et Unix autorisées. `EPERM` est un échec d’environnement, pas une validation ; ne pas l’ignorer ni contourner les restrictions.

Test explicite du conteneur, sur Linux AMD64 uniquement :

```bash
docker compose config --quiet
docker build --platform linux/amd64 --tag light-dns:validation .
cargo test --locked --test integration deployment::docker -- --ignored --nocapture
```

Ce test crée ses propres ressources temporaires. Ne pas le lancer contre des ressources de production. Le healthcheck Compose local n’exige pas Internet ; l’état `unhealthy` ne redémarre pas à lui seul le conteneur.

## Mesures reproductibles

Linux avec `/proc` et `getconf`. Construire les binaires avant de mesurer, sans build ni campagne concurrente. Les résultats restent sous `target/benchmarks/` : copier ce dossier avant un `cargo clean` pour les archiver.

```bash
cargo build --release --locked
cargo test --release --locked --lib --no-run
# Microbenchmarks Rust ignorés par défaut ; candidats uniquement dans les tests.
LIGHT_DNS_OFFLINE_DIR="$PWD/target/benchmarks/offline" cargo test --release --locked --lib offline:: -- --ignored --nocapture --test-threads=1
# Exemple : mesurer chaque variante cache dans un processus distinct.
LIGHT_DNS_OFFLINE_DIR="$PWD/target/benchmarks/cache-current" LIGHT_DNS_CACHE_VARIANT=current cargo test --release --locked --lib dns::cache::offline::cache_storage -- --ignored --nocapture --test-threads=1
LIGHT_DNS_OFFLINE_DIR="$PWD/target/benchmarks/cache-parsed" LIGHT_DNS_CACHE_VARIANT=parsed cargo test --release --locked --lib dns::cache::offline::cache_storage -- --ignored --nocapture --test-threads=1
```

Ces commandes produisent les CSV des harnais Rust ; elles ne remplacent pas l'ancienne orchestration Python : aucun manifeste, agrégat ni alternance de processus n'est généré automatiquement. Pour une campagne A/B, archiver le commit, l'état Git, le lockfile, les empreintes des sources/binaires et les versions des outils ; lancer cinq paires dans des dossiers distincts en alternant leur ordre. Ces expériences ne modifient pas le serveur. La borne EDNS ne couvre que A/AAAA/CNAME/SOA et garde la pré-sérialisation pour tout autre cas. Le cache pré-parsé conserve un message immuable, le clone au hit et vieillit les TTL ; son poids expérimental ne sait estimer que les fixtures A/SOA.

Les microbenchmarks incluent le coût du chronomètre et du harnais : p50/p95/p99 portent sur les 1000 dernières opérations, CPU et débit sur toutes les opérations pendant au moins 250 ms. Une opération cache concurrente est un lot de 32 ou 256 lectures, y compris les tâches Tokio, pas une latence DNS individuelle. Le CSV indique le nombre d’opérations ; multiplier par la taille du lot pour compter les lectures. Les expériences de conversion ASCII, de scratch `PatternSet` et de filtrage des identifiants en place isolent ces seuls coûts, pas un changement complet du moteur. Aucun compteur d’allocations n’est fourni.

La RSS des A/B cache dans un processus contient les deux variantes ; comparer plutôt les campagnes séparées. Les caches remplis partagent le budget configuré, mais peuvent retenir des nombres d’entrées différents : lire aussi les compteurs de capacité. VmHWM inclut les phases précédentes ; ne pas attribuer un pic global à une seule opération.

Campagnes DNS (loopback synthétique, pas le réseau réel) :

```bash
cargo test --release --locked --test integration performance::dns_cache -- --ignored --nocapture --test-threads=1
cargo test --release --locked --test integration performance::filtering::dns_filtering -- --ignored --nocapture --test-threads=1
cargo test --release --locked --test integration performance::forwarding::dns_forwarding -- --ignored --nocapture --test-threads=1
cargo test --release --locked --lib filtering::memory::filtering_memory -- --ignored --nocapture --test-threads=1
```

Le premier inclut client et upstream dans le processus mesuré ; les deux suivants isolent le démon. Filtrage : domaines simples/avancés, exceptions, rewrites, cache, CNAME, puis A/AAAA/NXDOMAIN avec EDNS, réponse près de 512 octets, troncature et TCP complet. Filtrage et forwarding couvrent 1/32/256 clients ; `LIGHT_DNS_BENCH_CONCURRENCY=1,32` réduit explicitement la campagne. Le filtrage réalise 50000 tentatives par cas et cinq essais ; le forwarding utilise 2 s d’échauffement et 10 s de charge par essai.

`LIGHT_DNS_BENCH_BINARY` et `LIGHT_DNS_BENCH_COMPARE_BINARY` sélectionnent deux binaires existants avec alternance ; `LIGHT_DNS_BENCH_CSV` choisit le CSV. Employer des chemins absolus. Les noms des variantes sont `current/before` pour le filtrage et `before/candidate` pour le forwarding. Les CSV séparent succès, erreurs et timeouts ; un fichier `*.error.txt` marque une campagne incomplète. Les percentiles ne portent que sur les succès ; le CPU par succès inclut le travail des échecs. Les mesures en boucle fermée ne reproduisent pas un trafic à débit d’arrivée imposé.

Le forwarding distingue la première bascule (`cold_failover_us`) du régime établi qui inclut les sondes. `LIGHT_DNS_BENCH_TIMEOUT_MS` change le timeout de sa fixture (défaut historique 100 ms). Comparer séparément 500, 750, 1000 et 2000 ms, en identifiant chaque CSV par ce paramètre. Les tests fonctionnels couvrent la reprise ; une mesure de reprise sous congestion réelle reste nécessaire.

Le défaut de production reste 2000 ms par upstream, UDP et fallback TCP compris. Un timeout plus court réduit l’attente de première panne mais augmente le risque d’exclure un upstream sain sur réseau congestionné. Les upstreams sont essayés séquentiellement ; l’exclusion dure 5 s par transport et les sondes partagent les ressources bornées. Ne pas déduire un timeout de production d’un upstream factice rapide.

Pour la mémoire de chargement : `LIGHT_DNS_MEMORY_DOMAINS=450000`, `LIGHT_DNS_MEMORY_CSV=chemin.csv`, ou `LIGHT_DNS_MEMORY_FILE=/corpus/fige.txt` sélectionnent un corpus. Conserver son SHA-256. Le pic de reload peut comprendre deux index et les structures de compilation ; le budget Moka n’est pas un plafond de RSS.

Conserver une optimisation seulement si le gain dépasse la dispersion des références et atteint 5 % dans quatre paires sur cinq, sans hausse répétée supérieure à 5 % des autres indicateurs ni erreur supplémentaire. Comparer CPU, p95/p99, débit réussi et mémoire. Sans campagne DNS autorisée, laisser le candidat hors du code de production et classer le résultat non concluant.

## Invariants à préserver

- Le serveur réserve OPT en cas de troncature UDP ; respecter la taille EDNS du client plafonnée à 1232, TC, les flags DNSSEC et le retour TCP complet. Ne jamais supprimer la pré-sérialisation sans borne sûre et mesures.
- Le cache sépare les contextes DNS pertinents, ne prolonge pas les TTL et rejette notamment les réponses partielles/signées non prises en charge. Les négatifs nécessitent un SOA de zone cohérent. Rewrites et filtres précèdent les hits ; purge/reload remplacent la génération pour empêcher un repeuplement tardif.
- Les rewrites exacts priment ; seuls les wildcards du closest encloser synthétisent une adresse. Un nom existant plus précis bloque un wildcard plus haut. Les zones wildcard, apex compris, ne sont jamais forwardées ; un type absent produit NODATA.
- Les formats domaines/hosts couvrent les descendants ; les modificateurs inconnus font ignorer toute la règle. Les exceptions prioritaires et l’allowlist utilisateur restent actives. Les exceptions `$dnsrewrite` concernent seulement les rewrites ; `$badfilter` compare l’identité complète de règle.
- Les rewrites TOML priment sur les listes. Les réécritures de listes restantes privilégient code/réponse vide, puis CNAME, puis union des RR ; l’ordre source/ligne départage les conflits. Les CNAME liés à la question sont contrôlés aussi au cache ; cycles et chaînes de plus de 16 liens sont refusés.
- Les fichiers locaux configurés sont de confiance pour `$dnsrewrite`. Les sources distantes l’interdisent par défaut, cache disque compris : une règle interdite est ignorée en entier, jamais convertie en blocage. `allow_dnsrewrite = true` exige HTTPS et accorde aussi confiance aux redirections HTTPS. Retirer l’autorisation puis recharger la révoque hors ligne. Aucun downgrade HTTPS vers HTTP n’est autorisé.
- Une erreur de téléchargement/compilation conserve le dernier index valide. Les fichiers distants sont privés (`0600`), liés à l’URL et remplacés atomiquement ; une sauvegarde échouée est réessayée. Ce mécanisme ne garantit pas la durabilité après coupure électrique. Nettoyer les temporaires abandonnés uniquement service arrêté.
- Le répertoire de contrôle est privé ; socket et verrou sont `0600`. Le verrou compagnon reste sur disque et ne doit pas être supprimé pendant le service. La récupération après crash préserve les chemins étrangers et les sockets actives. `SIGINT`/`SIGTERM` arrêtent proprement le service.

## Forgejo

`.forgejo/workflows/ci.yml` conserve la CI et la publication Docker Forgejo. Le runner `FORGEJO_CI_RUNNER` (défaut `ubuntu-latest`) doit être Linux AMD64 avec Git, Bash, Node.js 24, rustup, compilateur C, make, CMake, jq, tar, sha256sum, GNU install et CA ; le label ne provisionne pas cet environnement. Activer le service de cache du runner si utilisé.

La publication reste manuelle, sur une révision de confiance, avec `image_tag`. Le runner `FORGEJO_DOCKER_RUNNER` (défaut `docker-amd64`) exige Docker Linux AMD64 ; utiliser un moteur dédié. Configurer `FORGEJO_REGISTRY_USER` et le secret `FORGEJO_REGISTRY_TOKEN` limité à l’écriture des packages. Les PR n’ont pas besoin de ces secrets ni du moteur Docker.

L’image publiée est `<hôte-forgejo>/<propriétaire>/<dépôt>:<image_tag>` en minuscules ; un tag peut être remplacé. Pour la déployer, définir `LIGHT_DNS_IMAGE` dans `.env`, s’authentifier si nécessaire avec un compte de lecture, puis `docker compose pull` et `docker compose up -d --no-build`.

## Releases GitHub Alpine ARM64

`.github/workflows/release.yml` vérifie le projet sur Linux AMD64, compile vers `aarch64-unknown-linux-musl`, inspecte l'ELF sans l'exécuter et conserve l'archive en artefact. Les PR et les pushes sur `main` ne créent aucune release. Un push de tag `vX.Y.Z` correspondant à la version Cargo crée un brouillon ; `workflow_dispatch` permet de reconstruire un tag existant. Une release publiée n'est jamais remplacée par ce workflow.

Les actions sont figées par SHA. Les outils de compilation sont Rust 1.99.1, cargo-zigbuild 0.23.4, Zig 0.15.2 et CMake 4.4.5 ; les archives des trois derniers sont vérifiées par SHA-256. ShellCheck 0.11.0 est également figé et vérifié par SHA-256, afin d'utiliser la même version en local et en CI. Le jeton GitHub intégré suffit : seul le job de brouillon a `contents: write`.

Les huit tests de packaging sont intégrés à `cargo test --locked` sur Linux. Ils simulent les commandes système dans des dossiers temporaires, sans compte, paquet ni service réel. Ils nécessitent `sh`, `jq`, `tar`, `sha256sum` et GNU `install` sur l'hôte de test AMD64.

Reproduction depuis Linux AMD64 avec rustup, Git, jq, curl, tar, xz, binutils et les outils C/make :

```sh
rustup toolchain install 1.99.1 --profile minimal --target aarch64-unknown-linux-musl
sh scripts/setup-release-tools.sh "$PWD/target/release-tools"
export PATH="$PWD/target/release-tools/cargo-zigbuild:$PWD/target/release-tools/zig:$PWD/target/release-tools/cmake/bin:$PATH"
RUSTUP_TOOLCHAIN=1.99.1 RUSTFLAGS=-Dwarnings cargo zigbuild --release --locked --target aarch64-unknown-linux-musl
sh scripts/package-release.sh v0.1.0
cargo test --locked --test integration packaging::
shellcheck packaging/alpine/install.sh scripts/setup-release-tools.sh scripts/package-release.sh
sh -n packaging/alpine/light-dns.initd
actionlint .github/workflows/release.yml
```

Mettre à jour la version Cargo et le lockfile ensemble avant un nouveau tag. Committer les fichiers validés avant l'empaquetage : `REVISION` identifie le commit HEAD. L'archive et `SHA256SUMS` sont produits dans `target/release-assets/`. Les notes du brouillon viennent de `docs/release-notes.md`, complétées par le commit construit. Pour publier la première version, pousser le commit validé puis le tag `v0.1.0`, examiner le workflow et télécharger le brouillon avec `gh release download` (voir [installation et validation sur Pi](alpine.md)). Publier manuellement seulement après validation matérielle ; cette étape ne doit pas être déduite d'une compilation réussie.

Un tag publié ne doit pas être déplacé. Pour la reconstruction exceptionnelle du brouillon initial `v0.1.0`, vérifier son statut, sauvegarder sa référence et ses fichiers, attendre la fin des workflows de release, puis remplacer uniquement ce tag avec un lease sur sa référence distante vérifiée. Revérifier le statut brouillon avant remplacement et ne pas lancer de reconstruction concurrente. Les mises à jour suivantes utilisent un nouveau numéro de version.

Avec cette chaîne, Zig peut signaler `ignoring deprecated linker optimization setting '1'` à l'édition de liens. La compilation réussit et l'ELF est inspecté ; cet avertissement de l'outil reste visible et n'est pas une validation d'exécution ARM64. Clippy doit toujours rester sans avertissement.

ARM64, Alpine et Raspberry Pi restent une validation manuelle du mainteneur, avant remplacement d’AdGuard Home. Aucune mesure AMD64 ni fixture locale ne la remplace.
