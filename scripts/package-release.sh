#!/bin/sh
# Empaqueter un binaire déjà compilé, sans jamais l'exécuter.
set -eu
[ "$#" -eq 1 ] || { echo "Usage: $0 vX.Y.Z" >&2; exit 1; }
tag=$1
metadata=$(cargo metadata --locked --no-deps --format-version 1)
version=$(printf '%s\n' "$metadata" | jq -er '.packages[] | select(.name == "light-dns") | .version')
[ "$tag" = "v$version" ] || { echo 'Tag et version Cargo différents' >&2; exit 1; }
printf '%s\n' "$tag" | LC_ALL=C grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$' || exit 1
target=aarch64-unknown-linux-musl
binary="target/$target/release/light-dns"
LC_ALL=C readelf -h "$binary" | grep -Eq 'Machine: +AArch64' || { echo 'ELF AArch64 requis' >&2; exit 1; }
program_headers=$(LC_ALL=C readelf -l "$binary")
dynamic=$(LC_ALL=C readelf -d "$binary")
if printf '%s\n' "$program_headers" | grep -q INTERP || printf '%s\n' "$dynamic" | grep -q '(NEEDED)'; then
    echo 'Le binaire doit être statique, sans interpréteur ELF' >&2
    exit 1
fi

mkdir -p target/release-assets
stage=$(mktemp -d target/release-assets/.stage.XXXXXX)
trap 'rm -rf -- "$stage"' EXIT
name="light-dns-$tag-$target"
mkdir "$stage/$name"
install -m 0755 "$binary" "$stage/$name/light-dns"
install -m 0755 packaging/alpine/install.sh "$stage/$name/install.sh"
install -m 0755 packaging/alpine/light-dns.initd "$stage/$name/light-dns.initd"
install -m 0644 packaging/alpine/config.toml "$stage/$name/config.toml"
install -m 0644 LICENSE "$stage/$name/LICENSE"
install -m 0644 docs/alpine.md "$stage/$name/INSTALL.md"
git rev-parse HEAD > "$stage/$name/REVISION"
printf '%s\n' "$tag" > "$stage/$name/VERSION"
tar -czf "target/release-assets/$name.tar.gz" -C "$stage" "$name"
install -m 0755 "$binary" "target/release-assets/$name"
install -m 0755 packaging/alpine/bootstrap.sh target/release-assets/bootstrap.sh
install -m 0755 packaging/alpine/light-dns.initd target/release-assets/light-dns.initd
(cd target/release-assets && sha256sum "$name" "$name.tar.gz" bootstrap.sh light-dns.initd > SHA256SUMS)
printf '%s\n' "target/release-assets/$name.tar.gz"
