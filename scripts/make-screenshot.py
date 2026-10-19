"""Renders guide/overlay.png, the README's picture of Asides: the real overlay, over the website demo's
checkout page, with two pins, a measurement and the pill recording.

The screen is the demo's desktop from docs/index.html (rendered with Firefox); the overlay is
target/debug/asides-overlay, run on it in a window at the top left of your screen for a few seconds,
with a replay of the clicks and a capture (your terminal needs Screen Recording). Needs Pillow.

Usage: python3 scripts/make-screenshot.py
"""

import json
import os
import pathlib
import re
import subprocess
import tempfile

from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
OUT = ROOT / "guide" / "overlay.png"
FIREFOX = os.environ.get("FIREFOX", "/Applications/Firefox.app/Contents/MacOS/firefox")
# The screen, in points, drawn at 2×.
WIDTH, HEIGHT = 1024, 640

page = (DOCS / "index.html").read_text()
style = re.search(r"<style>(.*?)</style>", page, re.S).group(1)
desk = re.search(r'<div class="desk" data-app="terminal".*?(?=<div class="hud")', page, re.S).group(0)


def replace(text, old, new):
    assert old in text, f"not in the demo markup any more: {old}"
    return text.replace(old, new, 1)


# Safari in front, and nothing of the demo's own overlay: the real one draws on it.
desk = replace(desk, 'data-app="terminal"', 'data-app="safari"')
desk = replace(desk, '<div class="win safari inactive">', '<div class="win safari">')
html = f"""<!doctype html><html><head><meta charset="utf-8"><base href="{DOCS.as_uri()}/">
<style>{style}
html, body {{ margin: 0; overflow: hidden; }}
*, *::before, *::after {{ transition: none !important; animation: none !important; }}
/* The demo scales with its width: twice the points is a 2× screen. */
.display {{ width: {WIDTH * 2}px; border-radius: 0; }}
.win.term, .pill, .outline, .pin, .pointer {{ display: none !important; }}
</style></head><body><div class="display">{desk}</div></body></html>"""

# Where the demo's styles put them, in points: after changing the demo, look at the picture.
place = "Safari · \"Checkout · meadow\""
page_of = {"app": "Safari", "window": "Checkout · meadow", "url": "localhost:5173/checkout"}
card = {**page_of, "name": 'group "Order summary"', "selector": "main > aside.summary",
        "frame": {"x": 464, "y": 142, "width": 262, "height": 278}}
pay = {**page_of, "name": 'button "Pay now · $72.00"', "selector": "aside.summary > button.pay",
       "frame": {"x": 481, "y": 355, "width": 104, "height": 48}}


def pin(at, element):
    # Hovering asks the app for the element, and so does the pin: answer both.
    return [{"move": at, "wait": 250}, {"answer": element}, {"down": at}, {"up": at, "wait": 150},
            {"answer": element}]


replay = [
    {"wait": 500},
    {"recording": 14},
    *pin([700, 157], card),
    *pin([575, 364], pay),
    # Measure: a drag around the email field snaps to it.
    {"key": "m", "wait": 150}, {"down": [58, 221]}, {"drag": [200, 240]}, {"drag": [444, 268]},
    {"up": [444, 268], "wait": 200},
    {"key": "a", "wait": 200}, {"move": [930, 600], "wait": 300},
    {"capture": "overlay.png"},
    {"key": "escape"}, {"key": "escape", "wait": 300},
]

with tempfile.TemporaryDirectory() as tmp:
    tmp = pathlib.Path(tmp)
    (tmp / "profile").mkdir()
    source = tmp / "desk.html"
    source.write_text(html)
    shot = tmp / "desk.png"
    subprocess.run(
        [FIREFOX, "--headless", "--no-remote", "--profile", str(tmp / "profile"),
         f"--window-size={WIDTH * 2},{HEIGHT * 2}", "--screenshot", str(shot), source.as_uri()],
        check=True, capture_output=True,
    )
    (tmp / "screen-0.rgba").write_bytes(Image.open(shot).convert("RGBA").tobytes())
    spec = {
        "screens": [{
            "display": 1, "screen": {"name": "Built-in Retina Display", "width": WIDTH, "height": HEIGHT, "scale": 2},
            "origin": [0, 0], "frame": {"x": 0, "y": 0, "width": WIDTH, "height": HEIGHT},
            "pixels": "screen-0.rgba", "width": WIDTH * 2, "height": HEIGHT * 2, "place": place, "menu_bar": 28,
        }],
        "color": "#FF2D55", "tolerance": 2, "identify": True,
        "record_shortcut": "⌃⌥Space", "freeze_shortcut": "⌃⌥P", "listen": True,
        "model": {"ready": True, "label": "English, 190 MB"},
        "targets": {
            "sessions": [{"id": "200", "label": "Claude Code · shop", "app": "Terminal"}],
            "folders": [{"path": "/Users/me/Developer/shop", "name": "shop"}],
            "agents": ["claude", "codex", "opencode"], "chosen": {"kind": "session", "id": "200"},
        },
        # The window shows the part of the screen it covers: all of it, at the screen's origin.
        "windowed": {"x": 0, "y": 0, "width": WIDTH, "height": HEIGHT},
        "replay": replay,
    }
    (tmp / "session.json").write_text(json.dumps(spec, ensure_ascii=False))
    subprocess.run(["cargo", "build", "-q", "-p", "asides-overlay"], cwd=ROOT, check=True)
    subprocess.run([ROOT / "target" / "debug" / "asides-overlay", tmp], check=True, timeout=60,
                   stdout=subprocess.DEVNULL)
    # 256 colors keep it a fifth of the size, with no difference to see at the README's width.
    image = Image.open(tmp / "overlay.png").convert("RGB")
    image.quantize(colors=256, method=Image.Quantize.FASTOCTREE).save(OUT, optimize=True)
print(f"Wrote {OUT}")
