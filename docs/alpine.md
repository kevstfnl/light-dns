# Alpine Linux AArch64 — Raspberry Pi 5

Installation prévue pour Alpine 64 bits (`uname -m` : `aarch64`), stockage persistant **sys** et OpenRC. Aucun compilateur n’est requis. Le bootstrap et le binaire utilisent les outils système BusyBox ; HTTPS exige des certificats CA valides. Si le bundle système manque, installer `ca-certificates-bundle` avec `apk`, sans contourner TLS.

## Installation rapide

Depuis un shell root, utiliser la commande du [README](../README.md#installation-rapide--alpine-linux--raspberry-pi-5). Elle exige une release publique : un brouillon GitHub ne fonctionne pas avec les téléchargements anonymes.

Le bootstrap épinglé télécharge le binaire et `SHA256SUMS`, vérifie la somme du seul binaire attendu dans un dossier temporaire privé, puis lance `light-dns install`. Il nettoie ses téléchargements après succès ou échec. Le script distant s’exécute en root : inspecter sa source et utiliser la méthode suivante si cette confiance n’est pas appropriée.

## Installation manuelle avec vérification

Depuis un shell root :

```sh
version=v0.2.0-beta.2
base="https://github.com/kevstfnl/light-dns/releases/download/$version"
name="light-dns-$version-aarch64-unknown-linux-musl"
mkdir -m 700 light-dns-download
cd light-dns-download
wget -T 60 "$base/$name.tar.gz" "$base/SHA256SUMS" "$base/bootstrap.sh" "$base/$name" "$base/light-dns.initd"
cat bootstrap.sh
sha256sum -c SHA256SUMS
mkdir extracted
tar -xzf "$name.tar.gz" -C extracted
cat "extracted/$name/install.sh"
cat "extracted/$name/light-dns.initd"
sh "extracted/$name/install.sh"
cd ..
rm -r light-dns-download
```

Arrêter immédiatement si la vérification échoue. Comparer aussi la somme publiée via un canal indépendant si disponible ; un manifeste obtenu du même compte GitHub ne protège pas contre la compromission de ce compte. Ne pas utiliser `--no-check-certificate`.

## Service et configuration

Le service est activé dans le runlevel `default` et démarre automatiquement à la première installation. L’écoute initiale est uniquement `127.0.0.1:53`, en UDP et TCP. Un conflit sur ce port fait échouer l’installation et déclenche le rollback ; aucune configuration réseau n’est modifiée.

```sh
rc-service light-dns status
rc-service light-dns stop
rc-service light-dns start
rc-service light-dns restart
light-dns --socket /run/light-dns/control.sock health
light-dns --socket /run/light-dns/control.sock health --upstream
light-dns --socket /run/light-dns/control.sock stats
light-dns --socket /run/light-dns/control.sock cache clear
```

Éditer `/etc/light-dns/config.toml` pour choisir l’adresse LAN et les upstreams. Pour IPv6, utiliser `[adresse]:53`. Un changement d’écoute exige un redémarrage. Sinon `rc-service light-dns reload` relit explicitement le TOML ; aucun watcher n’est installé. Configurer ensuite les clients ou le DHCP manuellement et ne pas exposer le DNS à Internet.

Le démon s’exécute sous `light-dns:light-dns`, jamais root. Le binaire et le service appartiennent à root, le TOML est lisible par le service et non modifiable par lui. Les données résident dans `/var/lib/light-dns/`, le cache dans `filters/` et la socket privée dans `/run/light-dns/control.sock`. OpenRC recrée ce dernier dossier au démarrage. Les messages passent par `logger` vers le journal système.

Seule la capacité `CAP_NET_BIND_SERVICE` est accordée, lorsque le port configuré est inférieur au seuil privilégié du noyau. `libcap-setcap` est installé seulement si son outil est absent et nécessaire ; le système de fichiers doit accepter les capacités étendues.

## Mise à jour et rollback

```sh
light-dns version
light-dns update --check
light-dns update
light-dns update --version v0.2.0-beta.2
```

Les releases publiques stables et bêta compatibles sont comparées selon SemVer, sans rétrogradation automatique. `--version` permet de choisir explicitement une version antérieure. `--check` ne demande pas root et ne modifie rien. Les limites GitHub anonymes ou une panne réseau produisent une erreur explicite.

Les téléchargements HTTPS et leurs sommes sont vérifiés avant l’arrêt. Le binaire et le service sont remplacés atomiquement, le TOML et les listes sont conservés. Le service redémarre seulement s’il fonctionnait déjà, puis la santé locale UDP/TCP est contrôlée. Une installation répétée ne réactive pas un service volontairement désactivé.

En cas d’échec, l’ancienne version et l’état du service sont restaurés. Les sauvegardes et le journal restent dans `/etc/light-dns/.transaction` si le rollback échoue. Après interruption brutale, la prochaine commande de gestion mutante reprend la récupération avant toute nouvelle opération. Ne pas supprimer ces sauvegardes manuellement. Un service dont l’état OpenRC est indéterminé exige une vérification de l’administrateur.

`/etc/light-dns/install-state.toml` enregistre les comptes créés. Un verrou root dans `/run/light-dns-management.lock` sérialise les opérations ; son fichier reste présent pour éviter les courses entre propriétaires du verrou.

## Désinstallation

```sh
light-dns uninstall
light-dns uninstall --purge
```

La première commande arrête le service, retire ses activations OpenRC, le script et le binaire ; elle garde TOML, données et compte. La purge demande de taper `PURGE` sur le terminal avant toute suppression. Elle retire les dossiers gérés et les comptes dont la création est attestée ; les comptes préexistants sont conservés. Aucun paquet partagé n’est désinstallé et aucun chemin externe mentionné dans le TOML n’est supprimé.

## Validation mainteneur

La CI AMD64 et l’inspection ELF statique permettent la publication publique d’une bêta, **sans revendiquer une validation Alpine AArch64 ou Raspberry Pi physique**. Sur le Pi réel, le mainteneur doit encore vérifier : OpenRC et reboot, UID non-root et port 53, UDP/TCP et IPv4/IPv6, fallback, listes HTTPS, toutes les commandes DNS, installation répétée, mise à jour et rollback, désinstallation et purge. Reporter les résultats dans `TODO.MD` seulement après ces essais.

Le mode diskless n’est pas couvert par cette installation autonome ; gérer explicitement la persistance Alpine avant de l’utiliser.
