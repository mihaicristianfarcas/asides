"""Renders the clips in the asides window's Help page and setup's How it works step
(crates/asides-overlay/help/): the real overlay on the website demo's desktop, pinning, talking,
switching sessions, measuring, and the aside arriving in Claude Code.

The screen is the demo's desktop from docs/index.html (rendered with Firefox); the overlay is
target/debug/asides-overlay, run on it in a window at the top left of your screen for a few seconds
per clip, with a replay of the clicks and keys and a capture after each (your terminal needs Screen
Recording). A clip is the frames captured, cropped to the part that matters, and in clips.json what
happens between them: where the pointer goes (the window draws it gliding there), what it clicks or
drags, the key pressed, what's said. Needs Pillow.

Usage: python3 scripts/make-help.py [--keep DIR]  (--keep: the full captures too, to look at)
"""

import json
import math
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageChops, ImageDraw

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
OUT = ROOT / "crates" / "asides-overlay" / "help"
FIREFOX = os.environ.get("FIREFOX", "/Applications/Firefox.app/Contents/MacOS/firefox")
# The screen, the demo's 14-inch MacBook Pro, in points, drawn at 2×; its menu bar.
WIDTH, HEIGHT = 1512, 945
MENU_BAR = 37

page = (DOCS / "index.html").read_text()
style = re.search(r"<style>(.*?)</style>", page, re.S).group(1)
desk = re.search(r'<div class="desk".*?(?=<div class="hud")', page, re.S).group(0)


def replace(text, old, new):
    assert old in text, f"not in the demo markup any more: {old}"
    return text.replace(old, new, 1)


def html(markup):
    return f"""<!doctype html><html><head><meta charset="utf-8"><base href="{DOCS.as_uri()}/">
<style>{style}
html, body {{ margin: 0; overflow: hidden; }}
*, *::before, *::after {{ transition: none !important; animation: none !important; }}
/* The demo scales with its width: twice the points is a 2× screen. */
.display {{ width: {WIDTH * 2}px; border-radius: 0; }}
.apill, .ruler, .outline, .marker, .pointer {{ display: none !important; }}
</style></head><body><div class="display">{markup}</div></body></html>"""


# The demo's last frame: the aside in Claude Code's chat, in Terminal beside Safari, still in front.
sent = desk
# Before it: nothing sent yet. The real overlay draws on it.
safari = re.sub(r'<p class="you aside">.*?</p>\s*', "", desk, flags=re.S)
safari = re.sub(r'<p class="reply">.*?</p>\s*', "", safari, flags=re.S)

# Where the demo's styles put things, in points: after changing the demo, look at the frames
# (--keep).
page_of = {"app": "Safari", "window": "Checkout · meadow", "url": "localhost:5173/checkout"}
card = {**page_of, "name": 'group "Order summary"', "selector": "main > aside.summary",
        "frame": {"x": 584, "y": 170, "width": 320, "height": 319.5}}
pay = {**page_of, "name": 'button "Pay now · $72.00"', "selector": "aside.summary > button.pay",
       "frame": {"x": 604, "y": 417, "width": 124, "height": 51}}
CARD = [770, 225]
# Dragged over the Pay button, pinning an area.
AREA = [[594, 407], [660, 448], [740, 478]]
# Measured: the email field (84, 277.5, 460 × 38), then a circle drawn around the Pay button.
FIELD = [240, 297]
FIELD_DRAG = [[76, 277.5], [210, 295], [552, 330]]
CIRCLE = [[666 + 98 * math.cos(a), 443 + 50 * math.sin(a)]
          for a in (math.radians(200 + 15 * i) for i in range(25))]
# The pill's session name, in its window's points (one past its edge), once Tab picked Codex.
NAME = [104, 20]
# The pointer out of the way: on the page, under the summary card.
REST = [880, 540]
CLOSE = [{"key": "escape"}, {"key": "escape", "wait": 300}]


