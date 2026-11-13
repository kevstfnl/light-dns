# light-dns v0.2.0-beta.3

Bêta publique pour Alpine Linux AArch64 (installation sys, OpenRC).

- Administration sans `--socket` : priorité au chemin explicite, puis à la socket locale présente, puis à `/run/light-dns/control.sock`. Aucun repli après échec sur la socket sélectionnée ; les chemins du serveur restent inchangés.
- Nouvelle commande `light-dns cache status` : activation, entrées, mémoire pondérée, limites, TTL maximal, hits et vidages cumulés, au format TOML.
- Commande d’installation raccourcie : téléchargement complet du bootstrap avant exécution, sans pipeline ni fichier temporaire pour le script.
- README réorganisé et guides détaillés dans `docs/`.
- Configuration, données, commandes DNS existantes et mécanisme de mise à jour/rollback conservés ; aucune dépendance ajoutée.

Depuis la bêta 2, lancer en root `light-dns update`, puis `light-dns health` et `light-dns cache status`.

Assets : binaire musl statique AArch64, archive manuelle, bootstrap, service OpenRC et SHA256SUMS. Les tests automatiques et le service OpenRC sont validés sur AMD64 avant publication. La compilation croisée ne valide pas l’exécution sur Pi physique. Un premier démarrage fonctionnel sur Pi a été rapporté pour la bêta 2 ; la validation matérielle complète (reboot, mise à jour/rollback, purge) reste à effectuer par le mainteneur.
