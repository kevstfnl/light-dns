# Cache DNS

Le cache conserve uniquement des réponses upstream admissibles, après les rewrites et les filtres. Ces derniers sont consultés à chaque requête : une nouvelle règle de blocage s'applique même si une réponse positive est déjà en cache. Les réponses locales et les blocages ne sont pas stockés. Le cache reste uniquement en mémoire et disparaît à l'arrêt.

```toml
[cache]
enabled = true
max_entries = 4096
max_memory_bytes = 8388608
max_ttl_secs = 86400
```

Cette section est facultative et ses valeurs par défaut sont celles de l'exemple. Les champs inconnus sont refusés. `enabled = false` désactive le stockage et les recherches sans créer de cache Moka.

| Paramètre | Limites acceptées | Effet |
| --- | --- | --- |
| `max_entries` | 1 à 65536 | Nombre visé d'entrées conservées |
| `max_memory_bytes` | 65536 à 1073741824 | Budget pondéré en octets estimés |
| `max_ttl_secs` | 1 à 604800 | Durée maximale de conservation interne |

## TTL et réponses négatives

Chaque entrée expire au plus petit TTL de tous ses enregistrements, plafonné par `max_ttl_secs`. Une lecture ne repousse jamais cette échéance. Les TTL des réponses, autorités et enregistrements additionnels diminuent selon l'âge monotone de l'entrée, arrondi à la seconde supérieure pour ne pas prolonger leur validité. Aucun TTL minimal n'est imposé et aucune réponse périmée n'est servie.

Le plafond de conservation interne ne modifie pas les TTL reçus de l'upstream. Une réponse dont un enregistrement expire tôt est retirée entièrement : les sections restantes ne sont pas recomposées en réponses partielles.

Une réponse négative `NXDOMAIN` ou `NOERROR` sans le type demandé exige un SOA d'une zone contenant le nom interrogé. Sa durée prend aussi en compte `min(TTL du SOA, SOA.MINIMUM)` et le TTL du SOA stocké est réduit à cette valeur, conformément à la [RFC 2308, section 5](https://www.rfc-editor.org/rfc/rfc2308.html#section-5). Le cache négatif reste limité au même nom, type et classe ; il n'est pas étendu aux descendants ni aux autres types.

Restent hors cache :

- Les erreurs autres que `NOERROR` et `NXDOMAIN`, les réponses tronquées et les classes autres que IN.
- Les enregistrements de TTL nul ou dont le bit de poids fort est actif ([RFC 2181, section 8](https://www.rfc-editor.org/rfc/rfc2181.html#section-8)).
- Les réponses négatives sans SOA pertinent, les références NS seules et les chaînes CNAME incomplètes sans SOA pertinent.
- Les réponses contenant RRSIG, SIG ou une signature TSIG. Les signatures DNSSEC ont aussi des échéances absolues : elles continuent à être relayées, sans être mises en cache à ce stade. La validation reste assurée par l'upstream.
- Les entrées dont le poids dépasse à lui seul le budget configuré.

## Partage et protocole

La clé distingue le nom insensible à la casse, le type, la classe, la présence EDNS et les indicateurs DO, CD et AD de la requête. L'identifiant client, RD, le transport et la taille UDP annoncée ne fragmentent pas le cache.

Une réponse complète peut servir aussi bien en UDP qu'en TCP ; le serveur restaure l'identifiant et la question de chaque client, adapte les indicateurs et reconstruit EDNS. Il applique ensuite les limites UDP habituelles et `TC` si nécessaire. Une réponse cache n'est pas annoncée comme autoritaire (`AA=0`). Les options ECS ne sont ni relayées ni conservées.

Une requête `RD=0` peut lire une réponse déjà disponible, mais une absence dans le cache reste refusée. Les lectures ne consomment pas de place dans la limite des 256 échanges upstream simultanés. Les absences simultanées peuvent encore déclencher plusieurs échanges upstream ; elles ne sont pas regroupées.

## Budget mémoire

Le stockage utilise **Moka**, déjà présent dans les dépendances, pour l'accès concurrent, l'expiration par entrée et l'éviction pondérée. Les réponses sont conservées sous forme DNS sérialisée par Hickory, puis décodées à la lecture ; aucune implémentation manuelle du protocole n'est ajoutée.

Le poids d'une entrée estime la réponse sérialisée, le nom et une marge fixe pour les structures internes. Un poids minimal égal à `ceil(max_memory_bytes / max_entries)` fait aussi participer le nombre d'entrées au budget. Une grosse réponse consomme donc davantage de capacité qu'une petite, tandis que beaucoup de petites réponses déclenchent l'éviction à la limite d'entrées.

L'éviction Moka est différée et sa [capacité est une cible à convergence](https://docs.rs/moka/0.12.16/moka/future/struct.Cache.html#maximum-capacity). De brefs dépassements sont possibles pendant la maintenance. Ce budget ne représente pas une limite stricte de RSS : allocateur, index Moka, filtres, buffers réseau et requêtes en vol consomment aussi de la mémoire.

`DnsServer::cache_stats().await` expose le nombre d'entrées et leur poids après maintenance pour les tests et mesures locales, sans ajouter d'API HTTP ni de métriques Prometheus. Voir le [benchmark de latence et de consommation](performance.md).

## Purge et rechargement

`light-dns cache clear` remplace le cache actif par un cache vide, sans changer la configuration ni remettre les compteurs à zéro. Les anciennes requêtes en vol peuvent encore terminer, mais elles ne peuvent pas repeupler ce nouveau cache. La purge concerne le serveur ; elle ne vide pas les caches des clients ou navigateurs.

Un `light-dns reload` réussi remplace également le cache, afin de ne pas réutiliser des réponses d'un ancien upstream ou d'anciens paramètres. Un reload rejeté laisse le cache intact. `light-dns stats` affiche notamment `cache_entries`, `cache_estimated_bytes` et `cache_hits`. Voir l'[administration locale](administration.md).
