# Audit final avant premier déploiement

Référence initiale : `8a04a59b2846090023c6cc73f59fb38feed2543a`. Exécution sur Linux AMD64, Rust 1.99.1. Les corrections sont implémentées ; **la validation préalable au remplacement d’AdGuard Home reste incomplète**, car l’environnement refuse les sockets réseau et l’accès au moteur Docker.

## Sécurité et compatibilité

Le compilateur de filtres acceptait les `$dnsrewrite` de toutes les listes sans distinction, y compris les listes téléchargées et les caches disque existants. Une liste compromise pouvait donc synthétiser des adresses ou des CNAME arbitraires.

La nouvelle option facultative `allow_dnsrewrite = true` autorise cette famille uniquement pour une source HTTPS explicitement choisie. Par défaut, la règle entière est exclue, exceptions et `$badfilter` associés compris. Elle ne devient pas un blocage. Les rewrites TOML gardent leur priorité ; les fichiers locaux explicitement déclarés restent de confiance. Les blocages et exceptions ordinaires, `$important`, `$badfilter`, conditions de type et désactivations entre listes sont préservés. La politique est réappliquée au cache disque sans migration, au téléchargement et au reload.

Les redirections conservent leurs limites et ne peuvent sortir de HTTP(S), transmettre des identifiants dans l’URL ou revenir à HTTP après HTTPS. La validation TLS et les limites de téléchargement restent actives. HTTP demeure accepté pour les listes ordinaires : son contenu peut être altéré en transit. HTTPS ne protège pas d’un éditeur compromis lorsqu’on lui accorde volontairement les rewrites.

