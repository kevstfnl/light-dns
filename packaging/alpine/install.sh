#!/bin/sh
# À lancer depuis une archive de release vérifiée, jamais via curl | sh.
set -eu
umask 022

fail() { printf '%s\n' "$*" >&2; exit 1; }

[ "$#" -eq 0 ] || fail "Usage : sh install.sh"
[ "$(id -u)" -eq 0 ] || fail "Exécuter avec les privilèges root."
[ -f /etc/alpine-release ] || fail "Cette installation exige Alpine Linux."
[ "$(uname -m)" = aarch64 ] || fail "Cette archive exige aarch64."
command -v rc-service >/dev/null 2>&1 || fail "OpenRC est requis."

archive_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
for file in light-dns config.toml light-dns.initd; do
    [ -f "$archive_dir/$file" ] && [ ! -L "$archive_dir/$file" ] || fail "Archive incomplète : $file"
done

# Tous les chemins d'installation sont fixes et sous contrôle de root.
for path in /usr/local /usr/local/bin /usr/local/bin/light-dns /etc/light-dns \
    /etc/light-dns/config.toml /etc/init.d/light-dns /var/lib/light-dns \
    /var/lib/light-dns/filters /run/light-dns; do
    [ ! -L "$path" ] || fail "Lien symbolique refusé : $path"
done
if [ -e /etc/init.d/light-dns ]; then
    if rc-service light-dns status >/dev/null 2>&1; then
        fail "Arrêter le service avant mise à jour : rc-service light-dns stop"
    else
        status=$?
        [ "$status" -eq 3 ] || fail "État OpenRC indéterminé ($status) ; vérifier le service."
    fi
fi

apk add --no-cache ca-certificates libcap-setcap
if ! id -u light-dns >/dev/null 2>&1; then
    if ! grep -q '^light-dns:' /etc/group; then
        addgroup -S light-dns
    fi
    adduser -S -D -H -h /var/lib/light-dns -s /sbin/nologin -G light-dns light-dns
fi
[ "$(id -u light-dns)" -ne 0 ] || fail "L'utilisateur light-dns ne doit pas être root."
[ "$(id -gn light-dns)" = light-dns ] || fail "Le groupe principal doit être light-dns."

install -d -m 0755 -o root -g root /usr/local/bin /etc/light-dns
install -d -m 0750 -o light-dns -g light-dns /var/lib/light-dns
install -d -m 0700 -o light-dns -g light-dns /var/lib/light-dns/filters /run/light-dns
if [ ! -e /etc/light-dns/config.toml ]; then
    install -m 0644 "$archive_dir/config.toml" /etc/light-dns/config.toml
fi

# Préparer le nouvel inode et sa capacité avant le remplacement atomique.
staged_binary=$(mktemp /usr/local/bin/.light-dns.XXXXXX)
trap 'rm -f -- "$staged_binary"' EXIT
install -m 0755 "$archive_dir/light-dns" "$staged_binary"
setcap 'cap_net_bind_service=+ep' "$staged_binary"
install -m 0755 "$archive_dir/light-dns.initd" /etc/init.d/light-dns
mv -f -- "$staged_binary" /usr/local/bin/light-dns

printf '%s\n' \
    'Installation terminée ; configuration existante conservée.' \
    'Éditer /etc/light-dns/config.toml et vérifier que le port 53 est disponible.' \
    'Activer : rc-update add light-dns default' \
    'Démarrer : rc-service light-dns start' \
    'Vérifier : light-dns --socket /run/light-dns/control.sock health'
