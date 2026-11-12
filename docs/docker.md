# Docker Compose


Prérequis : Git, Docker et Docker Compose.

```bash
git clone https://github.com/kevstfnl/light-dns.git
cd light-dns
```

Modifier `docker/config.toml` : choisir les IP des upstreams dans `[dns]` et les adresses locales dans `[rewrites]`. Garder l’écoute interne `0.0.0.0:1053`. Les exemples complets se trouvent dans les fichiers TOML.

Compose publie **UDP et TCP sur `127.0.0.1:53` par défaut** : les autres appareils du LAN ne peuvent pas y accéder. Pour les desservir, créer un fichier `.env` contenant l’adresse LAN de cet hôte, par exemple :

```dotenv
LIGHT_DNS_BIND_IP=192.168.1.10
LIGHT_DNS_PORT=53
```

Choisir une adresse réellement présente sur l’hôte et un port libre. Autoriser UDP/TCP sur ce port dans le pare-feu du LAN ; ne pas exposer le résolveur à Internet. La publication fournie est IPv4 ; adapter les publications Compose pour desservir aussi les clients IPv6.

```bash
docker compose config --quiet
docker compose up -d --build
docker compose logs -f light-dns
docker compose exec light-dns light-dns health
```

Configurer ensuite le routeur/DHCP ou les clients pour utiliser l’adresse LAN du serveur comme DNS.

Le conteneur utilise l’UID/GID `65532:65532`, sans capacités privilégiées. Le dossier `docker/` est monté en lecture seule : ses fichiers doivent être lisibles et ses dossiers traversables par cet utilisateur. Les listes téléchargées persistent dans le volume `filter-data` ; la socket d’administration reste dans un `tmpfs` privé.

Après modification du TOML :

```bash
docker compose exec light-dns light-dns reload
```

Pour mettre à jour le code et reconstruire, en conservant la configuration locale et le fichier `.env` :

```bash
git pull --ff-only
docker compose up -d --build
```

Sauvegarder ses modifications locales avant la mise à jour et résoudre tout conflit de configuration avant de redémarrer. `docker compose restart light-dns` redémarre le service sans reconstruire ; `docker compose stop` l’arrête sans supprimer les listes persistées.

