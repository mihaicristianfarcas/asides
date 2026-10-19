#!/bin/bash
# Packs a notarized, stapled asides.app into a disk image with a link to Applications, for the
# website's download button. The app carries its own notarization ticket, so Gatekeeper accepts it
# from the image, offline too; the image itself is a plain, unsigned container.
# Usage: scripts/make-dmg.sh <asides.app> <output.dmg>
set -euo pipefail

APP="$1"
OUT="$2"
if ! xcrun stapler validate -q "$APP"; then
    echo "$APP has no notarization ticket; make the disk image from a \`make release\` build." >&2
    exit 1
fi

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/asides.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$OUT"
hdiutil create -quiet -volname asides -srcfolder "$STAGE" -fs HFS+ -format UDZO "$OUT"
echo "Ready: $OUT"
