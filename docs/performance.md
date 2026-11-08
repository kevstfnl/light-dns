# Mesurer la latence et la consommation

Un benchmark reproductible réside dans `tests/performance/`. Il utilise uniquement des sockets loopback, des filtres miniatures et un upstream factice local, sans Internet ni nouvelle dépendance Rust.

Sous Linux avec `/proc` et `getconf` disponibles :

```bash
cargo test --release --test integration performance::dns_cache -- --ignored --nocapture --test-threads=1
```

Le benchmark est ignoré par `cargo test` normal et refuse le mode debug. Il affiche les mesures et écrit `target/benchmarks/cache.csv`. Les tests fonctionnels du cache s'exécutent normalement avec `cargo test --test integration dns::cache`.

## Méthode

Chaque scénario effectue 100 requêtes d'échauffement, puis 5000 requêtes UDP séquentielles depuis une socket connectée. Deux workers Tokio exécutent le client, light-dns et l'upstream factice dans le même processus. Les téléchargements de filtres sont désactivés.

La latence mesure un échange client complet : encodage Hickory, aller-retour UDP et décodage. Le débit inclut aussi la préparation et la vérification des requêtes. Le CPU cumule les temps utilisateur et système de tous les threads, lus dans `/proc/self/stat` et convertis avec `getconf CLK_TCK`. La RSS est échantillonnée avant et après la boucle via `VmRSS` de `/proc/self/status`. Voir la documentation Linux de [stat](https://man7.org/linux/man-pages/man5/proc_pid_stat.5.html) et de [status](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html).

Les scénarios couvrent le forwarding sans cache, les lectures d'un cache chaud, les rewrites, le blocage local et 5000 noms distincts sous pression. Le benchmark vérifie les identifiants, codes DNS, échanges upstream et limites du cache après maintenance. Les mesures du cache comptent les entrées et leur poids estimé, sans les confondre avec la RSS.

## Mesure de référence

Mesure réalisée le 7 novembre 2026 : Linux x86_64, AMD Ryzen AI 7 350, Rust 1.99.1, profil release du projet. Cache par défaut : 4096 entrées et budget pondéré de 8388608 octets. Le tableau reprend la mesure après ajout de l'état partagé et des compteurs CLI, sans commande administrative concurrente ni isolation des autres activités de la machine.

| Scénario | Médiane µs | p95 µs | Requêtes/s | CPU ms | RSS avant → après Kio | Entrées | Poids estimé octets | Échanges upstream |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Forwarding sans cache | 31,13 | 43,62 | 30832 | 160 | 6692 → 6760 | 0 | 0 | 5000 |
| Cache chaud | 10,14 | 12,77 | 96233 | 50 | 6988 → 6988 | 1 | 2048 | 0 |
| Rewrite local | 8,39 | 11,40 | 110233 | 40 | 7052 → 7052 | 0 | 0 | 0 |
| Domaine bloqué | 9,90 | 12,67 | 98760 | 40 | 8240 → 8280 | 0 | 0 | 0 |
| Cache sous pression | 35,04 | 51,37 | 26802 | 200 | 8444 → 10948 | 4096 | 8388608 | 5000 |

Dans cette mesure, le cache chaud divise la latence médiane par environ 3,1 et supprime les échanges upstream des lectures mesurées. Le scénario sous pression montre le coût des insertions et l'éviction au budget configuré.

## Limites d'interprétation

- La RSS et le CPU incluent le client et l'upstream factice, ainsi que les allocations conservées entre scénarios. Ce ne sont pas des mesures du démon isolé ni un pic mémoire.
- Les boucles sont courtes et les compteurs CPU ont la granularité des ticks Linux ; les différences de quelques millisecondes ne sont pas significatives.
- Un client séquentiel mesure une latence locale et son débit associé, pas la capacité maximale sous concurrence. Le réseau réel et les délais upstream ne sont pas simulés.
- Les filtres miniatures ne représentent pas la consommation des listes de production. Le budget pondéré du cache ne constitue pas une limite stricte de RSS.
- Ces résultats ne valent pas validation sur Raspberry Pi. Réexécuter la même commande sur Linux ARM64, puis mesurer le processus seul avec sa configuration réelle avant de dimensionner un déploiement.

Les mesures sont des observations, sans seuil automatique de vitesse : l'ordonnanceur, la fréquence CPU et la charge de la machine font varier les résultats. Conserver les CSV de plusieurs exécutions pour comparer des modifications sur le même matériel.

## Audit du forwarding upstream

Hickory 0.26.3 ouvre une socket UDP à port source aléatoire par échange, même avec un client réutilisé : ce choix participe à la protection contre les réponses usurpées. Il n’est pas remplacé par une socket partagée. En TCP, le forwarder ouvre une connexion et un multiplexeur par échange. Le pool du resolver Hickory sait réutiliser et reconnecter ses transports, mais convertit notamment NXDOMAIN/NODATA en erreurs sémantiques en perdant une partie du message original. Son intégration directe ne préserverait pas toutes les réponses relayées aujourd’hui. Aucun pool maison n’est ajouté.

Une variante limitée réutilisant le `TokioRuntimeProvider` du forwarder est comparée à l’initialisation par échange. Cela supprime l’allocation répétée du handle (`Arc<Mutex<JoinSet>>`), sans réutiliser les sockets ni connexions. Seules les mesures permettent de décider de sa conservation.

### Protocole reproductible

```bash
cargo test --release --locked --test integration performance::forwarding::dns_forwarding -- --ignored --nocapture --test-threads=1
```

Le benchmark lance le binaire release dans un processus séparé, avec cache DNS et filtres désactivés. Le client et les upstreams factices restent dans le processus de test (quatre workers Tokio). Nagle est désactivé sur leurs connexions TCP pour éviter que les écritures séparées du préfixe de longueur et du message DNS ne mesurent surtout les ACK différés. Les workers poursuivent la charge après une erreur et reconnectent leurs connexions TCP si nécessaire. Chaque scénario utilise 1, 32 puis 256 clients, cinq essais, 2 secondes d’échauffement et au moins 10 secondes de mesure par essai (environ 12 minutes par version). Les scénarios sont UDP, TCP, UDP tronqué suivi de TCP, et premier upstream UDP silencieux suivi d’un upstream sain. Le timeout est de 100 ms par upstream.

`LIGHT_DNS_BENCH_CONCURRENCY=1` limite explicitement la campagne à un client ; `1,32` sélectionne deux niveaux. Le défaut reste `1,32,256`. Une campagne réduite ne valide pas les niveaux omis.

Le CSV `target/benchmarks/forwarding.csv` contient les latences médiane/p95 des réponses réussies, leur débit, les erreurs (y compris à l’échauffement), le CPU du démon total et par réponse réussie, sa RSS avant/maximum échantillonné toutes les 50 ms, et le nombre de connexions TCP acceptées par la fixture upstream. Ce maximum échantillonné n’est pas un pic mémoire garanti. Les réponses sont vérifiées (ID, question, code et enregistrements), et les erreurs ne sont jamais comptées dans le débit réussi.

`LIGHT_DNS_BENCH_BINARY` sélectionne un binaire existant ; `LIGHT_DNS_BENCH_COMPARE_BINARY` ajoute un second binaire avec alternance de l’ordre des versions entre essais ; `LIGHT_DNS_BENCH_CSV` sélectionne le CSV. Utiliser des chemins absolus pour les binaires. Construire et copier les deux versions avant de lancer la comparaison : ne pas lancer de build ni de test Docker pendant les mesures. La comparaison complète dure environ 24 minutes.

Les sockets loopback et la fixture peuvent aussi saturer sous 256 clients ; conserver les erreurs dans l’analyse plutôt que conclure à partir des seules latences réussies. Les buffers réseau de l’hôte ne sont pas modifiés. La capacité de cette boucle locale ne prédit pas celle d’un réseau réel. Le critère de conservation choisi est un gain d’au moins 5 % sur un indicateur de latence, débit ou CPU par réponse dans au moins quatre essais sur cinq, sans hausse répétée supérieure à 5 % des autres indicateurs ni augmentation des erreurs.

### Résultat de l’audit du 7 novembre 2026

La variante de provider partagé a été **retirée** : aucun gain reproductible n’a été démontré. Le code du forwarder reste identique à celui d’avant l’audit. La campagne complète prévue (cinq essais par cas) n’a pas pu aboutir dans cet environnement : plusieurs tentatives ont rencontré `Operation not permitted`, finalement localisé sur `fixture UDP send_to` à 256 clients. Le benchmark s’arrête sur cette erreur de permission au lieu de la compter comme une réponse DNS ou de publier une campagne réussie. Aucun réglage réseau de l’hôte n’a été modifié pour contourner ce refus.

Les seules mesures de la dernière tentative terminées avant ce refus sont ci-dessous (Linux AMD64, Rust 1.99.1, même profil release, un essai de 10 secondes après échauffement par ligne). Elles servent à documenter la calibration, **pas à conclure sur les performances** : elles ne satisfont ni les cinq répétitions ni l’ensemble UDP/TCP/fallback/failover. Aucun chiffre de gain n’est retenu.

| Version | Clients UDP | Médiane µs | p95 µs | Réponses réussies/s | CPU démon ms | RSS max échantillonnée Kio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Avant | 1 | 33,42 | 44,49 | 28641,67 | 7650 | 6380 |
| Provider partagé | 1 | 33,61 | 44,62 | 28518,29 | 7660 | 6312 |
| Avant | 32 | 237,19 | 461,70 | 122909,79 | 42670 | 12536 |
| Provider partagé | 32 | 234,81 | 449,13 | 124321,82 | 42250 | 12320 |

Aucune erreur n’est comptée dans ces quatre lignes. Le CPU cumule tous les threads : il peut dépasser le temps mural. Les essais de calibration antérieurs ont également révélé des pertes/timeouts sous forte concurrence ; un débit réussi doit toujours être lu avec ses erreurs. Si aucune réponse ne réussit, les latences et le CPU par réponse sont `NaN`.

Le CSV partiel reste dans `target/benchmarks/forwarding-comparison.csv`, avec les deux binaires et le patch expérimental `provider-candidate.patch` dans le même répertoire (artefacts locaux ignorés par Git). La campagne complète, notamment TCP, fallback et upstream indisponible, reste à reprendre dans un environnement autorisant cette charge loopback. Les tests fonctionnels de ces transports, du timeout et de la bascule passent indépendamment de ce benchmark.


## Campagne fiabilité et filtrage étendu

Le benchmark dédié lance un démon release séparé avec 100000 domaines locaux synthétiques et, pour les cas avancés, 1000 motifs typés plus une réécriture. Les fichiers n’utilisent pas Internet. Chaque scénario est répété cinq fois, avec 100 requêtes d’échauffement puis 50000 requêtes mesurées. Les réponses sont vérifiées (question, identifiant, code et présence des données attendues). Toute erreur arrête la campagne ; elle n’est pas masquée dans un débit réussi.

```bash
# Construire/copier le binaire de référence avant modification, puis construire la version courante.
cargo test --release --locked --test integration --no-run
LIGHT_DNS_BENCH_COMPARE_BINARY=/chemin/absolu/avant-light-dns cargo test --release --locked --test integration performance::filtering::dns_filtering -- --ignored --nocapture --test-threads=1
```

Le CSV `target/benchmarks/filtering.csv` contient médiane/p95, CPU total et par requête du démon seul, et RSS avant/après de ce processus. Les cas comparent les absences simples, blocages simples, cache chaud et CNAME en cache ; les cas nouveaux mesurent les motifs, réécritures et blocages CNAME. La référence n’exécute pas les fonctionnalités qu’elle ne possède pas. L’ordre avant/après alterne entre essais.

La RSS est celle du processus après compilation/échauffement et en fin de boucle ; elle inclut la mémoire gardée par l’allocateur, mais ne mesure pas le pic de chargement ni un reload avec deux index. Les corpus synthétiques ne représentent pas toutes les listes réelles. La mesure CPU conserve la granularité des ticks Linux. Aucun de ces essais AMD64 ne valide le Raspberry Pi.

Pour les scénarios existants, investiguer les hausses répétées dépassant 5 % ou 1 µs de latence, ou 5 % de CPU par réponse, avant de conclure. Les coûts des règles et contrôles nouveaux sont présentés séparément. Le benchmark de forwarding ci-dessus couvre UDP/TCP, repli et concurrence ; aucun build ou test Docker ne doit s’exécuter pendant les mesures.

### Résultats du 8 novembre 2026

Linux AMD64, Rust 1.99.1, profils release identiques, cinq essais alternés de 50000 réponses par cas, démon séparé. Le tableau donne la médiane des cinq médianes/p95 et des cinq mesures CPU/RSS ; il ne représente pas un percentile calculé sur toutes les exécutions réunies. Les 60 lignes du CSV final ont terminé sans erreur de réponse. Les anciens binaires, CSV et journaux sont conservés localement sous `target/benchmarks/`, ignoré par Git.

| Scénario | Médiane avant → après µs | p95 avant → après µs | CPU démon avant → après µs/réponse | RSS après Mio |
| --- | ---: | ---: | ---: | ---: |
| Absence simple → upstream | 38.60 → 38.77 | 55.49 → 57.05 | 30.40 → 30.80 | 22.02 |
| Blocage simple | 12.58 → 12.43 | 16.40 → 16.32 | 5.80 → 5.60 | 21.98 |
| Cache chaud | 13.57 → 13.50 | 17.96 → 17.78 | 6.40 → 6.40 | 22.10 |
| CNAME en cache, autorisé | 13.78 → 14.11 | 18.47 → 18.54 | 6.60 → 7.00 | 21.95 |
| Cache avec 1000 motifs actifs | — → 14.39 | — → 18.54 | — → 7.40 | 37.30 |
| Blocage par motif typé | — → 13.89 | — → 17.76 | — → 6.80 | 37.19 |
| Réécriture de liste | — → 14.13 | — → 17.94 | — → 6.80 | 37.19 |
| Cible CNAME bloquée | — → 39.12 | — → 57.75 | — → 31.20 | 22.11 |

La RSS du cas simple passe d’environ 21 à 22 Mio ; le cache chaud conserve le même CPU médian par réponse. Le contrôle CNAME ajoute ici environ 0,4 µs de CPU par réponse en cache : il s’agit du travail de filtrage nouvellement effectué. Les 1000 motifs avancés et leur compilation portent la RSS vers 37 Mio sur ce corpus ; le budget de cache DNS de 8 Mio ne borne pas cette mémoire.

Deux coûts ont été corrigés pendant la mesure : les conditions avancées sont allouées uniquement pour les règles concernées, et l’index simple est préalloué pour éviter de retenir les tables de croissance. Les noms simples gardent leur représentation compacte. La variante fondée uniquement sur GlobSet testait de nombreux motifs partageant une « extension » DNS séparément ; la version retenue compile explicitement leurs expressions dans un automate multi-motifs borné, sans ajouter de syntaxe regex utilisateur. Les CSV des variantes rejetées restent des diagnostics partiels, pas des campagnes validées.

Le benchmark historique cache/rewrites a aussi été exécuté cinq fois avant/après, en alternant les versions. Il inclut client et upstream dans le même processus : sa médiane de rewrite varie de 8,54 à 12,38 µs avant, et de 9,48 à 11,77 µs après ; les différences changent de signe entre essais et le CPU médian reste 50 ms pour 5000 réponses. Les médianes agrégées sont 9,32 → 10,42 µs : cette variation est conservée dans l’analyse, sans annoncer une absence universelle de surcoût. Le résultat du démon isolé avec corpus, ci-dessus, constitue une mesure distincte.

La campagne forwarding n’est **pas validée** : le premier passage a été interrompu par `EPERM` côté client pendant la comparaison à 32 clients. La tentative réduite à un client a également fini sur `EPERM`, après des réponses en erreur sous charge (notamment fallback/failover). Ces réponses ne sont pas comptées comme réussies. Les causes de tous les SERVFAIL précédant le refus système n’ont pas été établies ; aucun gain de débit, CPU ou de latence de bascule n’est retenu à partir de cette campagne. Les journaux `reliability-forwarding.log`, `reliability-forwarding-single.log` et leurs CSV partiels restent disponibles. Aucun paramètre réseau de l’hôte n’a été changé.

Les tests fonctionnels couvrent séparément les transports, la saturation, le timeout partagé, les upstreams silencieux, leur reprise et la disponibilité des réponses locales. Ils ne remplacent pas une campagne soutenue réussie. La validation à forte charge et la validation ARM64/Raspberry Pi restent ouvertes.
