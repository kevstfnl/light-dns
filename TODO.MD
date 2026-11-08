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
- [x] Ajouter des tests de configuration et d’intégration DNS, cache, filtres et CLI.
- [x] Configurer Forgejo Actions (fmt, Clippy, tests, check).
- [x] Préparer Forgejo Actions et la publication manuelle d’une image AMD64 dans son registre.
- [ ] Valider les runners, le cache et la publication sur l’instance Forgejo cible.
- [ ] Publier une première version `v0.1.0`.

## Audit correctif
- [x] Vérifier les tests existants : 87 réussis avant correction ; validation finale : 93 réussis, 3 ignorés par défaut, test Docker AMD64 réussi séparément, fmt/Clippy/check réussis.
- [x] Tester le premier démarrage, les caches invalides et une source indisponible.
- [x] Réessayer une sauvegarde de filtres échouée après au plus cinq minutes, sans retéléchargement.
- [x] Tester les réponses TCP invalides, REFUSED, la troncature TCP et le budget de fallback.
- [x] Auditer les transports Hickory et évaluer un provider partagé ; conserver le forwarder actuel faute de gain démontré.
- [ ] Terminer la comparaison forwarding (5 essais UDP/TCP/fallback/failover) : refus système `EPERM` sur `send_to` à 256 clients dans cet environnement ; protocole et calibration dans `docs/performance.md`.
- [x] Revalider Docker AMD64, y compris healthcheck et persistance après redémarrage.
- [x] Ajouter le fichier MIT et les métadonnées Cargo cohérentes.
- [x] Ajouter la CI Rust 1.99.1 ; ses commandes passent localement (exécution Forgejo à vérifier sur l’instance cible).

## Fiabilité et filtrage étendu
- [x] Filtrer les cibles CNAME liées à la question DNS, y compris dans le cache et en respectant les exceptions.
- [x] Récupérer la socket d’administration après un arrêt brutal, avec verrou exclusif et protection des chemins existants.
- [x] Écarter les upstreams en panne pendant 5 secondes, indépendamment en UDP/TCP, et sonder leur retour sans attendre dans le chemin de réponse sain.
- [x] Préserver le plafond commun de 256 échanges, sondes et anciennes générations comprises.
- [x] Ajouter `health` UDP/TCP local et `health --upstream`, sans relire le TOML.
- [x] Ajouter motifs DNS, `$dnstype`, `$denyallow`, `$dnsrewrite`, exceptions et désactivation par identité complète de règle.
- [x] Préserver les formats domaines/hosts historiques, les rewrites TOML et les priorités utilisateur.
- [x] Couvrir cycles CNAME, cache après mise à jour, reprise des upstreams, démarrages concurrents et saturation après reload ; 106 tests fonctionnels réussis.
- [x] Mesurer le cache et le filtrage avant/après sur cinq essais, avec 100000 domaines et 1000 motifs ; chiffres et surcoûts documentés.
- [x] Revalider Docker AMD64 : healthcheck UDP/TCP, reload, persistance, SIGKILL et arrêt normal.
- [ ] Reprendre la campagne forwarding soutenue hors de cet environnement : EPERM observé à 32 puis 1 client, réponses en erreur sous charge ; aucune conclusion de débit ni de bascule retenue.

## Audit final avant premier déploiement
- [x] Interdire par défaut la famille `$dnsrewrite` distante, cache disque compris, sans convertir les règles en blocages.
- [x] Ajouter `allow_dnsrewrite` par source, exiger HTTPS et préserver les rewrites TOML/locaux, exceptions et `$badfilter` ordinaires.
- [x] Valider sans socket la matrice de confiance, les redirections, les permissions/verrous et la persistance ; 13 tests internes réussis.
- [x] Refuser les dossiers de contrôle modifiables par groupe/autres et les verrous à liens multiples.
- [x] Éviter les collisions avec les temporaires abandonnés après réutilisation de PID ; nouveaux caches Unix mode 0600.
- [x] Ajouter les intégrations téléchargement/mise à jour/révocation et le scénario homelab avec trois SIGKILL (compilés ; exécution réseau en attente).
- [x] Instrumenter p99, débit, catégories d’erreurs/timeouts, VmHWM et première bascule avant échauffement.
- [x] Conserver les références release et réaliser cinq A/B mémoire par corpus (100000/450000 domaines + 1000 motifs), avec vérification de libération des anciens index.
- [ ] Réexécuter tous les tests réseau : `EPERM` dès les fixtures loopback dans l’environnement présent ; aucun contournement.
- [ ] Terminer la comparaison DNS p50/p95/p99/CPU/RSS/débit, tous scénarios et concurrences 1/32/256 ; aucune optimisation non mesurée conservée.
- [ ] Reproduire les pics mémoire et le surcoût observé après sécurisation avec corpus réel, démon complet et reload sous trafic.
- [ ] Revalider Docker AMD64 (build et intégration) : accès au moteur refusé ; syntaxe Compose validée.
- [ ] Terminer l’audit exhaustif des dépendances verrouillées : cargo-audit absent et requête OSV bloquée par la résolution DNS de l’environnement.

Rapport et limitations : `docs/audit-final.md`. La validation ARM64/Raspberry Pi/Alpine reste exclusivement manuelle par le mainteneur ; aucune case ARM64 n’a été cochée.

## Plus tard
- [ ] Exposer des métriques Prometheus (`/metrics`).
- [ ] Ajouter le support DNS-over-HTTPS / DNS-over-TLS.
