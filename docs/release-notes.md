# light-dns v0.2.0-beta.1

Bêta publique pour Alpine Linux AArch64 en installation sys avec OpenRC.

- CLI autonome : installation, mise à jour avec vérification SHA-256 et rollback, désinstallation et purge confirmée.
- Bootstrap POSIX BusyBox et installation manuelle vérifiée ; aucun compilateur, Git, GitHub CLI ou Docker sur le Pi.
- Service non-root, capacité réseau conditionnelle, configuration et données conservées.
- Releases stables et bêta reconnues ; sélection explicite et vérification sans mutation.
- Fonctions DNS et configuration TOML existantes conservées.

Assets : binaire musl statique AArch64, archive manuelle, bootstrap, service OpenRC et `SHA256SUMS`. Consulter le README et le guide Alpine avant installation.

**Raspberry Pi physique et exécution Alpine AArch64 non validés.** La compilation croisée et la CI AMD64 ne remplacent pas les essais matériels du mainteneur. Ne pas basculer les clients DNS avant ses propres vérifications. Les validations restantes figurent dans `TODO.MD`.
