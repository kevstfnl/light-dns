# light-dns

Un serveur DNS léger et sans interface graphique, développé en Rust pour les homelabs et les environnements à ressources limitées.

Configuration simple, faible consommation de ressources et déploiement via Docker.

> **Statut :** En cours de développement. Seul le chargement de la configuration TOML est actuellement implémenté.

## Fonctionnalités prévues

- **DNS Forwarding** — Transmission des requêtes vers des serveurs DNS externes.
- **DNS Rewrites** — Domaines personnalisés avec prise en charge des wildcards (`*.lan`).
- **DNS Filtering** — Blocage des publicités, trackers et domaines malveillants via des blocklists.
- **DNS Cache** — Mise en cache des réponses pour réduire la latence.
- **Configuration TOML** — Configuration centralisée dans un fichier.
- **Docker** — Déploiement léger avec Docker Compose.

## Fonctionnement

Les requêtes DNS sont traitées dans cet ordre :

1. Recherche d'une règle de réécriture DNS locale.
2. Vérification des listes de blocage.
3. Recherche d'une réponse dans le cache.
4. Transmission aux serveurs DNS externes.

## Configuration

Exemple de fichier `config.toml` :

```toml
[dns]
listen = "0.0.0.0:5353"
upstreams = ["1.1.1.1:53", "9.9.9.9:53"]

[rewrites]
"*.lan" = "192.168.1.10"
"router.lan" = "192.168.1.1"
"nas.lan" = "192.168.1.20"
```

## Développement

Nécessite une installation de Rust et Cargo.

```bash
cargo run
```

Le support Docker sera ajouté dans une prochaine version.

## Licence

Ce projet est distribué sous licence [MIT](https://opensource.org/licenses/MIT).
