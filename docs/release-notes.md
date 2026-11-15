# light-dns v0.2.0-beta.4

Bêta publique pour Alpine Linux AArch64 (installation sys, OpenRC).

- Correction de la consommation mémoire du filtrage multi-motifs : suppression des captures inutilisées dans `regex-automata`, qui pouvaient provoquer des caches PikeVM de plusieurs Gio après des recherches variées. Les règles, exceptions, priorités, conditions, réécritures et contrôles CNAME sont conservés.
- CLI plus lisible : tableaux et couleurs en terminal pour les statistiques et le cache, aide réorganisée, confirmations et erreurs contextualisées. `--format table`, `--format toml`, `--no-color` et `NO_COLOR` permettent de choisir l’affichage ; les sorties redirigées restent en TOML pour les scripts.
- Diagnostics du processus serveur dans `stats` : PID, version, système, architecture et, sous Linux si disponibles, RSS actuelle, pic RSS, VmData et threads. Les avertissements distinguent les erreurs cumulées d’une panne actuelle et le poids du cache de la mémoire totale.
- Tests mémoire, concurrence et libération des générations ; harnais de comparaison DNS et protocole de vérification sur Raspberry Pi documentés dans `docs/filter-memory.md`.
- Configuration TOML, cache DNS, fonctionnement UDP/TCP et IPv4/IPv6, installation et mécanisme de mise à jour/rollback conservés. Aucune dépendance ajoutée.

Depuis une bêta précédente, lancer en root `light-dns update`, puis `light-dns health`, `light-dns stats` et `light-dns cache status`.

Mesures locales AMD64 sur corpus synthétique : cache regex après 100000 recherches ramené de 2,55 Gio à 3,30 Mio, pic RSS du harnais avec rechargements ramené de 2,62 Gio à environ 78 Mio. Les comparaisons DNS isolées ont traité 16 millions de requêtes sans erreur. Les campagnes complètes restent limitées par des échecs EDNS/TCP reproduits aussi sur la référence ; aucun correctif DNS extérieur au problème mémoire n’est inclus.

Assets : binaire musl statique AArch64, archive manuelle, bootstrap, service OpenRC et SHA256SUMS. Le workflow vérifie les tests automatiques et le service OpenRC sur AMD64 avant publication. La compilation croisée ne valide pas l’exécution sur ARM64, Alpine ou Raspberry Pi physique. La confirmation du correctif sur les listes exactes de l’incident et le test matériel prolongé restent à effectuer par le mainteneur.
