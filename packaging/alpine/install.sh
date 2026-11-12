#!/bin/sh
# Depuis une archive vérifiée ; la gestion réside dans le binaire.
set -eu
[ "$#" -eq 0 ] || { echo 'Usage : sh install.sh' >&2; exit 1; }
archive_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
[ -f "$archive_dir/light-dns" ] && [ ! -L "$archive_dir/light-dns" ] || {
    echo 'Binaire light-dns absent ou symbolique.' >&2; exit 1;
}
exec "$archive_dir/light-dns" install