Les configurations existantes restent lisibles ; leur comportement change volontairement pour les rewrites distants auparavant implicites. L’API Rust `FilterSource` possède désormais un champ `allow_dnsrewrite`. Aucune dépendance ni modification du lockfile n’a été ajoutée. La [politique détaillée](filtrage.md#confiance-des-réécritures-de-listes) précise les priorités et la révocation.

## Reprise après crash

Le verrou exclusif persistant, le contrôle UID/inode/périphérique, le refus de récupérer une socket active et le nettoyage conditionné par son identité sont conservés. Le dossier parent du contrôle est désormais vérifié : propriétaire service/root, sans écriture groupe/autres. Ses ancêtres doivent rester maîtrisés. Les verrous à liens multiples sont refusés. Un contrôle directement dans un dossier partagé tel que `/tmp` exige donc un sous-dossier privé.

Un problème de persistance a été corrigé : après réutilisation du PID, notamment PID 1 dans Docker, un temporaire abandonné pouvait reprendre le prochain nom de sauvegarde. La création exclusive essaie maintenant jusqu’à 32 noms, sans ouvrir ni supprimer un chemin préexistant ; une nouvelle tentative ultérieure reprend après ces noms. Les nouveaux fichiers Unix sont privés (`0600`). Le remplacement reste atomique et un échec conserve la destination. Les temporaires abandonnés ne sont pas purgés automatiquement ; inspecter leur accumulation service arrêté. Cela protège les crashes du processus, sans ajouter de garantie contre une coupure électrique.

Les tests internes ont vérifié l’exclusion entre ouvertures du verrou, sa réutilisation, les modes, liens, fichiers spéciaux, collision de temporaire, lien symbolique et échec de remplacement. Les tests processus existants et le nouveau scénario homelab couvrent `SIGKILL`, trois redémarrages successifs, reload avec connexion TCP ouverte, filtrage persisté, plusieurs clients et bascule vers un secondaire. **Leur exécution reste bloquée par `EPERM`**, contrairement aux tests internes sans socket.

## Performances : observations et décisions

Le handler, les index, le parser, les rewrites, Moka, les transports Hickory et la gestion Tokio ont été inspectés. Les candidats identifiés sont les allocations `PatternSet`/vecteurs et conversions du filtrage, les parcours d’ancêtres, copies de textes à la compilation, clonages du handler et décodage du cache. Les contrôles CNAME, TTL, générations de cache et limites de concurrence doivent rester intacts.

**Aucune optimisation du chemin DNS n’a été retenue.** Aucun provider partagé, pool maison, nouveau cache ou réglage arbitraire Tokio n’est introduit. Les corrections de sécurité ne sont pas présentées comme des optimisations.

Les benchmarks disposent maintenant de p99, mesures du démon séparé, débit réussi, erreurs/timeouts distincts, maximum mémoire global et première bascule avant échauffement. Les campagnes avec erreurs sont explicitement signalées comme échouées ou incomplètes.

| Campagne DNS avant → après | p50 | p95 | p99 | CPU/réponse | RSS démon | Débit |
| --- | --- | --- | --- | --- | --- | --- |
| Rewrites exact/wildcard, blocages, allowlist, cache, UDP/TCP, fallback, failover, CNAME, concurrence | non mesuré | non mesuré | non mesuré | non mesuré | non mesurée | non mesuré |

Les trois tentatives release échouent dès les fixtures loopback, avant toute réponse mesurée. Les résultats historiques ne sont pas recyclés en validation de cette version. Les effets des sondes sur p95/p99, la concurrence soutenue et l’absence de fuite de ressources du démon restent à vérifier.

La seule comparaison de consommation achevée concerne le chargeur de filtres, **sans démon DNS** : cinq A/B alternés par corpus, module de mesure identique, un chargement et cinq reconstructions par processus. Les références faibles confirment la destruction des anciens index dans les 20 exécutions retenues.

| Corpus | CPU chargement ms avant → après | RSS chargée Kio avant → après | Maximum RSS/HWM Kio avant → après | RSS après 5 reconstructions Kio avant → après |
| --- | ---: | ---: | ---: | ---: |
| 100000 domaines + 1000 motifs | 70 → 70 | 36084 → 37188 | 76648 → 77764 | 75612 → 76728 |
| 450000 domaines + 1000 motifs | 340 → 350 | 100184 → 100824 | 242172 → 251900 | 151328 → 161188 |

Le surcoût observé sur le grand corpus est d’environ 4 % au maximum et 6,5 % de RSS finale. Sa cause précise n’est pas établie ; les anciens index sont libérés, mais la RSS reflète aussi la rétention de l’allocateur. Ces résultats ne prouvent ni gain ni absence universelle de régression. Le pic de reload doit être dimensionné avec le corpus réel et les autres consommateurs du démon. Aucun comptage d’allocations n’est disponible. Méthode, temps et références figurent dans [performance.md](performance.md#audit-final--mesures-disponibles-et-limites).

## Vérifications exécutées

| Vérification | Résultat actuel |
| --- | --- |
| `cargo fmt --all -- --check` | Réussi |
| `cargo clippy --all-targets -- -D warnings` | Réussi |
| `cargo test --locked` | 13 tests internes + 25 intégrations réussis ; 87 intégrations échouent avec `EPERM` ; 5 tests ignorés au total |
| `cargo check --locked` | Réussi |
| `cargo build --release --locked` | Réussi |
| Benchmarks réseau release | Cache, filtrage A/B et forwarding A/B interrompus par `EPERM`, aucune mesure DNS |
| Comparaison mémoire release | 20 exécutions A/B retenues réussies, cinq essais par version et corpus |
| `docker compose config --quiet` | Réussi |
| Test Docker AMD64 explicite | Échec d’accès au moteur ; build fonctionnel non exécuté |
| ARM64/Raspberry Pi/Alpine | Non exécuté, réservé au mainteneur |

Les nouvelles intégrations couvrent aussi la révocation de confiance après reload, les mises à jour distantes sans rewrite, la reprise hors ligne, un timeout TCP avec clients concurrents et une panne simultanée laissant cache/rewrites disponibles. Elles compilent sous Clippy mais ne sont pas validées à l’exécution. Les erreurs réseau étaient déjà présentes sur la référence initiale : 82 `EPERM` avant ajout des cinq nouveaux tests réseau. Aucune régression DNS ne peut être exclue à partir de ces échecs environnementaux.

L’arbre de dépendances a été inspecté sans mise à jour. Le cas ciblé [RUSTSEC-2026-0119](https://rustsec.org/advisories/RUSTSEC-2026-0119) est corrigé depuis Hickory 0.26.1 ; le lockfile utilise 0.26.3. Cela ne constitue pas un audit exhaustif : `cargo-audit` est absent et la requête groupée OSV du lockfile échoue sur la résolution DNS de l’environnement. Ne pas conclure à l’absence de toute vulnérabilité transitive.

## Avant de remplacer AdGuard Home

- Réexécuter toute la suite réseau sur un Linux AMD64 autorisé, puis les campagnes p50/p95/p99 aux concurrences 1/32/256 ; analyser toutes les erreurs et ressources sous panne/reload.
- Construire et tester une image Docker AMD64 dédiée, y compris healthcheck UDP/TCP, persistance, `SIGKILL` et arrêt normal. Les validations Docker antérieures ne couvrent pas ces changements.
- Terminer l’audit des dépendances contre une base d’avis à jour.
- Mesurer mémoire, CPU et latence avec les listes réelles, particulièrement au démarrage/reload ; investiguer le surcoût observé et vérifier la marge RAM.
- Faire valider manuellement ARM64/Raspberry Pi/Alpine par le mainteneur, puis tester le LAN IPv4/IPv6, DHCP, zones locales et exceptions utilisateur. L’image Debian actuelle est conservée.
- Précharger/vérifier les listes avant la bascule DHCP : le premier démarrage sans cache conserve le comportement existant de chargement distant en arrière-plan. Prévoir un retour simple vers le résolveur précédent.

Les références, CSV, diagnostics et manifestes sont conservés localement dans `target/benchmarks/final-audit/`. Aucun push, publication, workflow GitHub, changement de service ou validation ARM64 n’a été effectué. Forgejo Actions et le déploiement non privilégié restent en place.
