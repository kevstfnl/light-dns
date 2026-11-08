# Validations restantes

- [ ] Réexécuter tous les tests réseau UDP/TCP, IPv4/IPv6 et Unix dans un environnement autorisé : `EPERM` ici.
- [ ] Terminer les A/B DNS p50/p95/p99, CPU par succès, débit, erreurs et RSS à 1/32/256 clients ; aucun candidat hors réseau ne justifie une optimisation de production.
- [ ] Mesurer première panne, sondes et reprise avec upstreams retardés/congestionnés ; conserver le timeout par défaut tant que ces mesures manquent.
- [ ] Mesurer le démon et les pics de reload avec un corpus réel, puis confirmer les compromis mémoire du cache pré-parsé.
- [ ] Revalider le conteneur Linux AMD64 et les runners/cache/publication sur l’instance Forgejo cible.
- [ ] Terminer l’audit des dépendances verrouillées avec un outil et une base de vulnérabilités disponibles.
- [ ] Faire valider **manuellement par le mainteneur** ARM64, Alpine Linux et Raspberry Pi avant le déploiement réel.

Procédures : [maintenance](docs/maintenance.md). Aucun push ni publication n’est inclus dans cette passe.