def drag(path, element=None, held=None):
    """Drags along `path`; `held`: a capture before letting go (measuring, the size drawn shows then)."""
    steps = [{"move": path[0]}, {"down": path[0]}, *({"drag": p} for p in path[1:])]
    if held:
        steps += [{"wait": 250}, {"capture": held}]
    steps += [{"up": path[-1], "wait": 250}]
    return steps + ([{"answer": element}, {"wait": 250}] if element else [])


def click(at, element=None):
    # Hovering asks the app for the element, and so does the pin: answer both.
    answer = [{"answer": element}] if element else []
    return [{"move": at, "wait": 200}, *answer, {"down": at}, {"up": at, "wait": 150}, *answer, {"wait": 250}]


# Each run: the screen, and the replay (captures named for the clip's frames), in the screen's points.
RUNS = {
    "basics": ("safari", [
        {"wait": 600}, {"recording": 2}, {"move": REST, "wait": 200}, {"capture": "basics-0.png"},
        *click(CARD, card), {"capture": "basics-1.png"},
        *drag(AREA, pay), {"capture": "basics-2.png"},
        {"recording": 9}, {"level": 0.55}, {"move": REST, "wait": 250}, {"capture": "basics-3.png"},
        *CLOSE,
    ]),
    "target": ("safari", [
        {"wait": 600}, {"recording": 3}, {"move": REST, "wait": 200}, {"capture": "target-0.png"},
        {"key": "tab", "wait": 500}, {"capture": "target-1.png"},
        {"move": NAME, "toolbar": True, "wait": 500}, {"capture": "target-2.png"},
        *CLOSE,
    ]),
    "card": ("safari", [
        {"wait": 600}, {"recording": 4}, *click(CARD, card), {"move": REST, "wait": 200}, {"capture": "card-0.png"},
        *click(CARD), {"capture": "card-1.png"},
        {"type": "Give it room under the header", "wait": 300}, {"capture": "card-2.png"},
        {"key": "cmd+2", "wait": 300}, {"capture": "card-3.png"},
        *CLOSE,
    ]),
    "tools": ("safari", [
        # Listening, as asides opens by default: muted, the circle drawn would open its card.
        {"wait": 600}, {"recording": 5}, {"move": REST}, {"key": "m", "wait": 200}, {"move": FIELD, "wait": 300},
        {"capture": "tools-0.png"},
        *drag(FIELD_DRAG, held="tools-1.png"),
        {"key": "d", "wait": 200}, *drag(CIRCLE), {"move": REST, "wait": 300}, {"capture": "tools-2.png"},
        *CLOSE,
    ]),
    "keys": ("safari", [
        {"wait": 600}, {"move": REST, "wait": 200}, {"key": "?", "wait": 400}, {"capture": "keys-0.png"},
        *CLOSE,
    ]),
}

