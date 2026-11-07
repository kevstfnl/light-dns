# TODO — light-dns

## 1. Initialisation
- [x] Initialiser le projet Rust.
- [x] Mettre en place la configuration TOML.
- [x] Créer le README et choisir la licence MIT.
- [x] Valider la configuration (IP, ports, domaines).

## 2. Serveur DNS
- [x] Intégrer Tokio et Hickory DNS.
- [x] Créer un serveur DNS UDP.
- [x] Ajouter le support TCP.
- [x] Implémenter le DNS forwarding vers les upstreams.
- [x] Gérer les timeouts et les erreurs DNS.

## 3. DNS Rewrites
- [x] Implémenter les domaines personnalisés (`nas.lan`).
- [x] Ajouter le support des wildcards (`*.lan`).
- [x] Gérer les priorités entre les règles.
- [x] Prendre en charge les enregistrements A et AAAA.

## 4. Filtrage DNS
- [x] Charger les blocklists depuis des fichiers locaux.
- [x] Bloquer les domaines présents dans les listes.
- [x] Ajouter une allowlist pour les exceptions.
- [x] Permettre le téléchargement et la mise à jour des listes.

## 5. Cache et performances
- [x] Ajouter un cache DNS avec respect des TTL.
- [x] Limiter l'utilisation de la mémoire (entrées et budget pondéré estimé).
- [x] Réaliser des benchmarks locaux de latence, CPU et RSS reproductibles.

## 6. Configuration et administration locale
- [x] Recharger le TOML par commande CLI explicite, avec remplacement atomique.
- [x] Conserver l'ancienne configuration et le cache si le rechargement échoue.
- [x] Ajouter une commande de purge du cache DNS.
- [x] Afficher des statistiques légères par CLI via une socket Unix protégée.

## 7. Docker
- [x] Créer un Dockerfile multi-stage avec utilisateur non privilégié.
- [x] Ajouter un `compose.yaml` UDP/TCP.
- [x] Monter la configuration en lecture seule et persister les listes téléchargées.
- [x] Tester le conteneur Linux AMD64, la CLI, UDP/TCP et l'arrêt par SIGTERM.
- [ ] Tester sur Linux ARM64 (Raspberry Pi).

Le builder disponible ne propose que Linux AMD64 ; la procédure ARM64 est documentée dans `docs/docker.md`.

## 8. Qualité et publication
- [x] Structurer les modules par responsabilité et centraliser les tests dans une arborescence miroir sous `tests/`.
- [ ] Ajouter des tests unitaires et d'intégration.
- [ ] Configurer GitHub Actions (fmt, Clippy, tests).
- [ ] Publier une première version `v0.1.0`.

## Plus tard
- [ ] Filtrer les cibles CNAME liées à la question DNS, en respectant l'allowlist.
- [ ] Exposer des métriques Prometheus (`/metrics`).
- [ ] Ajouter le support DNS-over-HTTPS / DNS-over-TLS.
