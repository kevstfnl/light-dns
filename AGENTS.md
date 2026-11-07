# Règles de contribution

- Aucun code `unsafe` ; conserver les interdictions du compilateur.
- Architecture simple et modulaire ; aucune fonctionnalité non demandée.
- Performance first : mesurer les optimisations (latence, CPU, RAM, concurrence).
- Priorité à la fiabilité DNS : préserver UDP/TCP, IPv4/IPv6, validation et fallback.
- Préférer les bibliothèques maintenues aux réimplémentations et pools maison.
- Gérer explicitement erreurs, timeouts et limites de ressources.
- Aucun `unwrap()` ou `expect()` non justifié dans les chemins critiques.
- Préserver la CLI (`reload`, `stats`, `cache clear`) et la configuration TOML.
- Recharger le TOML uniquement sur commande ; aucune surveillance automatique.
- Ajouter des tests pour les changements fonctionnels ; fixtures réseau locales.
- Après chaque étape : fmt, Clippy sans avertissement, tests et check avec lockfile.
- Tenir `TODO.MD` et la documentation concernée à jour après validation.
- Docker doit rester compatible, non privilégié et local par défaut.
- Ne pas tester ARM64/Raspberry Pi ni cocher cette validation sans le mainteneur.
