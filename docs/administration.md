# Administration locale

Les paramètres DNS restent exclusivement dans le TOML. La CLI permet de démarrer le service ou d'envoyer trois commandes au processus actif, par une socket Unix locale. Aucune API HTTP, aucun port d'administration et aucune surveillance automatique du TOML ne sont ajoutés.

```bash
light-dns --config /etc/light-dns/config.toml --socket /run/light-dns/control.sock serve
light-dns --socket /run/light-dns/control.sock cache clear
light-dns --socket /run/light-dns/control.sock reload
light-dns --socket /run/light-dns/control.sock stats
```

Sans option : fichier `config.toml`, socket `light-dns.sock` et commande `serve`. Les chemins relatifs sont résolus depuis le répertoire de travail du serveur. Le dossier de la socket doit déjà exister et être accessible en écriture. `--help` affiche les commandes. Avec Cargo, les arguments suivent `cargo run --`.

Les commandes administratives utilisent seulement `--socket` : elles fonctionnent même si le TOML sur disque est devenu invalide. `reload` relit le chemin choisi au démarrage ; passer un autre `--config` au client ne change pas ce chemin. Les succès vont sur stdout ; un échec produit une erreur sur stderr et un code de sortie non nul.

## Purger et recharger

`cache clear` remplace le cache DNS actif. Les insertions des requêtes déjà en vol restent attachées à leur ancienne génération et ne repeuplent pas le nouveau cache. La configuration, le cache disque des listes et les compteurs sont conservés. La purge ne concerne pas les caches DNS des autres appareils.

`reload` valide le TOML, prépare les index locaux et de filtrage, puis publie un nouvel état complet en mémoire. Rewrites, upstreams, timeout, filtres et paramètres de cache peuvent changer sans fermer les écoutes ou les connexions TCP existantes. Les requêtes en cours terminent avec l'ancien état. Le nouveau cache est vide ; les statistiques restent cumulées.

Un TOML invalide, une erreur de lecture, un fichier de règles locales invalide ou un changement de `dns.listen` rejette le reload complet, en conservant l'état et le cache actifs. Changer l'adresse ou le port d'écoute exige un redémarrage. Les téléchargements distants conservent leur fonctionnement habituel : chargement du cache disque disponible puis mises à jour en arrière-plan. Ajouter une source distante sans cache ne garantit pas son activation avant le retour de `reload`.

La préparation se fait avant le remplacement atomique ; elle peut temporairement nécessiter la mémoire des deux configurations. La compilation des listes et des rewrites s'effectue hors des workers DNS. Les commandes de modification sont sérialisées ; les requêtes DNS continuent sans verrou d'administration.

## Statistiques

`stats` produit un document TOML sans nom de domaine ni historique des requêtes :

| Champ | Signification |
| --- | --- |
| `uptime_secs` | Durée depuis le démarrage |
| `requests` | Requêtes parvenues au gestionnaire Hickory |
| `local_answers` | Réponses traitées par les rewrites, y compris les réponses locales négatives |
| `blocked` | Requêtes bloquées par les filtres |
| `cache_hits` | Réponses trouvées dans le cache DNS |
| `forwarded` | Requêtes ayant commencé un forwarding ; les bascules et le repli TCP ne sont pas comptés séparément |
| `errors` | Réponses dont le code diffère de `NOERROR` et `NXDOMAIN`, dont `SERVFAIL` et `REFUSED` |
| `reloads` / `cache_clears` | Rechargements réussis et purges depuis le démarrage |
| `filter_rules` | Nombre de règles de l'index actif |
| `cache_entries` | Nombre d'entrées après maintenance du cache |
| `cache_estimated_bytes` | Poids estimé des entrées, distinct de la RSS du processus |

Les compteurs sont atomiques, lus indépendamment ; une lecture pendant le trafic peut présenter de légers écarts entre totaux. Les paquets rejetés par Hickory avant le gestionnaire et les échecs d'envoi ne sont pas tous comptés. Les commandes administratives et leurs connexions ne comptent pas comme des requêtes DNS. Aucun compteur n'est persisté sur disque.

## Socket et arrêt

La socket est créée en mode `0600`. Le service vérifie également l'UID du client ; l'utilisateur du service et root peuvent l'administrer. Le dossier parent doit rester sous leur contrôle. Les commandes ont une taille bornée, un délai de lecture de 2 secondes, un délai d'exécution de 30 secondes et au plus 8 connexions traitées simultanément. Le client attend au plus 32 secondes ; la réponse est limitée à 16 Kio.

Une interruption de la connexion client ne garantit pas l'annulation d'une commande déjà reçue. Un timeout pendant la préparation du reload laisse l'ancien état actif ; aucune attente n'est introduite au milieu du remplacement effectif. Une perte de réponse après le remplacement peut laisser le client dans l'incertitude : consulter `stats` avant de réessayer si nécessaire.

`SIGINT` et `SIGTERM` annulent les commandes en cours, arrêtent les tâches de filtrage et ferment le serveur. La socket créée est retirée à la sortie normale, sans supprimer un chemin remplacé par un autre fichier. Un chemin préexistant n'est jamais effacé automatiquement. Après un arrêt forcé, vérifier que l'ancien serveur est arrêté, puis retirer manuellement sa socket résiduelle avant de redémarrer. Dans Compose, la socket réside sur un `tmpfs` recréé au démarrage.
