#!/bin/bash
# End-to-end test on Linux: asides-app built with dev hooks, driven through the `asides` command, in
# a desktop session (it needs DISPLAY; on Wayland, the portal's screenshot isn't touched). WAV
# fixtures stand in for the microphone (`asides simulate`), a scripted session on a drawn screenshot
# for the screen (`asides simulate-measure`), and a stand-in agent (e2e-agent.c, run as `claude`, in
# tmux, and in xterm typed through `asides-keys`) for a session asides types into. The history and
# settings are the test's own (`ASIDES_HOME`, `XDG_*`), and nothing is sent to the user's sessions.
#
#   scripts/e2e-linux.sh [target/debug]
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
FIX="$ROOT/tests/fixtures"
BIN=$(cd "${1:-${CARGO_TARGET_DIR:-$ROOT/target}/debug}" && pwd)
CLI="$BIN/asides"
WORK=$(mktemp -d /var/tmp/asides-e2e.XXXXXX)
export ASIDES_HOME="$WORK/asides-home"
# The model stays where it is (it's big): the user's, or `ASIDES_E2E_MODELS`.
MODELS="${ASIDES_E2E_MODELS:-${XDG_DATA_HOME:-$HOME/.local/share}/asides/models}"
export XDG_CONFIG_HOME="$WORK/config" XDG_DATA_HOME="$WORK/data" XDG_STATE_HOME="$WORK/state" XDG_CACHE_HOME="$WORK/cache"
mkdir -p "$ASIDES_HOME" "$XDG_CONFIG_HOME/asides" "$XDG_DATA_HOME/asides" "$XDG_STATE_HOME" "$XDG_CACHE_HOME"
ln -s "$MODELS" "$XDG_DATA_HOME/asides/models"
CONFIG="$XDG_CONFIG_HOME/asides/config.toml"
APP_PID=""
TERM_PIDS=()
TMUX_SOCKET="$WORK/tmux.sock"
cleanup() {
    local status=$?
    [ -f "${AGENT_LOG:-}" ] && { echo "--- agent log"; cat "$AGENT_LOG"; }
    # Failed: what asides itself said.
    [ "$status" != 0 ] && [ -f "$XDG_STATE_HOME/asides/asides.log" ] && { echo "--- asides log"; tail -40 "$XDG_STATE_HOME/asides/asides.log"; }
    "$CLI" quit >/dev/null 2>&1 || true
    [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
    for pid in "${TERM_PIDS[@]:-}"; do [ -n "$pid" ] && kill "$pid" 2>/dev/null || true; done
    tmux -S "$TMUX_SOCKET" kill-server 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

cat > "$CONFIG" <<'TOML'
model = "large-v3-turbo"
languages = ["en", "ro"]
annotation_detail = "standard"
mic_on_open = false
wispr_flow = false

[measure]
units = "pt"
TOML

pass() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { printf '  \033[31m✗\033[0m %s\n' "$1"; exit 1; }
check() { # description, command...
    local what=$1; shift
    if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi
}
contains() { grep -qF -- "$2" "$1"; }
asides_count() { "$CLI" history --json | python3 -I -c 'import json, sys; print(len(json.load(sys.stdin)))'; }
aside_id() { "$CLI" history -n 100 --json | python3 -I -c 'import json, sys
print(next(a["id"] for a in json.load(sys.stdin) if sys.argv[1] in a["title"]))' "$1"; }
wait_aside() { # n, file
    for _ in $(seq 1 480); do
        [ "$(asides_count)" -ge "$1" ] && "$CLI" show last > "$2" && return 0
        sleep 0.5
    done
    "$CLI" history; fail "timed out waiting for aside $1"
}

RUST_BACKTRACE=1 "$BIN/asides-app" >"$WORK/app.log" 2>&1 &
APP_PID=$!
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
check "kept, not sent (no session named)" grep -q "· kept" <<<"$("$CLI" history)"

echo "A Romanian recording"
"$CLI" simulate "$FIX/ro_bug.wav" >/dev/null
wait_aside 2 "$WORK/2.md"
check "Romanian transcript with diacritics" contains "$WORK/2.md" "Butonul de salvare nu face nimic după ce schimb parola."

echo "asides on the screen (a scripted session on a drawn screen)"
SHOT="$WORK/screen.png"
python3 -I -c 'import struct, sys, zlib
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
    "severity": "important", "element": {"app": "Firefox", "window": "Settings", "name": "button \"Save\"",
    "selector": "form > button#save.btn", "url": "http://localhost:3000/settings",
    "frame": {"x": 60, "y": 170, "width": 100, "height": 24}}}},
  {"annotate": {"area": {"x": 250, "y": 40, "width": 110, "height": 50}}},
  {"draw": {"points": [[110, 182], [200, 150], [300, 110]], "arrow": true, "comment": "Move it up here",
    "element": {"app": "Firefox", "name": "button \"Save\""}, "target": {"app": "Firefox", "name": "heading \"Profile\""}}},
  {"shot": {"from": [30, 150], "to": [190, 215]}}
]}
JSON
"$CLI" simulate-measure "$SHOT" "$WORK/session.json" >/dev/null
wait_aside 3 "$WORK/3.md"
check "measurements, in points, with the screen" contains "$WORK/3.md" "**Measured** in pt (screen: Simulated screen, 400 × 240 pt, @2x)"
check "a held distance between edges" contains "$WORK/3.md" "- Distance from (100, 140): 320 across (x 40 to x 360), 82 down (y 88 to y 170)"
check "an area snapped to the button" contains "$WORK/3.md" "- Area 100 × 24 (25:6) at (60, 170), snapped to the object inside"
check "an annotation on an element, with its location" contains "$WORK/3.md" "**Location:** form > button#save.btn"
check "an arrow, from one element to another" contains "$WORK/3.md" "### 3. Arrow from button \"Save\" to heading \"Profile\""
check "the screenshot and the Shot tool's, in order" test "$(grep -c '^!\[Screenshot\]' "$WORK/3.md")" = 2
check "labels drawn on the exported image" test -s "$(sed -n 's/^!\[Screenshot\](\(.*\))$/\1/p' "$WORK/3.md" | head -1)"
check "asides measure finds the button" grep -q "200 px wide × 48 px high" <<<"$("$CLI" measure "$SHOT" 100,320,240,100)"

