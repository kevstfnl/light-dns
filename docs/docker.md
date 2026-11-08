# Déploiement Docker

Le Dockerfile utilise un build Rust séparé et une image finale Debian slim avec les certificats HTTPS nécessaires aux listes distantes. Le binaire s'exécute directement en PID 1 avec l'UID/GID `65532:65532`. Les ports internes sont `1053/udp` et `1053/tcp`, sans capacité privilégiée. La construction utilise `Cargo.lock` et ne copie ni configuration personnelle ni répertoire de données dans l'image.

## Démarrage avec Compose

Adapter les upstreams et les rewrites de `docker/config.toml`, puis :

```bash
docker compose config --quiet
docker compose up -d --build
docker compose logs light-dns
```

Les ports publiés par défaut sont `127.0.0.1:53`, en UDP et TCP. Pour le LAN :

```bash
LIGHT_DNS_BIND_IP=192.168.1.2 docker compose up -d --build
```

L'adresse doit appartenir à l'hôte. Le port hôte peut aussi être changé avec `LIGHT_DNS_PORT=1053`, notamment si le port 53 est déjà occupé. La publication par défaut est IPv4 ; ajouter une publication IPv6 adaptée au réseau et au moteur Docker si nécessaire. Le port interne reste 1053.

Compose monte le dossier `docker/` dans `/etc/light-dns` en lecture seule. Monter le dossier complet permet au serveur de voir aussi les sauvegardes d'éditeur faites par renommage atomique ; un montage de fichier unique peut rester attaché à l'ancien inode. Les éventuelles listes locales vont dans ce dossier, avec leurs chemins `/etc/light-dns/...` renseignés dans le TOML.

Les fichiers doivent être lisibles par l'UID 65532 et les dossiers traversables. Les téléchargements sont stockés dans le volume `filter-data` sous `/var/lib/light-dns/filters`. La racine du conteneur reste en lecture seule ; `/run/light-dns` est un `tmpfs` privé qui contient la socket de contrôle. Un volume déjà créé doit conserver des permissions adaptées à l'UID/GID 65532. Les options de montages, de publication et d'arrêt sont décrites dans la [référence Compose](https://docs.docker.com/reference/compose-file/services/).

Le filtrage distant fonctionne comme sur Linux : un premier démarrage sans cache disque peut répondre avant le chargement des listes. Les téléchargements HTTPS nécessitent une sortie réseau. Les rewrites des listes distantes restent désactivés par défaut, même lorsqu’elles sont préchargées dans le volume ; seule une source HTTPS avec `allow_dnsrewrite = true` peut les fournir. Les fichiers locaux montés explicitement sont de confiance pour les rewrites. Les nouveaux caches persistés sont créés en mode `0600` ; conserver le même UID pour les relire. Les chemins relatifs sont résolus depuis `/run/light-dns` ; privilégier les chemins absolus de l'exemple Docker.

## Commandes et arrêt

```bash
docker compose exec light-dns light-dns stats
docker compose exec light-dns light-dns cache clear
# Après modification de docker/config.toml :
docker compose exec light-dns light-dns reload
docker compose stop
```

La CLI utilise la socket locale du conteneur ; aucun port supplémentaire ni montage de socket sur l'hôte n'est nécessaire. Le contrôle de santé Compose appelle `health` : il vérifie le contrôle local et de vrais échanges DNS UDP/TCP, sans sonder un upstream Internet. `docker compose exec light-dns light-dns health --upstream` ajoute un diagnostic externe explicite. Un état Docker `unhealthy` ne redémarre pas à lui seul un conteneur ; `restart: unless-stopped` couvre les sorties du processus et les redémarrages du moteur.

Un reload réussi applique le TOML sans fermer les connexions DNS et remplace le cache. Un changement d'écoute nécessite un redémarrage ; pour conserver les publications Compose, garder le port interne 1053. Docker envoie `SIGTERM`, avec 10 secondes de grâce côté Compose et 5 secondes d'attente côté application. `docker compose stop` conserve le cache disque des filtres.

## Forgejo et registre d’images

La CI utilise Forgejo Actions dans `.forgejo/workflows/ci.yml`. Elle exécute les quatre commandes Rust sur les pushs et pull requests, sans secrets de registre ni benchmarks. Les téléchargements de dépendances sont nécessaires au premier build ; les tests fonctionnels utilisent seulement le réseau local.

Activer Actions dans le dépôt Forgejo et enregistrer les runners correspondants. La configuration vise une version récente de Forgejo Actions ; son exécution reste à valider sur l’instance cible. Les runners sont sélectionnés par les variables de dépôt suivantes :

| Variable | Défaut | Prérequis |
| --- | --- | --- |
| `FORGEJO_CI_RUNNER` | `ubuntu-latest` | Environnement Linux AMD64 avec Git, Bash, Node.js 24 pour checkout, rustup, compilateur C, make, CMake et certificats CA |
| `FORGEJO_DOCKER_RUNNER` | `docker-amd64` | Environnement Linux AMD64 avec Git, Bash, Node.js 24, client Docker et accès à un moteur Docker Linux AMD64 |

