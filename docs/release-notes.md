# light-dns v0.1.0

Première distribution native pour Alpine Linux aarch64 sur Raspberry Pi 5 avec stockage persistant.

- Serveur DNS UDP/TCP, IPv4/IPv6, forwarding avec fallback, cache TTL, rewrites locaux et filtrage de domaines.
- Administration locale : `health`, `stats`, `reload` et `cache clear`. Rechargement du TOML uniquement sur commande.
- Binaire musl statique ; aucun compilateur, Docker ou Python nécessaire sur le Pi.
- Installateur avec compte dédié, capacité limitée à l'écoute sur le port 53 et service OpenRC. Configuration existante conservée lors d'une réinstallation.

Télécharger l'archive `light-dns-v0.1.0-aarch64-unknown-linux-musl.tar.gz` et `SHA256SUMS`, vérifier la somme puis suivre `INSTALL.md` dans l'archive. Le service écoute initialement sur `127.0.0.1:53` ; l'activation OpenRC et la configuration LAN sont explicites.

**Brouillon : validation matérielle par le mainteneur requise avant publication.** La compilation croisée et les contrôles ELF ne valident pas l'exécution sur Alpine ou Raspberry Pi. Vérifier OpenRC, UDP/TCP, IPv4/IPv6, fallback, listes HTTPS, administration, réinstallation et reboot avant de basculer les clients DNS. Les mesures de performance et de mémoire restantes sont suivies dans `TODO.MD` ; aucun gain mesuré sur le Pi n'est revendiqué.
