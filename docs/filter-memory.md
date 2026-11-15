# Mémoire du filtrage : diagnostic et vérification

## Mécanisme reproduit

Le 15 novembre 2026, sur Linux AMD64/glibc, un corpus local de 100000 domaines
simples et 1000 masques conditionnels reproduit une allocation de plusieurs Gio.
Les 1000 masques `||adN*.example^$dnstype=AAAA` produisent 2000 motifs (apex et
sous-domaines). Aucun téléchargement, serveur DNS ou cache Moka n'est nécessaire
à cette reproduction.

`regex-automata` 0.4.18 conserve par défaut les captures implicites de chaque
motif. Lorsque le DFA paresseux ne peut plus servir une recherche,
`meta::strategy::Core::which_overlapping_matches` se replie sur PikeVM.
`PikeVMCache::get` initialise alors le cache jusque-là absent. Ses deux ensembles
d'états possèdent chacun une `SlotTable`, dont `reset` alloue
`nombre_etats * slots_par_etat + slots_temporaires` éléments.
Sur une construction multi-motifs, les slots des captures implicites croissent
avec le nombre de motifs : la limite NFA de 10 Mio ne borne donc pas ce cache.
Le pool du moteur conserve les caches pour leur réutilisation ; plusieurs
threads peuvent posséder des caches distincts. Ce mécanisme explique une hausse
après le démarrage, déclenchée par les recherches, plutôt qu'à la compilation.

Le filtrage lit uniquement les identifiants renvoyés dans `PatternSet`.
`WhichCaptures::None` supprime les captures inutilisées et le produit
états × captures, tout en conservant `MatchKind::All`, les DFA rapides et le
repli PikeVM. Les fonctions renvoyant des positions de correspondance ne doivent
pas être utilisées sur cet automate sans revoir cette configuration.

Les mesures comparatives figurent plus bas. Elles établissent ce défaut sur le
corpus synthétique ; les listes exactes et le commit du Pi de l'incident n'ont
pas été fournis. Elles ne constituent pas une validation Alpine/ARM64.

## Ce que mesurent les instruments

- `filtering::memory::filtering_memory` utilise le chemin de compilation réel et
  le pool interne de recherche : chargement, repos, requêtes répétées, mélange de
  domaines variés correspondants/non correspondants, cinq rechargements, charge
  concurrente puis destruction. Les références faibles vérifient la destruction
  des anciennes générations. Il ne lance pas la tâche périodique de l'updater.
- `filtering::index::offline::regex_cache_growth` utilise un cache explicite dans
  un processus distinct, pour mesurer sa mémoire déclarée avant/après recherches
  et isoler le moteur du cache DNS, de Tokio et des générations d'index.
- Le test non ignoré `pikevm_cache_is_bounded_and_preserves_all_pattern_ids`
  désactive les deux DFA uniquement dans le test, impose un cache inférieur à
  16 Mio pour 2000 motifs et vérifie tous les identifiants attendus. Les autres
  tests couvrent les priorités, exceptions, conditions, réécritures, CNAME et
  quatre lecteurs conservant une ancienne génération pendant son remplacement.

La mémoire déclarée par `Regex::memory_usage` concerne les structures compilées ;
`Cache::memory_usage` concerne les structures suivies du cache explicite. Ces
valeurs ne constituent ni un comptage global des allocations, ni un plafond de
RSS. RSS et VmHWM sont échantillonnés via `/proc/self/status` ; le CPU est le temps
utilisateur+système du processus via `/proc/self/stat`. La différence après
destruction peut inclure la mémoire conservée par l'allocateur. VmHWM est un pic
cumulé du processus, pas celui de la seule phase courante. Les CSV du cache DNS
rapportent son poids estimé et ses entrées, pas toute la mémoire de Moka.

Les mises à jour des listes peuvent reconstruire et publier un index sans
incrémenter `reloads`, qui compte les rechargements explicites de configuration.
L'updater garde les derniers textes ; il ne reconstruit pas systématiquement à
chaque requête. Les instantanés ArcSwap en vol retiennent une ancienne génération
jusqu'à la fin de leurs lecteurs. Aucun changement de ces comportements, du
cache DNS ou de l'allocateur n'est inclus dans cette correction.

