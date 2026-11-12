#!/bin/sh
# Bootstrap épinglé à la release, sans compilateur ni parsing JSON.
set -eu
umask 077
fail() { printf 'light-dns : %s\n' "$*" >&2; exit 1; }
[ "$#" -eq 0 ] || fail 'Usage : sh bootstrap.sh'
[ "$(id -u)" -eq 0 ] || fail 'Exécuter depuis un shell root.'
[ -f /etc/alpine-release ] || fail 'Alpine Linux requis.'
[ "$(uname -m)" = aarch64 ] || fail 'Architecture AArch64 requise.'
command -v rc-service >/dev/null 2>&1 || fail 'OpenRC requis.'
for tool in wget sha256sum mktemp; do
    command -v "$tool" >/dev/null 2>&1 || fail "Outil BusyBox absent : $tool"
done
version=v0.2.0-beta.2
name="light-dns-$version-aarch64-unknown-linux-musl"
base="https://github.com/kevstfnl/light-dns/releases/download/$version"
temporary=$(mktemp -d /tmp/light-dns-bootstrap.XXXXXX) || fail 'Création du dossier temporaire impossible.'
trap 'rm -rf -- "$temporary"' 0
trap 'exit 1' HUP INT TERM
cd "$temporary"
# Limite par fichier (blocs de 512 octets), plus expiration du transfert.
(
    ulimit -f 262144
    wget -T 60 -q -O SHA256SUMS "$base/SHA256SUMS" || fail 'Sommes indisponibles : vérifier réseau, certificats CA et release.'
    wget -T 60 -q -O "$name" "$base/$name" || fail 'Téléchargement du binaire impossible ou interrompu.'
) || exit 1
# Ne jamais transmettre des chemins arbitraires du manifeste au vérificateur.
awk -v name="$name" '$2 == name && length($1) == 64 && $1 !~ /[^0-9a-fA-F]/ { print $1 "  " name; count++ } END { if (count != 1) exit 1 }' SHA256SUMS > CHECKSUM || fail 'Somme SHA-256 absente, invalide ou dupliquée.'
sha256sum -c CHECKSUM || fail 'Intégrité du binaire incorrecte.'
chmod 0755 "$name"
"./$name" install || fail 'Installation échouée ; consulter les erreurs précédentes.'
