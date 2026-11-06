#!/bin/bash
# Has Apple notarize dist/asides.app (signed with a Developer ID and a secure timestamp: `make sign`),
# staples the ticket, and packs it for the release: dist/asides-<version>.zip (the update feed's and
# the cask's) and dist/asides-<version>.dmg (the website's download).
#
# notarytool's credentials: an App Store Connect API key (APPLE_API_KEY_PATH, APPLE_API_KEY_ID and
# APPLE_API_ISSUER, as the release workflow has them), or else a keychain profile (NOTARY_PROFILE,
# default asides-notary: `xcrun notarytool store-credentials asides-notary --apple-id … --team-id …`).
set -euo pipefail
cd "$(dirname "$0")/.."

APP=dist/asides.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ZIP="dist/asides-$VERSION.zip"
if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    NOTARY=(--key "$APPLE_API_KEY_PATH" --key-id "${APPLE_API_KEY_ID:?}" --issuer "${APPLE_API_ISSUER:?}")
else
    NOTARY=(--keychain-profile "${NOTARY_PROFILE:-asides-notary}")
fi

codesign --verify --deep --strict "$APP"
if ! codesign -dv "$APP" 2>&1 | grep -q '^Authority=Developer ID Application:'; then
    echo "$APP isn't signed with a Developer ID Application certificate (make sign SIGN_ID=…)" >&2
    exit 1
fi

rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "Submitting $ZIP to Apple's notary service (usually a few minutes)…"
RESULT=$(xcrun notarytool submit "$ZIP" "${NOTARY[@]}" --wait --timeout 1h --output-format json)
STATUS=$(plutil -extract status raw -o - - <<<"$RESULT")
if [ "$STATUS" != "Accepted" ]; then
    ID=$(plutil -extract id raw -o - - <<<"$RESULT")
    echo "Notarization finished with status \"$STATUS\". Apple's log:" >&2
    xcrun notarytool log "$ID" "${NOTARY[@]}" >&2 || true
    exit 1
fi

# The ticket attached, so Gatekeeper accepts the app offline; then the stapled app zipped again.
xcrun stapler staple "$APP"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
spctl --assess --type execute --verbose=2 "$APP"
scripts/make-dmg.sh "$APP" "dist/asides-$VERSION.dmg"
echo "Ready: $ZIP and dist/asides-$VERSION.dmg"
