# Validations restantes

- [x] Réexécuter les tests AMD64 avec fixtures UDP/TCP, IPv4/IPv6 et Unix dans un environnement autorisé : 139 tests Rust réussis le 2026-11-12 (9 tests explicitement ignorés : benchmarks et Docker).
- [x] Retirer Python du dépôt et de la chaîne de release ; 8 tests de packaging migrés en Rust, sans exécuter de binaire ARM ni modifier le système hôte.
- [x] Valider le workflow GitHub et les scripts d'archive/installation OpenRC : fmt, Clippy sans avertissement, tests et check avec lockfile, build release AMD64, ShellCheck 0.11.0 et actionlint 1.7.12 réussis le 2026-11-12.
- [ ] Terminer les A/B DNS p50/p95/p99, CPU par succès, débit, erreurs et RSS à 1/32/256 clients ; aucun candidat hors réseau ne justifie une optimisation de production.
- [ ] Mesurer première panne, sondes et reprise avec upstreams retardés/congestionnés ; conserver le timeout par défaut tant que ces mesures manquent.
- [ ] Mesurer le démon et les pics de reload avec un corpus réel, puis confirmer les compromis mémoire du cache pré-parsé.
- [ ] Revalider le conteneur Linux AMD64 et les runners/cache/publication sur l’instance Forgejo cible.
- [ ] Terminer l’audit des dépendances verrouillées avec un outil et une base de vulnérabilités disponibles.
- [ ] Faire valider **manuellement par le mainteneur** ARM64, Alpine Linux et Raspberry Pi avant le déploiement réel.

Procédures : [maintenance](docs/maintenance.md), [installation et validation Alpine](docs/alpine.md). La bêta publique est autorisée après CI ; la validation physique Raspberry Pi reste manuelle et non effectuée.

## Gestion autonome — v0.2.0-beta.1

- [x] CLI install/update/check/version/uninstall/purge, bootstrap BusyBox, vérification SHA-256, verrouillage et rollback avec récupération après interruption.
- [x] Tests AMD64 : 149 réussis, 10 ignorés par défaut ; test OpenRC Alpine AMD64 ignoré par défaut exécuté séparément avec succès. fmt, Clippy sans avertissement, check avec lockfile, ShellCheck 0.11.0 et actionlint 1.7.12 validés le 2026-11-12.
- [x] Compiler et inspecter l’ELF AArch64 musl statique sans l’exécuter ; ne constitue aucune validation ARM64 ou Raspberry Pi. Zig 0.15.2 émet un avertissement bénin sur l’option historique du linker `-O1`.
- [x] Vérifier sur Alpine AMD64 isolé : OpenRC start/restart/reload/stop, santé UDP/TCP, UID non-root, seule capacité effective CAP_NET_BIND_SERVICE et refus d’installation non-root. Test rendu obligatoire dans le workflow avant publication.
- [ ] Vérifier la CI distante et les téléchargements anonymes de la bêta publiée.
- [ ] Validation physique par le mainteneur : installation sys Alpine AArch64, reboot, mise à jour/rollback et purge interactive sur Raspberry Pi 5. Aucun essai ARM64 effectué.