# The clips: each one's part of the screen (x, y, width, height, in points), and its frames: the
# capture, then what happens before it shows (a `key` pressed, the pointer dragged along `path`, then
# going to `pointer`, a `click` there), how long it stays (ms), and what's being said meanwhile. `key` "open"
# and "send" are the shortcut, as set. The first frame's are what happens as the clip starts over.
# After the frames, anything else the window needs: `radius`, its corners'; `bare`, no outline.
BASICS = (560, 36, 740, 600)
# The pill's corners (`glass_panel` in the overlay's ui.rs), and its height: its buttons, padding and
# border (`toolbar_panel`).
PILL_RADIUS = 12
PILL_HEIGHT = 38
CLIPS = {
    "basics": (BASICS, [
        {"image": "basics-0", "pointer": REST, "key": "open", "hold": 1800, "step": 0},
        {"image": "basics-1", "pointer": CARD, "click": True, "hold": 1200, "step": 1},
        {"image": "basics-2", "path": AREA, "hold": 1400, "step": 1},
        {"image": "basics-3", "pointer": REST, "hold": 3400, "step": 2,
         "say": "This card [1] needs room under the header, and this button [2] wraps."},
        {"image": "sent", "key": "send", "hold": 3600, "step": 3},
    ]),
    # The pill alone, as it is (found on the frame, its corners cut out).
    "pill": ("pill", [{"image": "basics-0", "hold": 0}], {"radius": PILL_RADIUS, "bare": True}),
    "target": ((450, 36, 560, 470), [
        {"image": "target-0", "pointer": [900, 400], "hold": 1500},
        {"image": "target-1", "key": "target", "hold": 1700},
        {"image": "target-2", "pointer": [738, 66], "hold": 3000},
    ]),
    # Wider to the right: a pin's card opens beside it.
    "card": ((506, 36, 768, 540), [
        {"image": "card-0", "pointer": REST, "hold": 1300},
        {"image": "card-1", "pointer": CARD, "click": True, "hold": 1000},
        {"image": "card-2", "pointer": REST, "hold": 1200},
        {"image": "card-3", "key": "⌘2", "hold": 2600},
    ]),
    "tools": ((60, 36, 918, 520), [
        {"image": "tools-0", "pointer": FIELD, "key": "measure", "hold": 1800},
        {"image": "tools-1", "path": FIELD_DRAG, "hold": 1900},
        {"image": "tools-2", "path": CIRCLE, "pointer": REST, "key": "draw", "hold": 2600},
    ]),
    "keys": ((579, 36, 490, 480), [{"image": "keys-0", "hold": 0}]),
}


def screenshot(markup, out, tmp):
    profile = tmp / "profile"
    profile.mkdir(exist_ok=True)
    source = tmp / "desk.html"
    source.write_text(html(markup))
    subprocess.run(
        [FIREFOX, "--headless", "--no-remote", "--profile", str(profile),
         f"--window-size={WIDTH * 2},{HEIGHT * 2}", "--screenshot", str(out), source.as_uri()],
        check=True, capture_output=True,
    )


def run(name, screen, replay, tmp):
    run_dir = tmp / name
    run_dir.mkdir()
    (run_dir / "screen-0.rgba").write_bytes(Image.open(tmp / f"{screen}.png").convert("RGBA").tobytes())
    spec = {
        "screens": [{
            "display": 1, "screen": {"name": "Built-in Retina Display", "width": WIDTH, "height": HEIGHT, "scale": 2},
            "origin": [0, 0], "frame": {"x": 0, "y": 0, "width": WIDTH, "height": HEIGHT},
            "pixels": "screen-0.rgba", "width": WIDTH * 2, "height": HEIGHT * 2,
            "place": "Safari · \"Checkout · meadow\"", "menu_bar": MENU_BAR,
        }],
        "color": "#FF2D55", "tolerance": 2, "identify": True,
        "record_shortcut": "⌃⌥Space", "freeze_shortcut": "⌃⌥P", "listen": True,
        "model": {"ready": True, "label": "English, 190 MB"},
        "targets": {
            "sessions": [
                {"id": "200", "label": "Claude Code (shop)", "app": "Terminal"},
                {"id": "201", "label": "Codex (api)", "app": "Ghostty"},
                {"id": "202", "label": "OpenCode (blog)", "app": "Diri"},
            ],
            "folders": [{"path": "/Users/me/Developer/shop", "name": "shop"},
                        {"path": "/Users/me/Developer/api", "name": "api"},
                        {"path": "/Users/me/Developer/blog", "name": "blog"}],
            "agents": ["claude", "codex", "opencode"], "chosen": {"kind": "session", "id": "200"},
        },
        # The window shows the part of the screen it covers: all of it, at the screen's origin.
        "windowed": {"x": 0, "y": 0, "width": WIDTH, "height": HEIGHT},
        "replay": replay,
    }
    (run_dir / "session.json").write_text(json.dumps(spec, ensure_ascii=False))
    # The overlay dark, whatever the Mac's appearance, as the clips always were.
    subprocess.run([ROOT / "target" / "debug" / "asides-overlay", run_dir], check=True, timeout=60,
                   stdout=subprocess.DEVNULL, env={**os.environ, "ASIDES_WINDOW_APPEARANCE": "dark"})
    for png in run_dir.glob("*.png"):
        shutil.copy(png, tmp / "frames" / png.name)


