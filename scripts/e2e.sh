#!/bin/bash
# End-to-end test: the installed Asides.app (built with dev hooks) driven through the `asides`
# CLI in a throwaway git repo. WAV fixtures stand in for the microphone (`asides simulate`), a
# scripted session on a drawn screenshot for Measure & Annotate (`asides simulate-measure`, played
# by the overlay's process without windows), a database of our own for Wispr Flow's history, and
# scripts/e2e-mcp.py for a coding agent.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
FIX="$ROOT/tests/fixtures"
CLI="${ASIDES_CLI:-$HOME/.local/bin/asides}"
SUPPORT="$HOME/Library/Application Support/Asides"
CONFIG="$SUPPORT/config.toml"
# Dev-hooks builds read this instead of Wispr Flow's own history.
WISPR_DB="$SUPPORT/test-wispr-flow.sqlite"
WORK=$(mktemp -d)
mkdir -p "$SUPPORT"
# The agent test switches to batch delivery; the user's settings are put back afterwards, and so
# is the app's state (its binding: left on the test's deleted repo, the app couldn't record).
STATE="$SUPPORT/state.json"
cp "$CONFIG" "$WORK/config.toml.saved" 2>/dev/null || true
cp "$STATE" "$WORK/state.json.saved" 2>/dev/null || true
cleanup() {
    "$CLI" quit >/dev/null 2>&1 || true
    if [ -f "$WORK/config.toml.saved" ]; then cp "$WORK/config.toml.saved" "$CONFIG"; else rm -f "$CONFIG"; fi
    if [ -f "$WORK/state.json.saved" ]; then cp "$WORK/state.json.saved" "$STATE"; fi
    rm -f "$WISPR_DB" "$WISPR_DB-wal" "$WISPR_DB-shm"
    rm -rf "$WORK"
}
trap cleanup EXIT
# Sets a top-level setting, replacing the line (or array) the user's config.toml may already have.
set_config() {
    /usr/bin/python3 -I -c 'import re, sys; path, key, value = sys.argv[1:]
try: text = open(path).read()
except FileNotFoundError: text = ""
text = re.sub(rf"(?m)^{key} = (\[[^\]]*\]|.*)\n", "", text)
open(path, "w").write(f"{key} = {value}\n" + text)' "$CONFIG" "$1" "$2"
}
set_config agent_delivery '"each"'
set_config annotation_detail '"standard"'

pass() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { printf '  \033[31m✗\033[0m %s\n' "$1"; exit 1; }
check() { # description, command...
    local what=$1; shift
    if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi
}
contains() { grep -qF -- "$2" "$1"; }
wait_for() { # file, text
    for _ in $(seq 1 120); do
        [ -f "$1" ] && grep -qF -- "$2" "$1" && return 0
        sleep 0.5
    done
    echo "--- $1"; cat "$1" 2>/dev/null || true
    fail "timed out waiting for \"$2\""
}

# Wispr Flow's History table, as far as Asides reads it.
rm -f "$WISPR_DB" "$WISPR_DB-wal" "$WISPR_DB-shm"
/usr/bin/sqlite3 "$WISPR_DB" "PRAGMA journal_mode=WAL; CREATE TABLE History (transcriptEntityId TEXT PRIMARY KEY,
  timestamp DATETIME, app TEXT, status TEXT, formattedText TEXT, asrText TEXT, isArchived TINYINT(1) DEFAULT 0);" >/dev/null

# Use the build under test: another copy with the same bundle ID (say a release in /Applications)
# could otherwise be the one Launch Services opens for the CLI's links.
# The CLI sends its links to $ASIDES_APP (`open -a`) instead of whichever copy Launch Services picks.
export ASIDES_APP="${ASIDES_APP:-$HOME/Applications/Asides.app}"
"$CLI" quit >/dev/null 2>&1 || true
pkill -x Asides 2>/dev/null || true
sleep 1

REPO="$WORK/shop"
mkdir -p "$REPO/src"
git -C "$REPO" init -q -b feat/checkout
git -C "$REPO" config user.email e2e@example.com
git -C "$REPO" config user.name E2E
echo "app" > "$REPO/src/app.js"
git -C "$REPO" add .
git -C "$REPO" commit -q -m "Fix cart total rounding"
git -C "$REPO" remote add origin git@github.com:acme/shop.git
cd "$REPO/src" # binding from a subdirectory must bind the repo root

echo "Bind"
out=$("$CLI")
echo "$out" | sed 's/^/    /'
check "app started and bound to the repo root" grep -q "Asides ready in .*/shop$" <<<"$out"
check "no notes yet" grep -q "first recording creates a note" <<<"$out"
check ".asides/ excluded locally" grep -qx ".asides/" "$REPO/.git/info/exclude"

