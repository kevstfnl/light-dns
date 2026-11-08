#!/bin/sh
# Outils de build Linux AMD64, versions et sommes figées. Écrit seulement dans DEST.
set -eu
[ "$#" -eq 1 ] || { echo "Usage: $0 DEST" >&2; exit 1; }
[ "$(uname -sm)" = 'Linux x86_64' ] || { echo 'Linux AMD64 requis' >&2; exit 1; }
mkdir -p "$1"
tools_dir=$(CDPATH='' cd -- "$1" && pwd)

fetch() {
    name=$1 url=$2 digest=$3
    curl --proto '=https' --tlsv1.2 -fSL --retry 3 --connect-timeout 20 --max-time 600 "$url" -o "$tools_dir/$name.tar"
    printf '%s  %s\n' "$digest" "$tools_dir/$name.tar" | sha256sum -c -
    mkdir -p "$tools_dir/$name"
    tar -xf "$tools_dir/$name.tar" --strip-components=1 -C "$tools_dir/$name"
    rm "$tools_dir/$name.tar"
}

fetch cargo-zigbuild \
    https://github.com/rust-cross/cargo-zigbuild/releases/download/v0.23.4/cargo-zigbuild-x86_64-unknown-linux-musl.tar.xz \
    9e3cf73485edbd45905c8aadbc0fdf869c7ddc3848f0c898229f2680db52e44b
fetch zig \
    https://ziglang.org/download/0.15.2/zig-x86_64-linux-0.15.2.tar.xz \
    02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239
fetch cmake \
    https://github.com/Kitware/CMake/releases/download/v4.4.5/cmake-4.4.5-linux-x86_64.tar.gz \
    ed85135c5a5135b45e50423cb406dea04ca3338da0b8ef3c9ede963559680be0
printf '%s\n' "Outils installés dans $tools_dir (cargo-zigbuild, zig, cmake/bin)."
