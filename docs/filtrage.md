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

## Confiance des réécritures de listes

Les rewrites `[rewrites]` du TOML restent autorisés et prioritaires. Les fichiers locaux explicitement déclarés dans `blocklists` et `allowlist_files` sont de confiance pour la famille `$dnsrewrite` : leur contenu doit rester sous le contrôle de l’administrateur.

**Les sources distantes ne sont pas autorisées à fournir des `$dnsrewrite` par défaut**, y compris les sources intégrées et les listes déjà présentes dans le cache disque. Le blocage ordinaire, les exceptions, `$important` et `$badfilter` ordinaires restent actifs.

Une autorisation facultative s’applique uniquement à la source concernée :

```toml
[[filtering.sources]]
name = "rewrites-de-confiance"
url = "https://example.com/trusted-dns.txt"
allow_dnsrewrite = true # false si absent ; HTTPS obligatoire si true
```

Ne l’activer que pour une source dont l’éditeur peut légitimement synthétiser des réponses DNS pour le LAN. HTTPS protège le transport ; il ne protège pas d’un éditeur ou serveur compromis. La confiance inclut les destinations de ses redirections HTTPS. Les sources HTTP restent utilisables pour le filtrage ordinaire, avec un risque d’altération du blocage et des exceptions en transit.

Sans cette autorisation, toute règle reconnue utilisant le modificateur `$dnsrewrite` est ignorée **en entier**, y compris les exceptions `@@...$dnsrewrite`, les variantes `$important` et les désactivations `$dnsrewrite=...,badfilter`. Elle ne devient ni un blocage ni une exception ordinaire, et ne peut pas neutraliser un rewrite local/de confiance. Une règle de blocage distincte du même nom continue de s’appliquer. Les exclusions sont comptées et signalées au chargement, sans journalisation par requête.

Une liste valide composée seulement de rewrites interdits est acceptée mais inactive. Les listes distantes vides, incompatibles ou corrompues restent rejetées selon les règles habituelles. Le cache conserve le texte original ; le TOML courant décide de la confiance à chaque chargement. Retirer `allow_dnsrewrite` puis exécuter `light-dns reload` révoque l’autorisation, même hors ligne, et vide le cache DNS comme tout reload réussi. Aucun changement de format du cache disque ni retéléchargement n’est requis.

Les téléchargements gardent leurs limites de taille et de durée et la validation TLS de Reqwest. Les redirections sont bornées, limitées à HTTP(S), sans identifiants ; tout passage de HTTPS à HTTP est refusé. Une erreur de téléchargement ou de compilation conserve le dernier index et le dernier fichier persisté valides.

## Règles DNS avancées