## Reproduction locale

Construire avant de mesurer, puis exécuter chaque harnais dans un processus
séparé, sans compilation ni autre campagne simultanée :

```sh
cargo build --release --locked
cargo test --release --locked --lib --no-run
LIGHT_DNS_MEMORY_WORKERS=1 LIGHT_DNS_MEMORY_CSV=target/benchmarks/memory-1.csv cargo test --release --locked --lib filtering::memory::filtering_memory -- --ignored --nocapture --test-threads=1
LIGHT_DNS_MEMORY_WORKERS=4 LIGHT_DNS_MEMORY_CSV=target/benchmarks/memory-4.csv cargo test --release --locked --lib filtering::memory::filtering_memory -- --ignored --nocapture --test-threads=1
LIGHT_DNS_OFFLINE_DIR=target/benchmarks/regex-memory cargo test --release --locked --lib filtering::index::offline::regex_cache_growth -- --ignored --nocapture --test-threads=1
```

Le harnais du pool accepte `LIGHT_DNS_MEMORY_QUERIES` (défaut : 100000 par phase
et par lecteur, de 1 à 10000000), `LIGHT_DNS_MEMORY_DOMAINS` (défaut : 100000,
maximum 450000), `LIGHT_DNS_MEMORY_MASKS` (défaut : 1000, de 0 à 1000) et
`LIGHT_DNS_MEMORY_WORKERS` (défaut : 4, de 1 à 4). Mettre les masques à zéro isole
les domaines simples ; mettre les domaines à zéro isole les masques. Au moins
une règle est nécessaire. Le repos initial est de 100 ms ; un repos de plusieurs
heures se mesure séparément sur le démon.

`LIGHT_DNS_MEMORY_FILE=/chemin/corpus.txt` remplace le corpus synthétique par un
fichier local figé (maximum 16 Mio) ; concaténer les deux listes avec des retours
à la ligne entre fichiers si nécessaire et conserver leurs SHA-256. Les noms
interrogés restent synthétiques : pour reproduire fidèlement un trafic réel,
rejouer un corpus DNS anonymisé contre le démon avec sa configuration exacte.
Le test de cache explicite conserve volontairement son corpus synthétique fixe.

