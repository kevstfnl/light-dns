# Validations restantes

- [x] Réexécuter les tests AMD64 avec fixtures UDP/TCP, IPv4/IPv6 et Unix dans un environnement autorisé : 131 tests Rust réussis le 2026-11-09 (9 tests explicitement ignorés : benchmarks et Docker).
- [x] Préparer le workflow GitHub de compilation croisée Alpine aarch64 et les scripts d'archive/installation OpenRC ; 8 tests simulés d'installation et d'archive réussis, ShellCheck et actionlint sans erreur.
- [ ] Terminer les A/B DNS p50/p95/p99, CPU par succès, débit, erreurs et RSS à 1/32/256 clients ; aucun candidat hors réseau ne justifie une optimisation de production.
- [ ] Mesurer première panne, sondes et reprise avec upstreams retardés/congestionnés ; conserver le timeout par défaut tant que ces mesures manquent.
- [ ] Mesurer le démon et les pics de reload avec un corpus réel, puis confirmer les compromis mémoire du cache pré-parsé.
- [ ] Revalider le conteneur Linux AMD64 et les runners/cache/publication sur l’instance Forgejo cible.
- [ ] Terminer l’audit des dépendances verrouillées avec un outil et une base de vulnérabilités disponibles.
- [ ] Faire valider **manuellement par le mainteneur** ARM64, Alpine Linux et Raspberry Pi avant le déploiement réel.

Procédures : [maintenance](docs/maintenance.md), [installation et validation Alpine](docs/alpine.md). La release reste en brouillon jusqu'à validation manuelle du mainteneur sur Raspberry Pi.
