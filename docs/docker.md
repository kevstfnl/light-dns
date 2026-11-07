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

Le filtrage distant fonctionne comme sur Linux : un premier démarrage sans cache disque peut répondre avant le chargement des listes. Les téléchargements HTTPS nécessitent une sortie réseau. Les chemins relatifs sont résolus depuis `/run/light-dns` ; privilégier les chemins absolus de l'exemple Docker.

## Commandes et arrêt

```bash
docker compose exec light-dns light-dns stats
docker compose exec light-dns light-dns cache clear
# Après modification de docker/config.toml :
docker compose exec light-dns light-dns reload
docker compose stop
```

La CLI utilise la socket locale du conteneur ; aucun port supplémentaire ni montage de socket sur l'hôte n'est nécessaire. Le contrôle de santé Compose appelle `stats` : il vérifie la disponibilité du processus et de son contrôle local, sans sonder un upstream Internet.

Un reload réussi applique le TOML sans fermer les connexions DNS et remplace le cache. Un changement d'écoute nécessite un redémarrage ; pour conserver les publications Compose, garder le port interne 1053. Docker envoie `SIGTERM`, avec 10 secondes de grâce côté Compose et 5 secondes d'attente côté application. `docker compose stop` conserve le cache disque des filtres.

## Tests locaux

```bash
docker build --tag light-dns:validation .
cargo test --test integration deployment::docker -- --ignored --nocapture
```

Ce test explicite nécessite l'accès au moteur Docker et l'image construite. Il crée son propre conteneur et son volume temporaire, sans utiliser le déploiement Compose existant. Il vérifie l'utilisateur non privilégié, les montages, les réponses DNS sur les ports UDP/TCP publiés, la CLI, le rejet d'un reload invalide, une sauvegarde TOML atomique, la conservation d'une connexion TCP et l'arrêt par `SIGTERM` avec un code nul. Il utilise uniquement des réponses locales, sans upstream Internet, et nettoie ses ressources temporaires. Il reste ignoré par `cargo test` normal.

La construction et ce test ont réussi sur Linux AMD64 le 7 novembre 2026. `docker compose config --quiet` a également validé le fichier Compose fourni.

## Linux ARM64 / Raspberry Pi

Les images de base [Rust](https://hub.docker.com/_/rust/) et [Debian](https://hub.docker.com/_/debian/) existent en AMD64 et ARM64. Aucune plateforme n'est forcée dans Compose : sur un Raspberry Pi équipé d'un Linux 64 bits et de Docker, le build est natif :

```bash
docker compose up -d --build
```

Pour une construction ARM64 depuis une autre machine, un builder disposant déjà d'un worker ARM64 ou de l'émulation correspondante peut utiliser :

```bash
docker buildx build --platform linux/arm64 --load --tag light-dns:validation .
cargo test --test integration deployment::docker -- --ignored --nocapture
```

Le test d'exécution exige lui aussi un hôte ou une émulation ARM64. Le builder de cette session ne propose que les plateformes AMD64 ; aucune émulation n'a été installée et la validation Raspberry Pi reste ouverte dans le TODO. Ne pas confondre la disponibilité des bases multiarchitectures avec un test réussi du service en ARM64.