echo "New titled note: English recording + screenshot"
check "asides new acknowledges the title" grep -q 'new note "Checkout session"' <<<"$("$CLI" new "Checkout session")"
"$CLI" simulate "$FIX/en_bug.wav" --image "$FIX/screenshot.png" --at 6.5 >/dev/null
for _ in $(seq 1 120); do NOTE=$("$CLI" path 2>/dev/null) && break; sleep 0.5; done
wait_for "$NOTE" "## 1 ·"
check "note folder named after the title" grep -q "/.asides/[0-9-]* Checkout session/notes.md$" <<<"$NOTE"
check "front matter: title" contains "$NOTE" "title: Checkout session"
check "front matter: git branch" contains "$NOTE" "branch: feat/checkout"
check "front matter: git remote" contains "$NOTE" 'remote: "git@github.com:acme/shop.git"'
check "front matter: commit subject" contains "$NOTE" "commit_subject: Fix cart total rounding"
check "front matter: macOS version" grep -q "^  macos: " "$NOTE"
check "front matter: display" grep -q "^  display: [0-9]*x[0-9]* @" "$NOTE"
check "front matter: transcription" contains "$NOTE" "whisper large-v3-turbo (local)"
check "English transcript" contains "$NOTE" "padding on the summary card"
check "screenshot linked" grep -q '^!\[[0-9:]*\](images/01-[0-9]*\.png)$' "$NOTE"
check "screenshot saved unchanged" cmp "$FIX/screenshot.png" "$(dirname "$NOTE")/$(sed -n 's/^!\[.*\](\(images\/.*\))$/\1/p' "$NOTE" | head -1)"

echo "Append a Romanian recording to the latest note"
check "bind reports the latest note" grep -q 'appending to "Checkout session" (1 entry)' <<<"$("$CLI")"
"$CLI" simulate "$FIX/ro_bug.wav" >/dev/null
wait_for "$NOTE" "## 2 ·"
check "Romanian transcript with diacritics" contains "$NOTE" "Butonul de salvare nu face nimic după ce schimb parola."
check "metadata written only once" test "$(grep -c '^title:' "$NOTE")" = 1
check "same note" test "$("$CLI" path)" = "$NOTE"

echo "New untitled note: a generic title, not the first sentence"
"$CLI" new >/dev/null
"$CLI" simulate "$FIX/ro_bug.wav" >/dev/null
for _ in $(seq 1 120); do NOTE2=$("$CLI" path) && [ "$NOTE2" != "$NOTE" ] && break; sleep 0.5; done
wait_for "$NOTE2" "## 1 ·"
check "note folder named Untitled" grep -q "/.asides/[0-9-]* Untitled/notes.md$" <<<"$NOTE2"
check "front matter: generic title" contains "$NOTE2" "title: Untitled"
check "first note untouched" test "$(grep -c '^## ' "$NOTE")" = 2

echo "Wispr Flow dictations"
dictate() { # id, app, text
    /usr/bin/sqlite3 "$WISPR_DB" "INSERT INTO History VALUES ('$1', '$(date -u '+%Y-%m-%d %H:%M:%S.000 +00:00')',
      '$2', 'formatted', '$3', '', 0);"
}
dictate e2e-1 com.apple.Safari "The coupon field accepts expired codes."
wait_for "$NOTE2" "The coupon field accepts expired codes."
check "a dictation becomes an entry, marked as Wispr Flow" grep -q '^## 2 · [0-9:]* · Wispr Flow$' "$NOTE2"
set_config wispr_flow_apps '["Safari"]'
sleep 3
dictate e2e-3 com.tinyspeck.slackmacgap "Not about the app either."
dictate e2e-4 com.apple.Safari "The total is off by a cent."
wait_for "$NOTE2" "The total is off by a cent."
check "wispr_flow_apps leaves out other apps" test -z "$(grep -e "Not about the app" "$NOTE2")"
set_config wispr_flow_apps '[]'

echo "Measure & Annotate (a scripted session on a drawn screen)"
# A 400 × 240 pt screen at 2×: a light card with a title, a line of text and a blue button.
SHOT="$WORK/screen.png"
/usr/bin/python3 -I -c 'import struct, sys, zlib
w, h = 800, 480
rows = [[(255, 255, 255)] * w for _ in range(h)]
def fill(x, y, cw, ch, c):
    for j in range(y, y + ch):
        rows[j][x:x + cw] = [c] * cw
