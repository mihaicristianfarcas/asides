#!/bin/bash
# Publishes a release's files behind the website's download buttons: a Cloudflare Worker that only
# serves static files, at https://download.asides.mihaicristian.dev, under names that stay the same
# from one version to the next (the website's links never change):
#
#   asides.dmg                            the Mac's disk image
#   asides-windows-{arm64,x64}-setup.exe  Windows' installers
#   asides-linux-{x86_64,aarch64}.deb     Debian, Ubuntu
#   asides-linux-{x86_64,aarch64}.rpm     Fedora, openSUSE
#   asides-linux-{x86_64,aarch64}.tar.gz  any other Linux (its install.sh)
#
# and each with its minisign signature (`.minisig`), as the release has them.
#
#   scripts/publish-downloads.sh DIST
#
# DIST holds the release's files, as the release workflow gathers them. Needs Wrangler (WRANGLER, or
# `wrangler` on the PATH), logged in to the Cloudflare account that owns the Worker (`wrangler
# login`), or CLOUDFLARE_API_TOKEN. A Worker serves files of 25 MiB at most.
set -euo pipefail
cd "$(dirname "$0")/.."

DIST=${1:?the folder with the files of the release}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
OUT=dist/download
rm -rf "$OUT"
mkdir -p "$OUT"

put() {
    local from="$DIST/$1" to="$OUT/$2"
    [ -f "$from" ] || { echo "No $from: the release isn't whole." >&2; exit 1; }
    cp "$from" "$to"
    if [ -f "$from.minisig" ]; then cp "$from.minisig" "$to.minisig"; fi
}

put "asides-$VERSION.dmg" asides.dmg
for arch in arm64 x64; do
    put "asides-$VERSION-windows-$arch-setup.exe" "asides-windows-$arch-setup.exe"
done
for arch in x86_64 aarch64; do
    deb=$([ "$arch" = x86_64 ] && echo amd64 || echo arm64)
    put "asides_${VERSION}_$deb.deb" "asides-linux-$arch.deb"
    put "asides-$VERSION-1.$arch.rpm" "asides-linux-$arch.rpm"
    put "asides-$VERSION-linux-$arch.tar.gz" "asides-linux-$arch.tar.gz"
done
cp packaging/download/_redirects "$OUT/"
ls -l "$OUT"
too_big=$(find "$OUT" -type f -size +25M)
if [ -n "$too_big" ]; then
    echo "Over the 25 MiB a Worker serves: $too_big" >&2
    exit 1
fi
# WRANGLER may be a command with arguments (npx --yes wrangler@VERSION): split on purpose.
# shellcheck disable=SC2086
${WRANGLER:-wrangler} deploy --config packaging/download/wrangler.jsonc
