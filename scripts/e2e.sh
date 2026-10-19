#!/bin/bash
# End-to-end test: the installed asides.app (built with dev hooks) driven through the `asides` CLI.
# WAV fixtures stand in for the microphone (`asides simulate`), a scripted session on a drawn
# screenshot for the screen (`asides simulate-measure`, played by the overlay's process without
# windows), a database of our own for Wispr Flow's history, and a stand-in agent (e2e-agent.c, run
# as `claude` in Terminal) for an agent session asides types into. The history goes to a folder of
# the test's own (`ASIDES_HOME`), and nothing is sent to the user's own sessions.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
FIX="$ROOT/tests/fixtures"
CLI="${ASIDES_CLI:-$HOME/.local/bin/asides}"
SUPPORT="$HOME/Library/Application Support/asides"
CONFIG="$SUPPORT/config.toml"
STATE="$SUPPORT/state.json"
# Dev-hooks builds read this instead of Wispr Flow's own history.
WISPR_DB="$SUPPORT/test-wispr-flow.sqlite"
WORK=$(mktemp -d)
export ASIDES_HOME="$WORK/asides-home"
mkdir -p "$SUPPORT" "$ASIDES_HOME"
# The user's settings and the app's state are put back afterwards.
cp "$CONFIG" "$WORK/config.toml.saved" 2>/dev/null || true
cp "$STATE" "$WORK/state.json.saved" 2>/dev/null || true
AGENT_TTY=""
DIRI_SESSION=""
cleanup() {
    [ -f "${AGENT_LOG:-}" ] && { echo "--- agent log"; cat "$AGENT_LOG"; }
    "$CLI" quit >/dev/null 2>&1 || true
    launchctl unsetenv ASIDES_HOME 2>/dev/null || true
    pkill -f "$WORK/bin/claude" 2>/dev/null || true
    # A run that stops early leaves its Diri session behind, exited, unless it's removed here.
    if [ -n "$DIRI_SESSION" ]; then "$DIRIJOR" session release "$DIRI_SESSION" --remove >/dev/null 2>&1 || true; fi
    if [ -n "$AGENT_TTY" ] && [ "$AGENT_TTY" != diri ]; then
        osascript -e "tell application \"Terminal\" to close (every window whose tabs's tty contains \"$AGENT_TTY\")" >/dev/null 2>&1 || true
    fi
    if [ -f "$WORK/clipboard.saved" ]; then pbcopy < "$WORK/clipboard.saved"; fi
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
# Multilingual (the fixtures are English and Romanian), standard detail, nothing heard at once.
set_config model '"large-v3-turbo"'
set_config languages '["en", "ro"]'
set_config annotation_detail '"standard"'
set_config mic_on_open 'false'
set_config wispr_flow 'false'
# Measuring is checked in points, whatever the user's [measure] units.
/usr/bin/python3 -I -c 'import re, sys
text = open(sys.argv[1]).read()
open(sys.argv[1], "w").write(re.sub(r"(?m)^units = .*$", "units = \"pt\"", text))' "$CONFIG"

pass() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { printf '  \033[31m✗\033[0m %s\n' "$1"; exit 1; }
check() { # description, command...
    local what=$1; shift
    if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi
}
contains() { grep -qF -- "$2" "$1"; }
asides_count() { "$CLI" history --json | /usr/bin/python3 -I -c 'import json, sys; print(len(json.load(sys.stdin)))'; }
# The id of the aside whose title has TEXT.
aside_id() { "$CLI" history -n 100 --json | /usr/bin/python3 -I -c 'import json, sys
print(next(a["id"] for a in json.load(sys.stdin) if sys.argv[1] in a["title"]))' "$1"; }
# Waits for the history to hold N asides, and writes the newest one's message to FILE.
wait_aside() { # n, file
    for _ in $(seq 1 240); do
        [ "$(asides_count)" -ge "$1" ] && "$CLI" show last > "$2" && return 0
        sleep 0.5
    done
    "$CLI" history; fail "timed out waiting for aside $1"
}

# Wispr Flow's History table, as far as asides reads it.
rm -f "$WISPR_DB" "$WISPR_DB-wal" "$WISPR_DB-shm"
/usr/bin/sqlite3 "$WISPR_DB" "PRAGMA journal_mode=WAL; CREATE TABLE History (transcriptEntityId TEXT PRIMARY KEY,
  timestamp DATETIME, app TEXT, status TEXT, formattedText TEXT, asrText TEXT, isArchived TINYINT(1) DEFAULT 0);" >/dev/null

# Use the build under test: another copy with the same bundle ID (say a release in /Applications)
# could otherwise be the one Launch Services opens for the CLI's links.
export ASIDES_APP="${ASIDES_APP:-$HOME/Applications/asides.app}"
"$CLI" quit >/dev/null 2>&1 || true
pkill -x asides-app 2>/dev/null || true
sleep 1
# The app, started by Launch Services, keeps its history where the test says.
launchctl setenv ASIDES_HOME "$ASIDES_HOME"
open -g -a "$ASIDES_APP"
for _ in $(seq 1 40); do "$CLI" status 2>/dev/null | grep -q "is running" && break; sleep 0.5; done

REPO="$WORK/shop"
mkdir -p "$REPO/src"
git -C "$REPO" init -q -b feat/checkout
echo "app" > "$REPO/src/app.js"

echo "Started"
check "the app runs" grep -q "is running" <<<"$("$CLI" status)"
check "no asides yet" grep -q "No asides yet" <<<"$("$CLI" history)"

echo "An English recording with a screenshot: kept in the history"
"$CLI" simulate "$FIX/en_bug.wav" --image "$FIX/screenshot.png" --at 6.5 >/dev/null
wait_aside 1 "$WORK/1.md"
check "English transcript" contains "$WORK/1.md" "padding on the summary card"
check "the screenshot where it was taken" grep -q '^!\[Screenshot\](/.*/history/[0-9-]*/images/1.png)$' "$WORK/1.md"
SHOT1=$(sed -n 's/^!\[Screenshot\](\(.*\))$/\1/p' "$WORK/1.md" | head -1)
check "the screenshot saved unchanged" cmp "$FIX/screenshot.png" "$SHOT1"
check "in ~/.asides (here, the test's folder)" grep -q "^$ASIDES_HOME/history/" <<<"$SHOT1"
check "kept, not sent (no session named)" grep -q "· kept" <<<"$("$CLI" history)"

echo "A Romanian recording"
"$CLI" simulate "$FIX/ro_bug.wav" >/dev/null
wait_aside 2 "$WORK/2.md"
check "Romanian transcript with diacritics" contains "$WORK/2.md" "Butonul de salvare nu face nimic după ce schimb parola."
check "the history lists it first" grep -q "Butonul de salvare" <<<"$("$CLI" history -n 1)"

echo "asides on the screen (a scripted session on a drawn screen)"
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
  {"tool": "measure"},
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
wait_aside 3 "$WORK/3.md"
check "measurements, in points, with the screen" contains "$WORK/3.md" "**Measured** in pt (screen: Simulated screen, 400 × 240 pt, @2x)"
check "a held distance between edges" contains "$WORK/3.md" "- Distance from (100, 140): 320 across (x 40 to x 360), 82 down (y 88 to y 170)"
check "an area snapped to the button" contains "$WORK/3.md" "- Area 100 × 24 (25:6) at (60, 170), snapped to the object inside"
check "a guide snapped to the text's edge" contains "$WORK/3.md" "**Guides:** x 60"
check "an annotation on an element, with its location" contains "$WORK/3.md" "**Location:** form > button#save.btn"
check "its intent" contains "$WORK/3.md" "**Intent:** fix"
check "its severity" contains "$WORK/3.md" "**Severity:** important"
check "an area annotation" contains "$WORK/3.md" "### 2. Area 110 × 50 at (250, 40)"
check "an arrow, from one element to another" contains "$WORK/3.md" "### 3. Arrow from button \"Save\" to heading \"Profile\""
check "the screenshot and the Shot tool's, in order" test "$(grep -c '^!\[Screenshot\]' "$WORK/3.md")" = 2
check "asides measure finds the button in the screenshot" grep -q "200 px wide × 48 px high" <<<"$("$CLI" measure "$SHOT" 100,320,240,100)"

echo "Talking in asides, frozen twice, with a Wispr Flow dictation"
set_config wispr_flow 'true'
sleep 3
dictate() { # id, text
    /usr/bin/sqlite3 "$WISPR_DB" "INSERT INTO History VALUES ('$1', '$(date -u '+%Y-%m-%d %H:%M:%S.000 +00:00')',
      'com.apple.Safari', 'formatted', '$2', '', 0);"
}
dictate e2e-0 "Dictated while nothing was open."
# The button measured and annotated (a click inside it) at once; something said about the
# annotation from its card at 1 s; back to the live screen; talking at 2 s (8 s of it); frozen
# again, and a distance measured at 14 s.
cat > "$WORK/talk.json" <<'JSON'
{"scale": 2, "actions": [
  {"tool": "measure"},
  {"drag": {"from": [50, 160], "to": [175, 205]}},
  {"tool": "annotate"}, {"click": {"at": [140, 185]}},
  "live",
  {"wait": 14},
  "freeze",
  {"hold": {"axis": "x", "at": [100, 140]}},
  {"wait": 2}
]}
JSON
"$CLI" simulate-measure "$SHOT" "$WORK/talk.json" >/dev/null
sleep 1
"$CLI" simulate "$FIX/en_bug.wav" --annotation 1 >/dev/null
sleep 1
"$CLI" simulate "$FIX/ro_bug.wav" >/dev/null
wait_aside 4 "$WORK/4.md"
line() { grep -nF -- "$1" "$WORK/4.md" | head -1 | cut -d: -f1; }
check "a click in a measured section annotates the section" contains "$WORK/4.md" "### 1. Area 100 × 24 at (60, 170)"
check "what was said about the annotation is in it" grep -q '^\*\*Said:\*\* .*padding on the summary card' "$WORK/4.md"
check "the pin's number where it was made, before what was said" grep -q '^\[1\] Butonul de salvare' "$WORK/4.md"
check "in the order it happened: talked, measured and annotated (after it), measured again" test \
    "$(line 'Butonul de salvare')" -lt "$(line '- Area 100 × 24')" -a "$(line '- Area 100 × 24')" -lt "$(line '### 1. Area')" \
    -a "$(line '### 1. Area')" -lt "$(line '- Distance from (100, 140)')"
