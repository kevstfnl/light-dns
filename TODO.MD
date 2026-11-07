# TODO — light-dns

## 1. Initialisation
- [x] Initialiser le projet Rust.
- [x] Mettre en place la configuration TOML.
- [x] Créer le README et choisir la licence MIT.
- [ ] Valider la configuration (IP, ports, domaines).

## 2. Serveur DNS
- [ ] Intégrer Tokio et Hickory DNS.
- [ ] Créer un serveur DNS UDP.
- [ ] Ajouter le support TCP.
- [ ] Implémenter le DNS forwarding vers les upstreams.
- [ ] Gérer les timeouts et les erreurs DNS.

## 3. DNS Rewrites
- [ ] Implémenter les domaines personnalisés (`nas.lan`).
- [ ] Ajouter le support des wildcards (`*.lan`).
- [ ] Gérer les priorités entre les règles.
- [ ] Prendre en charge les enregistrements A et AAAA.

## 4. Filtrage DNS
- [ ] Charger les blocklists depuis des fichiers locaux.
- [ ] Bloquer les domaines présents dans les listes.
- [ ] Ajouter une allowlist pour les exceptions.
- [ ] Permettre le téléchargement et la mise à jour des listes.

## 5. Cache et performances
- [ ] Ajouter un cache DNS avec respect des TTL.
- [ ] Limiter l'utilisation de la mémoire.
- [ ] Réaliser des benchmarks de latence et de consommation.

## 6. Configuration
- [ ] Ajouter le hot reload du fichier TOML.
- [ ] Conserver l'ancienne configuration si la nouvelle est invalide.

## 7. Docker
- [ ] Créer un Dockerfile multi-stage.
- [ ] Ajouter un `compose.yaml`.
- [ ] Monter la configuration via un volume.
- [ ] Tester sur Linux ARM64 (Raspberry Pi).

## 8. Qualité et publication
- [ ] Ajouter des tests unitaires et d'intégration.
- [ ] Configurer GitHub Actions (fmt, Clippy, tests).
- [ ] Publier une première version `v0.1.0`.

## Plus tard
- [ ] Exposer des métriques Prometheus (`/metrics`).
- [ ] Ajouter le support DNS-over-HTTPS / DNS-over-TLS.
