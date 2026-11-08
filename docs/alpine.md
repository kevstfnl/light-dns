# Alpine Linux aarch64 — Raspberry Pi 5

Cette distribution native utilise un binaire Rust musl statique pour Alpine **64 bits** (`uname -m` doit afficher `aarch64`). Rust, Cargo, CMake et Docker ne sont pas nécessaires sur le Pi. Les certificats CA restent nécessaires pour les listes HTTPS.

## Télécharger et installer

Télécharger l'archive et `SHA256SUMS` depuis la même [release GitHub](https://github.com/kevstfnl/light-dns/releases). Exemple pour la première version publiée :

```sh
version=v0.1.0
archive="light-dns-$version-aarch64-unknown-linux-musl.tar.gz"
base="https://github.com/kevstfnl/light-dns/releases/download/$version"
wget "$base/$archive" "$base/SHA256SUMS"
sha256sum -c SHA256SUMS
tar -xzf "$archive"
cd "light-dns-$version-aarch64-unknown-linux-musl"
doas sh install.sh
```

Utiliser `sudo` ou un shell root si `doas` n'est pas configuré. Ne continuer que si la vérification SHA-256 réussit ; elle détecte une archive corrompue, sans constituer une signature indépendante de GitHub.

Un brouillon n'est accessible qu'aux comptes autorisés : le mainteneur peut utiliser `gh release download v0.1.0 --repo kevstfnl/light-dns --dir release-v0.1.0`, puis transférer les deux fichiers sur le Pi. Les commandes `wget` ci-dessus fonctionnent après publication.

Le script exige Alpine, OpenRC et aarch64. Il installe `ca-certificates` et `libcap-setcap` via `apk`, crée le compte système `light-dns`, installe le binaire et donne uniquement la capacité `cap_net_bind_service`. Le système de fichiers du binaire doit permettre les capacités de fichiers. Aucun service n'est activé ou démarré automatiquement.

## Configurer et démarrer

Éditer `/etc/light-dns/config.toml`. Le défaut écoute uniquement sur **127.0.0.1:53**, en UDP et TCP, avec deux upstreams et le filtrage distant par défaut. Pour desservir le LAN, choisir l'adresse LAN du Pi dans `dns.listen`, autoriser UDP/TCP dans le pare-feu local et configurer les clients ou le DHCP. Pour une écoute IPv6, utiliser la syntaxe `[adresse]:53`. Ne pas exposer le résolveur à Internet.

Vérifier qu'aucun autre DNS, notamment AdGuard Home, n'occupe déjà le port choisi avant de démarrer. Ne pas modifier le DNS des clients avant les vérifications.

```sh
doas rc-update add light-dns default
doas rc-service light-dns start
doas light-dns --socket /run/light-dns/control.sock health
doas light-dns --socket /run/light-dns/control.sock health --upstream
```

Le démon s'exécute sous `light-dns:light-dns`, depuis `/var/lib/light-dns`. Les listes persistent dans `/var/lib/light-dns/filters`. OpenRC recrée `/run/light-dns` avec les permissions privées requises à chaque démarrage ; la socket et son verrou ne doivent pas être supprimés pendant l'exécution. Les sorties passent par `logger` vers le journal système (habituellement `/var/log/messages` si syslog est actif).

```sh
doas light-dns --socket /run/light-dns/control.sock stats
doas light-dns --socket /run/light-dns/control.sock cache clear
# Après modification du TOML, sans changement d'écoute :
doas rc-service light-dns reload
# Après changement de l'adresse ou du port :
doas rc-service light-dns restart
```

Le TOML n'est jamais surveillé automatiquement. OpenRC supervise le démon et limite ses relances après crash ; une configuration invalide doit être corrigée avant de le relancer.

## Mise à jour et retour arrière

Télécharger et vérifier la nouvelle archive avant d'arrêter le service. Conserver l'ancienne archive et sauvegarder `/etc/light-dns/config.toml`.

```sh
doas rc-service light-dns stop
# Depuis le dossier de la nouvelle archive extraite :
doas sh install.sh
doas rc-service light-dns start
doas light-dns --socket /run/light-dns/control.sock health
```

Le script refuse de remplacer un service actif, conserve la configuration existante et les listes, remplace le binaire atomiquement et réapplique sa capacité. Il ne redémarre pas le service. En cas d'échec, garder le service arrêté, corriger la cause ou réinstaller l'ancienne archive avec la même procédure. Si la configuration a été modifiée, restaurer aussi sa sauvegarde.

Installation prévue sur stockage persistant. En Alpine **diskless**, rendre persistants le binaire, la configuration, le compte système, le service et son activation avec les mécanismes Alpine (`lbu`/stockage persistant) ; placer les listes sur un stockage persistant. Les capacités étendues doivent être préservées ou réappliquées après restauration. Vérifier ces points après redémarrage avant d'utiliser ce mode en production.

## Validation mainteneur avant publication

La compilation croisée et l'inspection ELF ne valident pas l'exécution sur Pi. Sur le Raspberry Pi 5 réel, vérifier :

- démarrage, arrêt et redémarrage OpenRC, fonctionnement sans root et accès au port 53 ;
- résolutions UDP/TCP, IPv4/IPv6, rewrites et fallback avec fixtures locales ;
- téléchargement HTTPS des listes et conservation du cache disque ;
- `health`, `stats`, `reload` et `cache clear`, puis une mise à jour conservant le TOML ;
- démarrage après reboot et absence de conflit avec l'ancien serveur DNS.

Reporter le résultat dans `TODO.MD` et publier le brouillon seulement après cette validation. Aucune validation matérielle n'est automatisée dans le workflow.