echo "Sending to agent sessions (a stand-in agent)"
mkdir -p "$WORK/bin"
cc -O0 -o "$WORK/bin/claude" "$ROOT/scripts/e2e-agent.c"
session_in() { # repo: the session's id, once asides finds it
    for _ in $(seq 1 40); do
        local found
        found=$("$CLI" sessions --json | python3 -I -c 'import json, sys
print(next((str(s["pid"]) for s in json.load(sys.stdin) if s["dir"] == sys.argv[1]), ""))' "$(cd "$1" && pwd -P)")
        [ -n "$found" ] && { printf '%s' "$found"; return 0; }
        sleep 0.5
    done
    return 1
}
sends_to() { # how: the session in REPO takes text, Return and a screenshot's path
    local how=$1 session=$2
    "$CLI" send "$(aside_id Butonul)" --to "$session" > "$WORK/sent.txt"
    check "$how: send says it got there" grep -q "Sent to Claude Code" "$WORK/sent.txt"
    check "$how: the text typed in, whole" grep -q 'Butonul de salvare nu face nimic după ce schimb parola' "$AGENT_LOG"
    check "$how: then Return, which sends it" grep -Eq '^<CR>$|<ESC>\[201~<CR>' "$AGENT_LOG"
    : > "$AGENT_LOG"
    "$CLI" send "$(aside_id padding)" --to "$session" >/dev/null
    check "$how: a screenshot goes in first, as its path, on its own" \
        grep -q "<ESC>\[200~$ASIDES_HOME/history/.*/images/1.png<ESC>\[201~" "$AGENT_LOG"
    check "$how: then the text" grep -q "padding on the summary card" "$AGENT_LOG"
    : > "$AGENT_LOG"
}

AGENT_LOG="$WORK/agent.log"
: > "$AGENT_LOG"
if command -v tmux >/dev/null; then
    tmux -S "$TMUX_SOCKET" -f /dev/null new-session -d -s e2e -c "$REPO" "ASIDES_E2E_LOG='$AGENT_LOG' exec '$WORK/bin/claude'"
    SESSION=$(session_in "$REPO")
    check "tmux: asides finds it" test -n "$SESSION"
    check "tmux: listed with where it works" grep -q "Claude Code (shop) .* in tmux" <<<"$("$CLI" sessions)"
    sends_to tmux "$SESSION"
    "$CLI" simulate-measure "$SHOT" "$WORK/session.json" --to "$SESSION" >/dev/null
    for _ in $(seq 1 60); do grep -q "Area 110" "$AGENT_LOG" && break; sleep 0.5; done
    check "tmux: an aside from the screen goes straight to the session" grep -q "### 2. Area 110 × 50 at (250, 40)" "$AGENT_LOG"
    for _ in $(seq 1 60); do grep -Eq '^<CR>$|<ESC>\[201~<CR>' "$AGENT_LOG" && break; sleep 0.5; done
    tmux -S "$TMUX_SOCKET" kill-server 2>/dev/null || true
    sleep 0.5
    : > "$AGENT_LOG"
else
    echo "  (skipped tmux: not installed)"
fi

if command -v xterm >/dev/null; then
    xterm -e env -C "$REPO" ASIDES_E2E_LOG="$AGENT_LOG" "$WORK/bin/claude" &
    TERM_PIDS+=($!)
    SESSION=$(session_in "$REPO")
    check "a terminal: asides finds it" test -n "$SESSION"
    if "$CLI" sessions | grep "Claude Code (shop)" | grep -q "can't type"; then
        echo "  (asides-keys may not type here: sudo setcap cap_sys_ptrace+ep $BIN/asides-keys; skipped)"
    else
        sends_to "a terminal" "$SESSION"
    fi
else
    echo "  (skipped a terminal: no xterm)"
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
check "list_asides lists the history" grep -q 'Butonul de salvare' <<<"$MCP"
check "get_aside sends the screenshots inline" grep -q '"type":"image"' <<<"$MCP"
check "measure_image measures" grep -q '200 px wide × 48 px high' <<<"$MCP"

echo "Quit"
"$CLI" quit >/dev/null
sleep 1
check "app quit" grep -q "not running" <<<"$("$CLI" status)"
APP_PID=""

echo
echo "All end-to-end checks passed."
