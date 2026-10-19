#!/bin/bash
# Builds asides.app (with the CLI inside), signs it with a Developer ID certificate and a secure
# timestamp, has Apple notarize it, staples the ticket, and zips it for a GitHub release and packs it
# into a disk image for the website. Writes the update feed (dist/appcast.json, a release asset that
# installed copies read) with the release notes, from dist/notes-<version>.md.
#
# One-time setup: store notarytool credentials under a profile name (an app-specific password from
# account.apple.com), then run `make release`:
#   xcrun notarytool store-credentials asides-notary --apple-id <Apple ID> --team-id <Team ID>
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE="${NOTARY_PROFILE:-asides-notary}"
IDENTITY="$(scripts/signing-identity.sh)"
case "$IDENTITY" in
    "Developer ID Application: "*) ;;
    *) echo "Notarization needs a \"Developer ID Application\" certificate; found: $IDENTITY" >&2; exit 1 ;;
esac
TEAM_ID=$(sed -n 's/.*(\([A-Z0-9]*\))$/\1/p' <<<"$IDENTITY")
if ! xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null 2>&1; then
    echo "No notarytool credentials named \"$PROFILE\". Store them once with:" >&2
    echo "  xcrun notarytool store-credentials $PROFILE --apple-id <your Apple ID> --team-id $TEAM_ID" >&2
    echo "or set NOTARY_PROFILE to an existing profile." >&2
    exit 1
fi

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NOTES="dist/notes-$VERSION.md"
if [ ! -s "$NOTES" ]; then
    echo "Write the release notes first, in $NOTES: Markdown, what changes for people using asides." >&2
    echo "asides shows them in What's New, and they're the GitHub release's notes." >&2
    exit 1
fi

make bundle TIMESTAMP=--timestamp SIGN_ID="$IDENTITY"
APP=dist/asides.app
ZIP="dist/asides-$VERSION.zip"

rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "Submitting $ZIP to Apple's notary service (usually a few minutes)…"
RESULT=$(xcrun notarytool submit "$ZIP" --keychain-profile "$PROFILE" --wait --output-format json)
STATUS=$(plutil -extract status raw -o - - <<<"$RESULT")
if [ "$STATUS" != "Accepted" ]; then
    ID=$(plutil -extract id raw -o - - <<<"$RESULT")
    echo "Notarization finished with status \"$STATUS\". Apple's log:" >&2
    xcrun notarytool log "$ID" --keychain-profile "$PROFILE" >&2
    exit 1
fi

# Attach the ticket so Gatekeeper can verify the app offline, then zip the stapled app.
xcrun stapler staple "$APP"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
spctl --assess --type execute --verbose=2 "$APP"
SHA=$(shasum -a 256 "$ZIP" | cut -d' ' -f1)
# The Homebrew cask installs this exact zip from the GitHub release.
sed -i '' -e "s/^  version \".*\"/  version \"$VERSION\"/" -e "s/^  sha256 \".*\"/  sha256 \"$SHA\"/" Casks/asides.rb
# The website's download button serves a disk image of the same stapled app.
scripts/make-dmg.sh "$APP" "dist/asides-$VERSION.dmg"

# The update feed: this release, then the newest few before it, as published.
REPO=mihaicristianfarcas/asides
PUBLISHED=$(curl -fsL "https://github.com/$REPO/releases/latest/download/appcast.json" || echo '{"releases": []}')
OLDEST_MACOS=$(plutil -extract LSMinimumSystemVersion raw packaging/Info.plist)
python3 -I - "$REPO" "$VERSION" "$ZIP" "$SHA" "$OLDEST_MACOS" "$NOTES" "$PUBLISHED" >dist/appcast.json <<'PY'
import datetime, json, os, sys
repo, version, zip_path, sha, oldest_macos, notes, published = sys.argv[1:]
release = {
    "version": version,
    "url": f"https://github.com/{repo}/releases/download/v{version}/{os.path.basename(zip_path)}",
    "size": os.path.getsize(zip_path),
    "sha256": sha,
    "minimum_system_version": oldest_macos,
    "published": datetime.datetime.now(datetime.timezone.utc).date().isoformat(),
    "notes": open(notes, encoding="utf-8").read().strip(),
}
before = [r for r in json.loads(published).get("releases", []) if r.get("version") != version]
print(json.dumps({"releases": [release] + before[:4]}, indent=2, ensure_ascii=False))
PY

echo "SHA-256: $SHA"
echo "Ready: $ZIP, dist/asides-$VERSION.dmg and dist/appcast.json (the update feed)"
echo "Publish all three in one release:"
echo "  git tag v$VERSION && git push --tags"
echo "  gh release create v$VERSION $ZIP dist/asides-$VERSION.dmg dist/appcast.json --title \"asides $VERSION\" --notes-file $NOTES"
echo "Updated Casks/asides.rb; commit it after publishing the GitHub release."
echo "Publish the disk image behind the website's download button with \`make publish-dmg\`."