fill(80, 60, 640, 380, (0xf3, 0xf4, 0xf6)); fill(120, 100, 300, 28, (0x11, 0x18, 0x27))
fill(120, 160, 480, 16, (0x6b, 0x72, 0x80)); fill(120, 340, 200, 48, (0x25, 0x63, 0xeb))
fill(160, 356, 120, 16, (255, 255, 255))
raw = b"".join(b"\0" + bytes(v for px in row for v in px) for row in rows)
chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
open(sys.argv[1], "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))' "$SHOT"
cat > "$WORK/session.json" <<'JSON'
{"scale": 2, "actions": [
  {"click": {"at": [100, 140]}},
  {"drag": {"from": [50, 160], "to": [175, 205]}},
  {"guide": {"axis": "x", "at": [59, 50]}},
  {"annotate": {"at": [110, 182], "comment": "Align it with the text above", "intent": "fix",
    "severity": "important", "element": {"app": "Safari", "window": "Settings", "name": "button \"Save\"",
    "selector": "form > button#save.btn", "url": "http://localhost:3000/settings",
    "frame": {"x": 60, "y": 170, "width": 100, "height": 24}}}},
  {"annotate": {"area": {"x": 250, "y": 40, "width": 110, "height": 50}}},
  {"draw": {"points": [[110, 182], [200, 150], [300, 110]], "arrow": true, "comment": "Move it up here",
    "element": {"app": "Safari", "name": "button \"Save\""}, "target": {"app": "Safari", "name": "heading \"Profile\""}}},
  {"shot": {"from": [30, 150], "to": [190, 215]}}
]}
JSON
"$CLI" simulate-measure "$SHOT" "$WORK/session.json" >/dev/null
wait_for "$NOTE2" "· Screen"
check "a session outside a recording is an entry of its own" grep -q '^## 4 · [0-9:]* · Screen$' "$NOTE2"
check "measurements, in points, with the screen" contains "$NOTE2" "**Measured** in pt (screen: Simulated screen, 400 × 240 pt, @2x):"
check "a held distance between edges" contains "$NOTE2" "- Distance from (100, 140): 320 across (x 40 to x 360), 82 down (y 88 to y 170)"
check "an area snapped to the button" contains "$NOTE2" "- Area 100 × 24 (25:6) at (60, 170), snapped to the object inside"
check "a guide snapped to the text's edge" contains "$NOTE2" "- Guides: x 60"
check "an annotation on an element, with its location" contains "$NOTE2" "**Location:** form > button#save.btn"
check "its feedback, intent and severity" contains "$NOTE2" "**Intent:** fix · **Severity:** important"
check "an area mark" contains "$NOTE2" "### 2. Area 110 × 50 at (250, 40)"
check "an arrow, from one element to another" contains "$NOTE2" "### 3. Arrow from button \"Save\" to heading \"Profile\""
check "with where it goes" contains "$NOTE2" "**From → to:** (110, 182) → (300, 110)"
check "the screenshot and the Shot tool's, in the order they were made" \
    test "$(sed -n 's/^!\[.*\](\(images\/04-.*\.png\))$/\1/p' "$NOTE2" | wc -l | tr -d ' ')" = 2
SCREEN_PNG="$(dirname "$NOTE2")/$(sed -n 's/^!\[.*\](\(images\/04-.*\.png\))$/\1/p' "$NOTE2" | head -1)"
check "the annotated screenshot is saved" test -s "$SCREEN_PNG"
check "asides measure finds the button in the screenshot" grep -q "200 px wide × 48 px high" <<<"$("$CLI" measure "$SHOT" 100,320,240,100)"

echo "Status, export, git hygiene"
check "status shows the binding" grep -q "bound to .*/shop" <<<"$("$CLI" status)"
"$CLI" export --out "$WORK/note.zip" >/dev/null
zip_names() { /usr/bin/python3 -I -c 'import sys, zipfile; print("\n".join(i.filename + ("" if i.flag_bits & 0x800 or i.filename.isascii() else " !utf8") for i in zipfile.ZipFile(sys.argv[1]).infolist()))' "$1"; }
check "export zips notes.md" grep -q "/notes.md$" <<<"$(zip_names "$WORK/note.zip")"
check "zip names are UTF-8 flagged" test -z "$(grep "!utf8" <<<"$(zip_names "$WORK/note.zip")")"
check "zip has no macOS metadata files" test -z "$(grep -E "/\._|DS_Store" <<<"$(zip_names "$WORK/note.zip")")"
check "notes never show up in git status" test -z "$(git -C "$REPO" status --porcelain)"

echo "Switch between repos (the menu's Directory list)"
OTHER="$WORK/admin"
mkdir -p "$OTHER"
git -C "$OTHER" init -q -b main
recent() { /usr/bin/python3 -I -c 'import json, os, sys; print("\n".join(os.path.basename(d) for d in json.load(open(sys.argv[1]))["recent_dirs"][:2]))' "$HOME/Library/Application Support/Asides/state.json"; }
"$CLI" -C "$OTHER" >/dev/null
check "binding another repo puts it first" test "$(recent | tr '\n' ' ')" = "admin shop "
check "the new repo has no notes yet" grep -q "No notes in" <<<"$("$CLI" -C "$OTHER" status)"
"$CLI" >/dev/null
check "switching back reorders the list" test "$(recent | tr '\n' ' ')" = "shop admin "
check "and appends to the same note again" grep -q 'appending to "Untitled"' <<<"$("$CLI")"

echo "Coding agents (asides mcp)"
E2E_SHOT="$SHOT" /usr/bin/python3 -I "$ROOT/scripts/e2e-mcp.py" "$CLI" "$REPO" "$FIX" "$CONFIG" "$SUPPORT/listeners" "$OTHER"

echo "Quit"
"$CLI" quit >/dev/null
sleep 1
check "app quit" grep -q "not running" <<<"$("$CLI" status)"

echo
echo "Note 1:"
sed 's/^/    /' "$NOTE"
echo "All end-to-end checks passed."
