"""Renders guide/overlay.png, the README's picture of asides: the real overlay over the website demo's
checkout page, with all it does: the pill recording, pins on elements and an area, a pin's card typed
in, a measurement and an arrow.

The screen is the demo's desktop from docs/index.html (rendered with Firefox), Safari alone over
readme-wallpaper.webp (beside this script); the overlay is
target/debug/asides-overlay, run on it in a window at the top left of your screen for a few seconds,
with a replay of the clicks and a capture (your terminal needs Screen Recording), dark whatever your
Mac's appearance. Needs Pillow.

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
# The screen, the demo's 14-inch MacBook Pro, in points, drawn at 2×.
WIDTH, HEIGHT = 1512, 945
WALLPAPER = pathlib.Path(__file__).resolve().parent / "readme-wallpaper.webp"
# Safari, alone, moves from where the demo puts it (24, 62) to the middle, below the pill.
SAFARI = (286, 104)
DX, DY = SAFARI[0] - 24, SAFARI[1] - 62

page = (DOCS / "index.html").read_text()
style = re.search(r"<style>(.*?)</style>", page, re.S).group(1)
desk = re.search(r'<div class="desk".*?(?=<div class="hud")', page, re.S).group(0)


def replace(text, old, new):
    assert old in text, f"not in the demo markup any more: {old}"
    return text.replace(old, new, 1)


# Safari alone (no agent's terminal), and nothing of the demo's own overlay: the real one draws on it.
desk = re.sub(r'<div class="win term.*?(?=<!-- asides on the frozen screen)', "", desk, flags=re.S)
html = f"""<!doctype html><html><head><meta charset="utf-8"><base href="{DOCS.as_uri()}/">
<style>{style}
html, body {{ margin: 0; overflow: hidden; }}
*, *::before, *::after {{ transition: none !important; animation: none !important; }}
/* The demo scales with its width: twice the points is a 2× screen. */
.display {{ width: {WIDTH * 2}px; border-radius: 0; }}
.apill, .ruler, .outline, .marker, .pointer {{ display: none !important; }}
.desk {{ background: #2e2a5c url({WALLPAPER.as_uri()}) 50% 50% / cover; }}
.safari {{ left: calc({SAFARI[0]} * var(--u)); top: calc({SAFARI[1]} * var(--u)); height: calc(700 * var(--u)); }}
</style></head><body><div class="display">{desk}</div></body></html>"""

# Where the demo's styles put them on its desktop, in points (Safari where the demo has it), moved
# with Safari: after changing the demo, look at the picture.
def at(x, y):
    return [x + DX, y + DY]


def frame(x, y, width, height):
    return {"x": x + DX, "y": y + DY, "width": width, "height": height}


place = "Safari · \"Checkout · meadow\""
page_of = {"app": "Safari", "window": "Checkout · meadow", "url": "localhost:5173/checkout"}
card = {**page_of, "name": 'group "Order summary"', "selector": "main > aside.summary",
        "frame": frame(584, 170, 320, 319.5)}
pay = {**page_of, "name": 'button "Pay now · $72.00"', "selector": "aside.summary > button.pay",
       "frame": frame(604, 417, 124, 51)}
city = {**page_of, "name": 'group "Delivery"', "selector": "form > div.pair", "frame": frame(85, 450, 458, 37)}


def pin(at, element):
    # Hovering asks the app for the element, and so does the pin: answer both.
    return [{"move": at, "wait": 250}, {"answer": element}, {"down": at}, {"up": at, "wait": 150},
            {"answer": element}]


def drag(points, **keys):
    """A press at the first point, moves through the others, let go at the last."""
    return [{"down": points[0], **keys}, *({"drag": p, **keys} for p in points[1:]),
            {"up": points[-1], **keys, "wait": 200}]


replay = [
    {"wait": 500},
    {"recording": 14},
    *pin(at(870, 186), card),
    *pin(at(700, 430), pay),
    # An area: a drag around the city and the postcode.
    {"move": at(76, 442), "wait": 150}, *drag([at(76, 442), at(300, 470), at(552, 495)]), {"answer": city},
    # Measure: a drag around the email field snaps to it.
    {"key": "m", "wait": 150}, *drag([at(76, 277.5), at(210, 295), at(552, 330)]),
    # Draw: an arrow (⇧) at the security code.
    {"key": "d", "wait": 150}, *drag([at(560, 700), at(510, 664), at(456, 624)], shift=True),
    {"key": "a", "wait": 200},
    # The Pay button's card, opened from its number: typed in, and a fix (⌘1).
    {"down": at(700, 430)}, {"up": at(700, 430), "wait": 250},
    {"type": "Keep the label on one line", "wait": 150}, {"key": "cmd+1", "wait": 250},
    {"move": [1500, 935], "wait": 300},
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
            "pixels": "screen-0.rgba", "width": WIDTH * 2, "height": HEIGHT * 2, "place": place, "menu_bar": 37,
        }],
        "color": "#FF2D55", "tolerance": 2, "identify": True,
        "record_shortcut": "⌃⌥Space", "freeze_shortcut": "⌃⌥P", "listen": True,
        "model": {"ready": True, "label": "English, 190 MB"},
        "targets": {
            "sessions": [{"id": "200", "label": "Claude Code (shop)", "app": "Terminal"}],
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
                   stdout=subprocess.DEVNULL, env={**os.environ, "ASIDES_WINDOW_APPEARANCE": "dark"})
    # 2048 pixels wide, in full color: in 256, the wallpaper's sky came out in bands.
    image = Image.open(tmp / "overlay.png").convert("RGB")
    image = image.resize((2048, round(image.height * 2048 / image.width)), Image.LANCZOS)
    image.save(OUT, optimize=True)
print(f"Wrote {OUT}")