def pill_box(frame, desk):
    """The pill on `frame` (pixels): where it differs from the desktop, near the top, as tall as it is
    from its bottom (over a dark sky, its top edge hardly differs)."""
    top = (0, 0, frame.width, 400)
    changed = ImageChops.difference(frame.crop(top), desk.crop(top)).convert("L").point(lambda v: 255 if v > 24 else 0)
    x0, _, x1, y1 = changed.getbbox()
    return x0 / 2, y1 / 2 - PILL_HEIGHT, (x1 - x0) / 2, PILL_HEIGHT


def cut_corners(image, radius):
    """`image` with its corners `radius` pixels round, transparent outside them."""
    big = 4
    mask = Image.new("L", (image.width * big, image.height * big), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, mask.width - 1, mask.height - 1), radius * big, fill=255)
    image = image.convert("RGBA")
    image.putalpha(mask.resize(image.size, Image.LANCZOS))
    return image


def in_clip(point, box):
    return [round(point[0] - box[0], 1), round(point[1] - box[1], 1)]


def main():
    keep = pathlib.Path(sys.argv[sys.argv.index("--keep") + 1]) if "--keep" in sys.argv else None
    subprocess.run(["cargo", "build", "-q", "-p", "asides-overlay"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        (tmp / "frames").mkdir()
        screenshot(safari, tmp / "safari.png", tmp)
        screenshot(sent, tmp / "frames" / "sent.png", tmp)
        for name, (screen, replay) in RUNS.items():
            run(name, screen, replay, tmp)
        if keep:
            shutil.copytree(tmp / "frames", keep, dirs_exist_ok=True)
        if OUT.exists():
            shutil.rmtree(OUT)
        OUT.mkdir(parents=True)
        manifest = {}
        desk = Image.open(tmp / "safari.png").convert("RGB")
        for name, (box, frames, *extra) in CLIPS.items():
            options = extra[0] if extra else {}
            if box == "pill":
                box = pill_box(Image.open(tmp / "frames" / f"{frames[0]['image']}.png").convert("RGB"), desk)
            x, y, w, h = box
            listed = []
            for i, frame in enumerate(frames):
                image = Image.open(tmp / "frames" / f"{frame['image']}.png").convert("RGB")
                out = f"{name}-{i}.webp" if len(frames) > 1 else f"{name}.webp"
                image = image.crop((round(x * 2), round(y * 2), round((x + w) * 2), round((y + h) * 2)))
                if options.get("bare"):
                    image = cut_corners(image, options["radius"] * 2)
                image.save(OUT / out, quality=84 if image.mode == "RGB" else 92, method=6)
                entry = {"image": out, "hold": frame["hold"]}
                if "pointer" in frame:
                    entry["pointer"] = in_clip(frame["pointer"], box)
                if "path" in frame:
                    entry["path"] = [in_clip(p, box) for p in frame["path"]]
                for field in ("click", "key", "say", "step"):
                    if field in frame:
                        entry[field] = frame[field]
                listed.append(entry)
            manifest[name] = {"size": [w, h], **options, "frames": listed}
        # A frame a line.
        clips = ",\n".join(
            f'  "{name}": {{' + "".join(f'"{k}": {json.dumps(v)}, ' for k, v in clip.items() if k != "frames")
            + '"frames": [\n'
            + ",\n".join("    " + json.dumps(f, ensure_ascii=False) for f in clip["frames"]) + "\n  ]}"
            for name, clip in manifest.items()
        )
        (OUT / "clips.json").write_text("{\n" + clips + "\n}\n")
    total = sum(f.stat().st_size for f in OUT.iterdir())
    print(f"Wrote {OUT} ({total // 1024} KB)")


if __name__ == "__main__":
    main()
