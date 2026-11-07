# Filtrage DNS et configuration des listes

Le filtrage est actif par défaut, y compris sans section `[filtering]`. Deux sources sont proposées :

- **[AdGuard DNS filter](https://github.com/AdguardTeam/HostlistsRegistry/tree/main/filters/general/filter_1_DnsFilter)** : combinaison adaptée au DNS de listes dont EasyList et EasyPrivacy.
- **[Peter Lowe’s Ad and tracking server list](https://pgl.yoyo.org/adservers/)** : liste de domaines publicitaires et de suivi également [activée par défaut dans uBlock Origin](https://github.com/gorhill/uBlock/wiki/Dashboard:-Filter-lists).

Les URL sont explicites et éditables dans `config.toml`. Les listes sont téléchargées auprès de leurs éditeurs ; leur sélection ne requiert aucun abonnement ni clé.

Des blocs **commentés, donc désactivés**, proposent aussi [HaGeZi Light/Normal](https://github.com/hagezi/dns-blocklists), [OISD Small/Big](https://oisd.nl/setup/adguardhome), [StevenBlack Unified](https://github.com/StevenBlack/hosts) et [AdAway](https://github.com/AdAway/adaway.github.io). Décommenter les trois lignes du bloc choisi suffit à l'ajouter. Pour remplacer les listes actives par un agrégateur, commenter aussi leurs blocs. Choisir un seul niveau HaGeZi ou OISD ; empiler leurs variantes augmente surtout les doublons et le risque de perturbations. La taille des listes évolue : conserver les limites de chargement plutôt que les relever automatiquement.

```toml
[filtering]
enabled = true
blocklists = ["filters/block.txt"]
allowlist_files = ["filters/allow.txt"]
allowlist = ["safe.example.com"]
cache_dir = "data/filters"
update_interval_secs = 86400
download_timeout_secs = 15
max_list_bytes = 8388608
# Pour utiliser uniquement les fichiers locaux, remplacer les sources par :
sources = []
```

Pour personnaliser les téléchargements, remplacer `sources = []` par un ou plusieurs blocs :

```toml
[[filtering.sources]]
name = "ma-liste"
url = "https://example.com/dns-blocklist.txt"
```

`sources` remplace entièrement la sélection par défaut lorsqu'il est renseigné. `enabled = false` désactive le filtrage, les lectures de fichiers et les téléchargements. Les chemins sont relatifs au répertoire de travail ; un fichier local explicitement configuré doit exister au démarrage. Une liste vide ou composée de commentaires est acceptée ; une liste ne contenant que des règles incompatibles est rejetée.

Si tous les blocs `[[filtering.sources]]` sont commentés, le champ est absent et les deux sources par défaut sont rétablies. Pour ne télécharger aucune liste, renseigner explicitement `sources = []` dans `[filtering]`, avant les éventuels blocs de sources.

Les entrées suivantes sont reconnues, avec des noms ASCII/punycode insensibles à la casse. Un domaine inclut ses sous-domaines, avec respect des limites de labels : `ads.example` ne correspond pas à `notads.example`.

| Format | Exemple | Effet |
| --- | --- | --- |
| Domaine | `ads.example` | Blocage du domaine et de ses sous-domaines |
| Hosts | `0.0.0.0 ads.example tracker.example` | Blocage des noms associés à une IP nulle ou loopback, IPv4 ou IPv6 |
| Règle DNS Adblock | `\|\|ads.example^` | Blocage |
| Exception de liste | `@@\|\|safe.ads.example^` | Autorisation |
| Priorité | `\|\|ads.example^$important` | Blocage prioritaire sur les exceptions ordinaires |
| Désactivation | `\|\|ads.example^$badfilter` | Retrait de la règle correspondante, même dans une autre liste |

Les exceptions `$important` priment sur les blocages `$important`. L'allowlist TOML et les fichiers d'allowlist sont les exceptions explicites de l'utilisateur : elles priment sur tous les blocages. Les rewrites restent prioritaires sur ces filtres. Les en-têtes, commentaires, doublons et entrées hosts de localhost sont traités sans créer de blocage parasite.

Le filtrage porte sur le nom interrogé. Les sélecteurs cosmétiques, chemins d'URL, expressions régulières, conditions de navigateur, inclusions et modificateurs non pris en charge (dont `$dnstype`, `$dnsrewrite`, `$third-party`) sont ignorés et comptabilisés dans les journaux. Les blocs conditionnels sont ignorés en entier. Seule la partie DNS compatible des listes uBlock/AdGuard est importée.

Les fichiers locaux et le cache distant valide sont chargés avant l'écoute DNS. Les téléchargements se font ensuite en arrière-plan ; **au premier démarrage sans cache, les listes distantes deviennent actives après leur chargement**, avec un avertissement dans les journaux. Une panne de téléchargement conserve la dernière version valide ; sans version disponible, la liste concernée reste inactive et le service DNS continue. Les téléchargements vides, les pages HTML, les erreurs HTTP, les dépassements de taille et les timeouts sont rejetés.

Les mises à jour relisent aussi les fichiers locaux. Un nouvel index est construit hors des tâches DNS puis remplacé atomiquement en mémoire ; une version invalide conserve l'ancien index. Le cache est enregistré par remplacement de fichier et lié à l'URL de sa source, pour permettre un redémarrage hors ligne. Le répertoire de cache doit être dédié au service et accessible en écriture. En cas d'échec de lecture, de téléchargement ou de compilation, une nouvelle tentative a lieu après au plus 5 minutes. Un échec d’écriture du cache est signalé ; l’index valide reste actif en mémoire et la sauvegarde est réessayée après au plus 5 minutes (ou l’intervalle configuré s’il est plus court), sans retélécharger une liste encore fraîche. L’ancien fichier reste intact ; un redémarrage avant la sauvegarde réussie recharge cette ancienne version.

Les limites sont de 32 sources distantes, 32 fichiers locaux, 8 Mio par liste par défaut (1024 octets à 16 Mio configurables), 64 Mio de texte cumulé et 500000 règles avant compilation. Ces limites concernent les entrées ; elles ne constituent pas une mesure de RAM. L'intervalle de mise à jour accepte 1 à 604800 secondes et le timeout 1 à 120 secondes. Les tests utilisent des listes miniatures et un serveur HTTP local, sans Internet.

## Confidentialité et réglages légers

- **Déjà présent :** les requêtes sortantes reprennent la question DNS et les indicateurs nécessaires, sans relayer ECS ou les autres options EDNS fournies par le client. ECS peut transmettre un préfixe de son réseau ([RFC 7871](https://www.rfc-editor.org/rfc/rfc7871.html#section-2)). Cela ne masque pas l'adresse IP publique du serveur à son upstream.
- **Déjà présent :** pas de journalisation de chaque requête en configuration normale ; les rewrites et domaines bloqués sont traités localement. Les listes distantes utilisent HTTPS par défaut et leurs anciennes versions valides restent utilisables hors ligne.
- **Côté réseau :** annoncer light-dns aux clients via le routeur/DHCP, en IPv4 et en IPv6 si disponibles. Un DNS secondaire public ou un DNS chiffré configuré directement dans une application peut contourner le filtrage. Pour la redondance, utiliser un second résolveur appliquant la même politique. Ces réglages se font sur les clients et le routeur.
- **Côté navigateur :** conserver un bloqueur tel que [uBlock Origin](https://github.com/gorhill/uBlock) pour les chemins d'URL, éléments de page et publicités partageant le domaine d'un contenu utile. Le DNS seul ne distingue pas ces ressources.
- **Nouveaux noms de homelab :** `home.arpa` est réservé aux réseaux résidentiels ([RFC 8375](https://www.rfc-editor.org/rfc/rfc8375.html)). Déclarer sa zone locale par une règle telle que `"*.home.arpa" = "192.168.1.10"` dans `[rewrites]` évite d'envoyer ses noms aux upstreams ; une règle exacte seule ne réserve pas tout le suffixe.

Le lien vers les upstreams utilise actuellement UDP/TCP en clair. DoH/DoT restent dans la feuille de route. Le [cache DNS en mémoire](cache.md) réduit les requêtes répétées envoyées à l'extérieur ; les filtres restent prioritaires sur ses réponses. Ce cache est distinct du cache disque des listes distantes. Une autre amélioration à étudier est le contrôle des cibles CNAME atteignables depuis la question : le moteur filtre aujourd'hui le nom interrogé, et les exceptions devront être préservées lors de cette évolution.

