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

Mesure réalisée le 7 novembre 2026 : Linux x86_64, AMD Ryzen AI 7 350, Rust 1.99.1, profil release du projet. Cache par défaut : 4096 entrées et budget pondéré de 8388608 octets. Deux exécutions ont réussi ; le tableau reprend la dernière, sans isolation des autres activités de la machine.

| Scénario | Médiane µs | p95 µs | Requêtes/s | CPU ms | RSS avant → après Kio | Entrées | Poids estimé octets | Échanges upstream |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Forwarding sans cache | 31,84 | 44,73 | 29988 | 170 | 6368 → 6436 | 0 | 0 | 5000 |
| Cache chaud | 11,62 | 13,54 | 84960 | 50 | 6556 → 6556 | 1 | 2048 | 0 |
| Rewrite local | 11,26 | 12,59 | 87553 | 50 | 6596 → 6596 | 0 | 0 | 0 |
| Domaine bloqué | 10,23 | 15,18 | 91120 | 60 | 7832 → 7872 | 0 | 0 | 0 |
| Cache sous pression | 35,28 | 53,42 | 26505 | 190 | 7972 → 10604 | 4096 | 8388608 | 5000 |

Dans cette mesure, le cache chaud divise la latence médiane par environ 2,7 et supprime les échanges upstream des lectures mesurées. Le scénario sous pression montre le coût des insertions et l'éviction au budget configuré.

## Limites d'interprétation

- La RSS et le CPU incluent le client et l'upstream factice, ainsi que les allocations conservées entre scénarios. Ce ne sont pas des mesures du démon isolé ni un pic mémoire.
- Les boucles sont courtes et les compteurs CPU ont la granularité des ticks Linux ; les différences de quelques millisecondes ne sont pas significatives.
- Un client séquentiel mesure une latence locale et son débit associé, pas la capacité maximale sous concurrence. Le réseau réel et les délais upstream ne sont pas simulés.
- Les filtres miniatures ne représentent pas la consommation des listes de production. Le budget pondéré du cache ne constitue pas une limite stricte de RSS.
- Ces résultats ne valent pas validation sur Raspberry Pi. Réexécuter la même commande sur Linux ARM64, puis mesurer le processus seul avec sa configuration réelle avant de dimensionner un déploiement.

Les mesures sont des observations, sans seuil automatique de vitesse : l'ordonnanceur, la fréquence CPU et la charge de la machine font varier les résultats. Conserver les CSV de plusieurs exécutions pour comparer des modifications sur le même matériel.