check "a screenshot for each freeze" test "$(grep -c '^!\[Screenshot\]' "$WORK/4.md")" = 2
check "a dictation made before asides opened isn't in it" test -z "$(grep -rl 'Dictated while nothing' "$ASIDES_HOME/history")"
cat > "$WORK/wait.json" <<'JSON'
{"scale": 2, "actions": [{"annotate": {"area": {"x": 250, "y": 40, "width": 110, "height": 50}}}, {"wait": 5}]}
JSON
"$CLI" simulate-measure "$SHOT" "$WORK/wait.json" >/dev/null
sleep 2
dictate e2e-1 "The total is off by a cent."
wait_aside 5 "$WORK/5.md"
check "a dictation made while frozen joins the aside" contains "$WORK/5.md" "The total is off by a cent."
set_config wispr_flow 'false'

echo "Sending to an agent session (a stand-in agent)"
# What the user has on the clipboard comes back at the end; the test checks asides puts back its own.
pbpaste > "$WORK/clipboard.saved" 2>/dev/null || true
CLIPBOARD="asides e2e clipboard $$"
printf '%s' "$CLIPBOARD" | pbcopy
mkdir -p "$WORK/bin"
clang -O0 -o "$WORK/bin/claude" "$ROOT/scripts/e2e-agent.c"
AGENT_LOG="$WORK/agent.log"
: > "$AGENT_LOG"
# In Diri when it's installed (asides drives it with its command, no permission needed), else in
# Terminal (the first time, macOS asks whether asides may control Terminal: answer it).
DIRIJOR="/Applications/Diri.app/Contents/Resources/bin/dirijor"
if [ -x "$DIRIJOR" ]; then
    HOST=Diri
    DIRI_SESSION=$("$DIRIJOR" session run --cwd "$REPO" --title "asides e2e agent" --json -- /usr/bin/env \
        ASIDES_E2E_LOG="$AGENT_LOG" "$WORK/bin/claude" | grep -o 's_[0-9a-f]\{8,\}' | head -1)
    AGENT_TTY="diri"
