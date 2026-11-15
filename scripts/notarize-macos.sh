#!/bin/bash
# Has Apple notarize dist/asides.app (signed with a Developer ID and a secure timestamp: `make sign`),
# staples the ticket, and packs it for the release: dist/asides-<version>.zip (the update feed's and
# the cask's) and dist/asides-<version>.dmg (the website's download), itself signed as the app is,
# notarized and stapled.
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

DMG="dist/asides-$VERSION.dmg"

# Has Apple's notary service check $1 (a zip or a disk image), and waits for its answer.
notarize() {
    local result status id
    echo "Submitting $1 to Apple's notary service (usually a few minutes)…"
    result=$(xcrun notarytool submit "$1" "${NOTARY[@]}" --wait --timeout 1h --output-format json)
    status=$(plutil -extract status raw -o - - <<<"$result")
    if [ "$status" != "Accepted" ]; then
        id=$(plutil -extract id raw -o - - <<<"$result")
        echo "Notarization of $1 finished with status \"$status\". Apple's log:" >&2
        xcrun notarytool log "$id" "${NOTARY[@]}" >&2 || true
        exit 1
    fi
}

# Attaches $1's ticket, so Gatekeeper accepts it offline. Stapling fetches the ticket from Apple's
# CloudKit, and a connection lost on the way fails it (error 68, in a release run): a few tries,
# further apart each time, since nothing says when it would work.
staple() {
    local try
    for try in 1 2 3 4; do
        xcrun stapler staple "$1" && return
        if [ "$try" = 4 ]; then
            echo "Stapling $1 failed $try times: run this again (it's notarized, the ticket is Apple's)." >&2
            exit 1
        fi
        sleep $((try * 15))
    done
}

codesign --verify --deep --strict "$APP"
# Only -dvv lists the certificates. Read it whole first: sed would stop reading early otherwise, and
# codesign, still writing, fails on the closed pipe (pipefail). The disk image is signed with it too.
IDENTITY=$(sed -n 's/^Authority=\(Developer ID Application: .*\)$/\1/p' <<<"$(codesign -dvv "$APP" 2>&1)")
if [ -z "$IDENTITY" ]; then
    echo "$APP isn't signed with a Developer ID Application certificate (make sign SIGN_ID=…)" >&2
    exit 1
fi

rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
notarize "$ZIP"
# The ticket attached, then the stapled app zipped again.
staple "$APP"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
spctl --assess --type execute --verbose=2 "$APP"

# The disk image signed as the app is, notarized and stapled too: Gatekeeper then accepts the
# download itself, not only the app in it.
scripts/make-dmg.sh "$APP" "$DMG"
codesign --sign "$IDENTITY" --timestamp "$DMG"
notarize "$DMG"
staple "$DMG"
spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"
echo "Ready: $ZIP and $DMG"