Ces labels doivent exister sur le runner ; ils ne provisionnent pas une machine Ubuntu. L’image minimale Node.js d’un runner Forgejo n’inclut pas forcément les outils Rust ci-dessus. Les actions checkout et cache sont référencées par des URL complètes, indépendamment de la forge configurée par défaut pour les actions. Le cache Cargo utilise le service de cache du runner, qui doit être activé. Les différences d’environnement sont détaillées dans la [documentation Forgejo Actions](https://forgejo.org/docs/latest/user/actions/github-actions/).

Configurer l’accès Docker du runner de publication selon la [documentation Forgejo](https://forgejo.org/docs/latest/admin/actions/docker-access/). Utiliser un environnement de build dédié, distinct du moteur hébergeant Forgejo et les services de production ; les tests de PR n’ont pas besoin d’un accès Docker.

La construction et la publication s’effectuent uniquement par lancement manuel du workflow depuis l’onglet Actions, avec le champ `image_tag` (par exemple `0.1.0`, ou `latest`). Choisir une révision de confiance. Les vérifications Rust doivent réussir avant la construction AMD64 et un test CLI `--help` sans réseau. Ce test rapide ne remplace pas la validation Docker fonctionnelle décrite plus bas. Aucune construction ARM64 n’est effectuée.

Pour la publication, ajouter aux paramètres Actions du dépôt :

- La variable `FORGEJO_REGISTRY_USER` : compte autorisé à publier dans l’espace du propriétaire du dépôt.
- Le secret `FORGEJO_REGISTRY_TOKEN` : jeton de ce compte, limité à l’écriture des packages dans l’espace concerné. Il n’est utilisé que lors de la publication manuelle ; les contrôles Rust ne l’utilisent pas.

Le workflow déduit l’image sous la forme `<hôte-forgejo>/<propriétaire>/<dépôt>:<image_tag>`, avec le nom en minuscules, et ajoute les labels OCI source/révision/licence. Un tag déjà présent peut être remplacé : réserver les versions publiées et utiliser `latest` seulement pour le suivi volontaire de la dernière image. L’authentification Docker passe par l’entrée standard et un répertoire de configuration temporaire supprimé à la fin de l’étape. Le nommage et l’authentification suivent le [registre de conteneurs Forgejo](https://forgejo.org/docs/latest/user/packages/container/).

Pour déployer une image publiée, conserver le dossier `docker/` et le fichier Compose, puis adapter l’adresse suivante :

```bash
export LIGHT_DNS_IMAGE=forgejo.example.com/proprietaire/light-dns:0.1.0
# Si le package est privé : docker login forgejo.example.com
docker compose pull
docker compose up -d --no-build
```

Sans `LIGHT_DNS_IMAGE`, le build local et le tag `light-dns:local` restent les défauts. Les montages TOML en lecture seule, le volume des listes, le healthcheck et la publication sur loopback sont identiques dans les deux cas. Pour un registre privé, le compte de déploiement a seulement besoin de lire les packages.

## Tests locaux

```bash
docker build --platform linux/amd64 --tag light-dns:validation .
cargo test --test integration deployment::docker -- --ignored --nocapture
```

Ce test explicite nécessite l'accès au moteur Docker et l'image construite. Il crée son propre conteneur et son volume temporaire, sans utiliser le déploiement Compose existant. Il vérifie l'utilisateur non privilégié, les montages, les réponses DNS sur les ports UDP/TCP publiés, la CLI, le rejet d'un reload invalide, une sauvegarde TOML atomique, la conservation d'une connexion TCP et l'arrêt par `SIGTERM` avec un code nul. Il vérifie aussi le healthcheck `health` et un cache distant préchargé dans le volume, retrouvé après redémarrage hors ligne. Les tests HTTP locaux couvrent séparément le téléchargement et la sauvegarde. Il utilise uniquement des réponses locales, sans upstream Internet, et nettoie ses ressources temporaires. Il reste ignoré par `cargo test` normal. `LIGHT_DNS_TEST_IMAGE` permet de choisir un tag de validation dédié sans remplacer une image existante ; le test vérifie et impose Linux AMD64.

Lors de la validation précédente, la construction et ce test ont été revalidés sur Linux AMD64 le 8 novembre 2026, avec le healthcheck UDP/TCP et un redémarrage après SIGKILL. `docker compose config --quiet` a également validé le fichier Compose fourni.

Pour le présent audit final, seul `docker compose config --quiet` a pu être réexécuté : l’accès à `/var/run/docker.sock` est refusé. La construction et le test fonctionnel AMD64 des changements restent donc à reprendre ; la validation précédente ne les couvre pas. Aucun conteneur ou service existant n’a été modifié.

## Linux ARM64 / Raspberry Pi

Les images de base [Rust](https://hub.docker.com/_/rust/) et [Debian](https://hub.docker.com/_/debian/) existent en AMD64 et ARM64. Aucune plateforme n'est forcée dans Compose : sur un Raspberry Pi équipé d'un Linux 64 bits et de Docker, le build est natif :

```bash
docker compose up -d --build
```

La validation ARM64/Raspberry Pi est réservée au mainteneur et reste ouverte dans le TODO. Le test Docker automatisé ci-dessus cible explicitement AMD64 ; il n’a pas été exécuté en ARM64. Ne pas confondre la disponibilité des bases multiarchitectures avec une validation du service sur cette architecture.