else
    HOST=Terminal
    AGENT_TTY=$(osascript -e "tell application \"Terminal\" to tty of (do script \"cd '$REPO' && ASIDES_E2E_LOG='$AGENT_LOG' exec '$WORK/bin/claude'\")" 2>/dev/null || true)
fi
if [ -z "$AGENT_TTY" ]; then
    echo "  (skipped: no Diri, and Terminal can't be controlled from here)"
else
    SESSION=""
    for _ in $(seq 1 40); do
        SESSION=$("$CLI" sessions --json | /usr/bin/python3 -I -c 'import json, sys
repo = sys.argv[1]
print(next((str(s["pid"]) for s in json.load(sys.stdin) if s["dir"] == repo), ""))' "$(cd "$REPO" && pwd -P)")
        [ -n "$SESSION" ] && break
        sleep 0.5
    done
    check "asides finds it, as Claude Code in $HOST" test -n "$SESSION"
    check "listed with where it works" grep -q "Claude Code (shop) .* in $HOST" <<<"$("$CLI" sessions)"
    "$CLI" send "$(aside_id Butonul)" --to "$SESSION" > "$WORK/sent.txt"
    check "send says it got there" grep -q "Sent to Claude Code (shop)" "$WORK/sent.txt"
    check "the text typed in" grep -q 'Butonul de salvare' "$AGENT_LOG"
    check "then Return, on its own, which sends it" grep -Eq '^<CR>$|<ESC>\[201~<CR>' "$AGENT_LOG"
    check "the history says where it went" grep -q "→ Claude Code (shop)" <<<"$("$CLI" history)"
    : > "$AGENT_LOG"
    "$CLI" send "$(aside_id padding)" --to "$SESSION" >/dev/null
    if [ "$HOST" = Diri ]; then
        check "a screenshot goes in with Ctrl+V" grep -q '<16>' "$AGENT_LOG"
    else
        check "a screenshot goes in first, as its path, pasted on its own" \
            grep -q "^<ESC>\[200~$ASIDES_HOME/history/.*/images/1.png<ESC>\[201~<CR>$" "$AGENT_LOG"
    fi
    check "then the text" grep -q "padding on the summary card" "$AGENT_LOG"
    check "the user's clipboard is put back" test "$(pbpaste)" = "$CLIPBOARD"
    : > "$AGENT_LOG"
    "$CLI" simulate-measure "$SHOT" "$WORK/session.json" --to "$SESSION" >/dev/null
    for _ in $(seq 1 60); do grep -q "Area 110" "$AGENT_LOG" && break; sleep 0.5; done
    check "an aside from the screen goes straight to the session" grep -q "### 2. Area 110 × 50 at (250, 40)" "$AGENT_LOG"
    if [ "$HOST" = Diri ]; then
        "$DIRIJOR" session release "$DIRI_SESSION" --remove >/dev/null 2>&1 || true
        DIRI_SESSION=""
        AGENT_TTY=""
    fi
fi

echo "The MCP server (asides mcp)"
MCP=$(printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"e2e"}}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
    '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_asides","arguments":{"count":20}}}' \
    '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"get_aside","arguments":{"id":"last"}}}' \
    "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"measure_image\",\"arguments\":{\"path\":\"$SHOT\",\"area\":{\"x\":100,\"y\":320,\"width\":240,\"height\":100}}}}" \
    | "$CLI" mcp)
check "it introduces itself" grep -q '"serverInfo":{"name":"asides"' <<<"$MCP"
check "three read-only tools" test "$(grep -o '"readOnlyHint":true' <<<"$MCP" | wc -l | tr -d ' ')" = 3
check "list_asides lists the history" grep -q 'Butonul de salvare' <<<"$MCP"
check "get_aside sends the screenshots inline" grep -q '"type":"image"' <<<"$MCP"
check "measure_image measures" grep -q '200 px wide × 48 px high' <<<"$MCP"

echo "Status"
check "status says where asides go" grep -q "asides go to\|stay in the history" <<<"$("$CLI" status)"

echo "Quit"
"$CLI" quit >/dev/null
sleep 1
check "app quit" grep -q "not running" <<<"$("$CLI" status)"

echo
echo "Aside 4:"
sed 's/^/    /' "$WORK/4.md"
echo "All end-to-end checks passed."