Les règles DNS courantes suivent la [syntaxe AdGuard](https://adguard-dns.io/kb/general/dns-filtering-syntax/), avec les limites explicites ci-dessous. La compatibilité n’est pas intégrale. Le comportement historique de light-dns est conservé pour les domaines nus et les entrées hosts : ils couvrent aussi leurs sous-domaines, contrairement aux correspondances exactes de ces formats chez AdGuard. Les entrées hosts avec une adresse non nulle/non loopback restent ignorées ; utiliser `$dnsrewrite` ou le TOML pour les adresses locales.

| Syntaxe | Exemple et effet |
| --- | --- |
| Motifs | `||ad*.example^`, `|exact.example|`, `|prefix*`, `suffix.example|` ; `*` couvre zéro ou plusieurs caractères, `||` respecte les limites de labels |
| Types | `||ads.example^$dnstype=A\|AAAA` ; `~A\|~CNAME` exclut ces types. Les noms de types sont insensibles à la casse. Un mélange conserve les inclusions |
| Domaines exclus | `||example^$denyallow=safe.example\|other.example` ; les sous-domaines de ces exceptions sont inclus |
| Réécritures | `||host.example^$dnsrewrite=192.0.2.1`, `2001:db8::1`, `target.example` ou `REFUSED` |
| Forme complète | `||host.example^$dnsrewrite=NOERROR;A;192.0.2.1` ; A, AAAA, CNAME, PTR, MX, TXT, SRV, HTTPS et SVCB sont reconnus |
| Réponses vides | `$dnsrewrite=NOERROR;;` pour NODATA ; codes reconnus : NOERROR, FORMERR, SERVFAIL, NXDOMAIN, NOTIMP, REFUSED |
| Exceptions de réécriture | `@@||host.example^$dnsrewrite` retire toutes ses réécritures ; `@@||host.example^$dnsrewrite=192.0.2.1` retire cette valeur seulement |

Un motif absent, par exemple `$dnstype=AAAA,denyallow=safe.example,dnsrewrite=NOERROR;;`, couvre tous les noms. Les valeurs TXT contenant une virgule utilisent `\,` pour ne pas séparer les modificateurs. Les autres données sont validées par le parseur DNS de Hickory. Les réécritures locales ont un TTL de 60 secondes ; elles répondent uniquement pour les types disponibles, ou NODATA.

Les rewrites TOML restent prioritaires, puis l’allowlist utilisateur. Dans les listes, les exceptions `$dnsrewrite` retirent les réécritures correspondantes ; les réécritures restantes priment sur les blocages ordinaires ou `$important`. Un code de réponse/une réponse vide prime sur CNAME, qui prime sur les autres données. Les valeurs A/AAAA et les autres RR compatibles sont réunies sans doublon. Les conflits entre codes ou CNAME de même priorité sont départagés par l’ordre des sources et des lignes. Une exception `$dnsrewrite` ne désactive pas un blocage ordinaire. `$badfilter` compare la règle complète, conditions comprises, indépendamment de l’ordre des modificateurs.

Une réécriture CNAME recherche sa cible dans le même pipeline : rewrites, filtres, cache, upstreams. Le budget réseau est partagé sur la chaîne. Avec `RD=0`, seuls le CNAME et les éventuelles données locales/en cache sont rendus. Les synthèses ne sont pas conservées dans le cache upstream. Les cycles et chaînes dépassant 16 liens répondent SERVFAIL ; les réponses synthétiques ne revendiquent ni autorité ni authentification DNSSEC.

## Contrôle des réponses CNAME

Les cibles CNAME atteignables depuis la question sont contrôlées, y compris sur les réponses déjà en cache et après changement des listes. Les CNAME sans rapport, les autorités et les additionnels ne provoquent pas de blocage. Leur ordre dans la réponse n’a pas d’importance. Le contrôle utilise le type CNAME pour `$dnstype`, sans émettre de requête réseau supplémentaire. Une cible bloquée donne le NXDOMAIN local habituel ; cette synthèse n’est pas cachée. Une exception sur la question dispense du contrôle de blocage de sa chaîne ; une exception sur une cible ne protège pas les cibles suivantes. Les cycles et les chaînes trop longues sont rejetés.

Les noms simples restent dans un index de domaines ; les motifs sont analysés par globset puis compilés en un automate multi-motifs avec regex-automata, hors des workers DNS. Cette compilation évite la stratégie par extension de fichiers de globset, inadaptée aux suffixes DNS communs. Le moteur de compilation borne son NFA à 10 Mio et une erreur conserve l’ancien index actif. Le chargement/reload peut temporairement conserver plusieurs index et structures de compilation : les limites de texte et du cache DNS ne constituent pas un plafond strict de RSS.

Les regex, profils par appareil (`$client`, `$ctag`), sélecteurs cosmétiques, chemins d’URL, conditions de navigateur, inclusions et modificateurs inconnus (`$third-party`, etc.) restent ignorés et comptabilisés. Un modificateur inconnu fait ignorer toute la règle. Les blocs conditionnels sont ignorés en entier. Le filtrage des IP de réponse et des hints HTTPS/SVCB n’est pas ajouté.

Les fichiers locaux et le cache distant valide sont chargés avant l'écoute DNS. Les téléchargements se font ensuite en arrière-plan ; **au premier démarrage sans cache, les listes distantes deviennent actives après leur chargement**, avec un avertissement dans les journaux. Une panne de téléchargement conserve la dernière version valide ; sans version disponible, la liste concernée reste inactive et le service DNS continue. Les téléchargements vides, les pages HTML, les erreurs HTTP, les dépassements de taille et les timeouts sont rejetés.

Les mises à jour relisent aussi les fichiers locaux. Un nouvel index est construit hors des tâches DNS puis remplacé atomiquement en mémoire ; une version invalide conserve l'ancien index. Le cache est enregistré par remplacement de fichier et lié à l'URL de sa source, pour permettre un redémarrage hors ligne. Le répertoire de cache doit être dédié au service et accessible en écriture. En cas d'échec de lecture, de téléchargement ou de compilation, une nouvelle tentative a lieu après au plus 5 minutes. Les nouveaux fichiers persistés sont privés (`0600` sous Unix). Un temporaire abandonné après un crash n’est ni écrasé ni supprimé ; jusqu’à 32 noms sont essayés par sauvegarde, puis la tentative est reportée. Les noms déjà pris ne bloquent donc pas systématiquement les redémarrages avec le PID 1 de Docker. Les temporaires abandonnés restent à inspecter/nettoyer manuellement service arrêté si des crashes répétés les accumulent. Le remplacement atomique protège un crash du processus ; aucune garantie supplémentaire contre une coupure électrique n’est ajoutée. Un échec d’écriture du cache est signalé ; l’index valide reste actif en mémoire et la sauvegarde est réessayée après au plus 5 minutes (ou l’intervalle configuré s’il est plus court), sans retélécharger une liste encore fraîche. L’ancien fichier reste intact ; un redémarrage avant la sauvegarde réussie recharge cette ancienne version.

Les limites sont de 32 sources distantes, 32 fichiers locaux, 8 Mio par liste par défaut (1024 octets à 16 Mio configurables), 64 Mio de texte cumulé et 500000 règles avant compilation. Ces limites concernent les entrées ; elles ne constituent pas une mesure de RAM. L'intervalle de mise à jour accepte 1 à 604800 secondes et le timeout 1 à 120 secondes. Les tests utilisent des listes miniatures et un serveur HTTP local, sans Internet.

## Confidentialité et réglages légers

- **Déjà présent :** les requêtes sortantes reprennent la question DNS et les indicateurs nécessaires, sans relayer ECS ou les autres options EDNS fournies par le client. ECS peut transmettre un préfixe de son réseau ([RFC 7871](https://www.rfc-editor.org/rfc/rfc7871.html#section-2)). Cela ne masque pas l'adresse IP publique du serveur à son upstream.
- **Déjà présent :** pas de journalisation de chaque requête en configuration normale ; les rewrites et domaines bloqués sont traités localement. Les listes distantes utilisent HTTPS par défaut et leurs anciennes versions valides restent utilisables hors ligne.
- **Côté réseau :** annoncer light-dns aux clients via le routeur/DHCP, en IPv4 et en IPv6 si disponibles. Un DNS secondaire public ou un DNS chiffré configuré directement dans une application peut contourner le filtrage. Pour la redondance, utiliser un second résolveur appliquant la même politique. Ces réglages se font sur les clients et le routeur.
- **Côté navigateur :** conserver un bloqueur tel que [uBlock Origin](https://github.com/gorhill/uBlock) pour les chemins d'URL, éléments de page et publicités partageant le domaine d'un contenu utile. Le DNS seul ne distingue pas ces ressources.
- **Nouveaux noms de homelab :** `home.arpa` est réservé aux réseaux résidentiels ([RFC 8375](https://www.rfc-editor.org/rfc/rfc8375.html)). Déclarer sa zone locale par une règle telle que `"*.home.arpa" = "192.168.1.10"` dans `[rewrites]` évite d'envoyer ses noms aux upstreams ; une règle exacte seule ne réserve pas tout le suffixe.

Le lien vers les upstreams utilise actuellement UDP/TCP en clair. DoH/DoT restent dans la feuille de route. Le [cache DNS en mémoire](cache.md) réduit les requêtes répétées envoyées à l'extérieur ; les filtres restent prioritaires sur ses réponses. Ce cache est distinct du cache disque des listes distantes. Les cibles CNAME liées à la question sont contrôlées selon les règles et exceptions décrites ci-dessus.