Sur l'ancienne version, commencer avec un lecteur : quatre caches de plusieurs
Gio peuvent dépasser la RAM disponible. Une campagne interrompue ou ayant des
erreurs ne vaut pas validation. Ne pas exécuter simultanément les essais avant
et après. Les benchmarks DNS et leurs variables A/B sont documentés dans
[maintenance](maintenance.md#mesures-reproductibles).

## Mesures AMD64 du 15 novembre 2026

Référence : commit `918671a9125a34ad0122dc8d309c0fda8888bdd0`, Rust 1.99.1,
profil release, dépendances verrouillées. Les exécutables avant/après, CSV,
journaux et empreintes sont conservés sous
`target/benchmarks/filter-memory-fix/` (non suivis par Git).

| Mesure | Avant | Après |
| --- | ---: | ---: |
| Cache regex explicite, après 100000 recherches | 2740185692 octets | 3457013 octets |
| RSS du processus de cache explicite, fin de charge | 2691480 Kio | 18744 Kio |
| Pool réel : RSS après chargement de 101000 règles | 38592 Kio | 38132 Kio |
| Pool réel : RSS après 100000 recherches répétées puis 100000 variées | 2711836 Kio | 38240 Kio |
| Pic échantillonné du pool, cinq reloads et un lecteur compris | 2746552 Kio | 80232 Kio |
| RSS en fin de charge, médiane de cinq paires à 10000 recherches par phase | 2757116 Kio | 83184 Kio |
| CPU cumulé de ces cinq paires, médiane | 6320 ms | 4620 ms |

Un essai corrigé à quatre lecteurs et 600000 recherches au total reste sous
79840 Kio de pic échantillonné, avec 71116 Kio en fin de charge. Le contrôle
sans masques reste stable sous les recherches avant/après. Les références faibles
confirment la destruction des anciennes générations ; le RSS après destruction
peut rester supérieur au démarrage à cause de la rétention de l'allocateur.
La configuration sans captures ne cherche pas à imposer un retour artificiel du
RSS au minimum.

Le test de régression forcé échoue sur la référence avec un cache de
2738732496 octets, puis passe sur la correction. La suite fonctionnelle donne
156 réussites et 11 tests ignorés ; fmt, Clippy sans avertissement, check et build
release avec lockfile passent. Les harnais ignorés mémoire, filtrage hors réseau,
cache Moka hors réseau et DNS cache ont été exécutés séparément avec succès.

Les premiers microbenchmarks de filtrage, lancés séquentiellement avant puis
après, montrent des variations de CPU y compris sur les opérations inchangées
(ASCII, parcours des domaines simples). Ils ne suffisent pas à conclure sur une
régression ; les comparaisons DNS alternées sont nécessaires.

Le benchmark forwarding existant a échoué : réponses TCP invalides sur les deux
versions, filtrage désactivé, puis timeout de la fixture à quatre clients. Il
s'est arrêté après six lignes de résultats, sans achever les cinq essais. Son
CSV et son marqueur `dns-forwarding-ab.error.txt` sont conservés ; ces données
partielles ne valident ni les latences TCP ni le débit global. Aucun correctif
DNS/TCP extérieur au problème mémoire n'est inclus.


### Comparaisons DNS

La campagne isolée de onze scénarios termine ses cinq essais alternés à 1 et 4
clients : **11000000 requêtes réussies, aucune erreur**. La campagne complète
s’arrête avant ces cinq essais sur `edns_near` (réponse invalide dès
l’échauffement). Ce scénario isolé échoue également sur les deux versions ;
il reste une limite de validation, sans modification EDNS dans cette correction.

La dispersion de la première campagne motive une répétition sur quatre cœurs
physiques distincts autorisés (`taskset -c 0,1,2,3`) et avec
`TOKIO_WORKER_THREADS=4`, hérités par les démons. Cinq scénarios, concurrences 1
et 4, cinq essais alternés : **5000000 requêtes réussies, aucune erreur**.
Les valeurs suivantes sont les médianes des cinq essais, avant → après :

| Scénario | Clients | p50 µs | p95 µs | p99 µs | requêtes/s | CPU µs/requête |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| warm_cache | 1 | 10.65 → 10.08 | 14.27 → 13.01 | 20.02 → 16.37 | 90816 → 93100 | 5.20 → 5.20 |
| advanced_miss | 1 | 11.28 → 11.18 | 14.15 → 13.31 | 19.73 → 19.18 | 85176 → 86518 | 6.20 → 6.20 |
| advanced_hit | 1 | 10.61 → 10.48 | 14.35 → 13.44 | 19.20 → 17.58 | 89661 → 90375 | 5.60 → 5.40 |
| advanced_rewrite | 1 | 11.45 → 11.42 | 15.47 → 14.72 | 21.01 → 21.11 | 85704 → 85258 | 6.00 → 6.00 |
| warm_cache | 4 | 22.66 → 22.19 | 36.92 → 36.00 | 45.21 → 45.04 | 171221 → 176180 | 5.40 → 5.60 |
| advanced_miss | 4 | 25.13 → 23.95 | 40.53 → 39.75 | 49.70 → 50.89 | 156273 → 161757 | 6.60 → 6.20 |
| advanced_hit | 4 | 21.61 → 22.67 | 35.07 → 36.00 | 42.83 → 44.18 | 180086 → 174594 | 5.40 → 5.60 |
| advanced_rewrite | 4 | 22.54 → 22.51 | 36.34 → 36.83 | 46.30 → 45.60 | 172727 → 173112 | 5.60 → 5.60 |

Aucun de ces cinq scénarios sur quatre cœurs ne présente une dégradation de
latence, de CPU ou de débit supérieure à 5 % dans quatre paires sur cinq. Le
contrôle `simple_hit` à un client présente néanmoins une médiane p95 de
12,30 → 13,17 µs (+7,1 %), avec une seule paire dépassant 5 %. Son chemin de
recherche ne consulte pas l’automate de masques. Ces variations interdisent une
garantie absolue sur une machine partagée ; les tests fixés ne reproduisent pas
la hausse initiale du CPU observée dans les recherches avancées non fixées.
Les résultats complets, y compris les contrôles simples et les pics RSS, sont
dans `dns-filtering-selected.csv`, `dns-filtering-pinned.csv` et
`dns-summary.json` sous le dossier de mesures.

Pour répéter la campagne fixée après avoir conservé les deux exécutables :

```sh
TOKIO_WORKER_THREADS=4 LIGHT_DNS_BENCH_CONCURRENCY=1,4 LIGHT_DNS_BENCH_SCENARIOS=simple_hit,warm_cache,advanced_miss,advanced_hit,advanced_rewrite LIGHT_DNS_BENCH_BINARY=/chemin/apres LIGHT_DNS_BENCH_COMPARE_BINARY=/chemin/avant LIGHT_DNS_BENCH_CSV=target/benchmarks/fixe.csv taskset -c 0,1,2,3 cargo test --release --locked --test integration performance::filtering::dns_filtering -- --ignored --nocapture --test-threads=1
```

Choisir quatre cœurs autorisés sur l’hôte ; les numéros ci-dessus décrivent
uniquement l’essai AMD64. Construire au préalable pour éviter de mesurer pendant
une compilation. Ne pas assimiler cette affinité CPU à une émulation ARM64.

### Fichiers concernés

- `src/filtering/index.rs` : captures désactivées, configuration partagée avec
  les tests et accès diagnostique uniquement sous `cfg(test)` Linux.
- `tests/filtering/index_memory.rs` : allocation bornée du repli PikeVM,
  correspondances multiples, sémantiques avancées et générations concurrentes.
- `tests/performance/filter_memory.rs` et `filter_offline.rs` : charge du pool
  réel, contrôle de destruction et mesure séparée du cache regex explicite.
- `tests/performance/filtering.rs` et `measurement.rs` : réservation de ports
  avec la fixture UDP/TCP existante, sélection de scénarios et concurrence 4.
- `docs/maintenance.md`, ce rapport et `TODO.MD` : reproduction, résultats et
  validations restantes. Aucun format TOML, dépendance ou cache DNS modifié.

## Protocole Raspberry Pi 5 — mainteneur uniquement

1. Enregistrer le commit/version, `rustc -Vv`, le lockfile, le TOML, le nombre de
   workers, les empreintes des deux listes mises en cache, ainsi que la version
   Alpine, le noyau et l'allocateur. Conserver les mêmes fichiers et paramètres
   entre les essais ; utiliser des ports locaux non privilégiés pour la charge
   contrôlée et une fixture upstream locale.
2. Pour chaque version, démarrer sans trafic pendant une heure, puis rejouer la
   même séquence de domaines simples, masques et domaines absents : 10000 puis
   100000 requêtes, répétées et variées, avec concurrence 1 et 4. Comparer les
   réponses et mesurer latences p50/p95/p99, débit réussi, CPU et erreurs. Inclure
   UDP/TCP et IPv4/IPv6. Le contrôle cache désactivé est un essai diagnostique ;
   conserver le cache activé dans la configuration normale.
3. Toutes les minutes, relever dans `/proc/$PID/status` VmRSS, VmHWM, VmData et
   Threads, dans `/proc/$PID/stat` le CPU, et via `stats`/`cache status` les règles,
   requêtes, entrées et poids estimé du cache DNS. Conserver les journaux
   « index de filtrage chargé » pour repérer chaque publication automatique.
4. Prolonger l'essai corrigé au moins six heures, puis 24 heures pour couvrir un
   cycle de mise à jour par défaut. Effectuer aussi cinq `reload` explicites avec
   trafic et vérifier que les générations libérées ne provoquent pas une hausse
   cumulative. Tester un changement de liste, ses exceptions et ses CNAME.
5. Archiver les CSV, empreintes et journaux. Après échauffement, attendre un
   plateau mémoire à charge comparable, sans croissance liée au nombre total
   de requêtes. Tout dépassement répété de 5 % en CPU/latence ou perte de débit
   doit être investigué sur cinq essais alternés ; un hôte trop variable donne
   un résultat non concluant. Ne pas cocher la validation ARM64/Alpine/Pi avant
   ces vérifications réelles. Une garantie de stabilité sur 8 Go ne peut pas être
   déduite des seuls essais AMD64.
