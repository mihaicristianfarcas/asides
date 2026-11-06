"""Renders the clips in the asides window's Help page and setup's How it works step
(crates/asides-overlay/help/): the real overlay on the website demo's desktop, pinning, talking,
switching sessions, measuring, and the aside arriving in Claude Code.

The screen is the demo's desktop, scripts/demo-desktop/index.html (rendered with Firefox); the overlay is
target/debug/asides-overlay, run on it in a window at the top left of your screen for a few seconds
per clip, with a replay of the clicks and keys and a capture after each (your terminal needs Screen
Recording). A clip is the frames captured, cropped to the part that matters, and in clips.json what
happens between them: where the pointer goes (the window draws it gliding there), what it clicks or
drags, the key pressed, what's said. Needs Pillow.

On Windows it writes Windows' clips (crates/asides-overlay/help-windows/, which Windows builds embed):
the same desktop with Edge and Windows Terminal, the real overlay as Windows draws it, at the
display's scale (100% gives 1× clips; the window needs 1512 × 945 points of screen).

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

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEMO = ROOT / "scripts" / "demo-desktop"
WINDOWS = sys.platform == "win32"
OUT = ROOT / "crates" / "asides-overlay" / ("help-windows" if WINDOWS else "help")
FIREFOX = os.environ.get("FIREFOX", r"C:\Program Files\Mozilla Firefox\firefox.exe" if WINDOWS
                         else "/Applications/Firefox.app/Contents/MacOS/firefox")
OVERLAY = ROOT / "target" / "debug" / ("asides-overlay.exe" if WINDOWS else "asides-overlay")
# The screen, the demo's 14-inch MacBook Pro, in points, drawn at 2×; its menu bar.
WIDTH, HEIGHT = 1512, 945
MENU_BAR = 37

page = (DEMO / "index.html").read_text(encoding="utf-8")
style = re.search(r"<style>(.*?)</style>", page, re.S).group(1)
desk = re.search(r'<div class="desk".*?(?=<div class="hud")', page, re.S).group(0)


def replace(text, old, new):
    assert old in text, f"not in the demo markup any more: {old}"
    return text.replace(old, new, 1)


def html(markup):
    return f"""<!doctype html><html><head><meta charset="utf-8"><base href="{DEMO.as_uri()}/">
<style>{style}
html, body {{ margin: 0; overflow: hidden; }}
*, *::before, *::after {{ transition: none !important; animation: none !important; }}
/* The demo scales with its width: twice the points is a 2× screen. */
.display {{ width: {WIDTH * 2}px; border-radius: 0; }}
.apill, .ruler, .outline, .marker, .pointer {{ display: none !important; }}
</style></head><body><div class="display">{markup}</div></body></html>"""


# Windows: Edge (vertical tabs: one row) and Windows Terminal, with Windows' caption buttons, its
# corners and fonts. Where the page's content is doesn't change. No menu bar: the clips start below it.
CAPTION = '<span class="caption"><i>\ue921</i><i>\ue922</i><i>\ue8bb</i></span>'
WINDOWS_STYLE = """
.menubar { display: none !important; }
.win { border-radius: calc(8 * var(--u)) !important;
  box-shadow: 0 0 0 calc(1 * var(--u)) rgba(0, 0, 0, 0.3), 0 calc(8 * var(--u)) calc(32 * var(--u)) rgba(0, 0, 0, 0.35) !important; }
.bar .caption { display: flex; align-self: stretch; margin-left: auto; margin-right: calc(-12 * var(--u)); }
.bar .caption i { display: flex; align-items: center; justify-content: center; width: calc(46 * var(--u));
  font: normal calc(10 * var(--u)) "Segoe Fluent Icons", "Segoe MDL2 Assets"; }
.safari .bar { padding-left: calc(12 * var(--u)) !important; }
.safari .bar .url { width: calc(600 * var(--u)); margin: 0 0 0 calc(10 * var(--u)); justify-content: flex-start;
  padding-left: calc(14 * var(--u)); border-radius: calc(8 * var(--u)); }
.term { background: #0c0c0c !important; }
.term .bar { height: calc(34 * var(--u)) !important; padding: 0 calc(12 * var(--u)) 0 calc(8 * var(--u)) !important;
  background: #202020 !important; border: 0 !important; align-items: flex-end; }
.term .bar .caption { align-self: flex-start; height: 100%; color: #ccc; }
.term .tab { display: flex; align-items: center; gap: calc(8 * var(--u)); width: calc(190 * var(--u));
  height: calc(28 * var(--u)); padding: 0 calc(10 * var(--u)); color: #fff; background: #0c0c0c;
  border-radius: calc(6 * var(--u)) calc(6 * var(--u)) 0 0; }
.term .tab svg { width: calc(14 * var(--u)) !important; height: calc(14 * var(--u)) !important; padding: 0 !important; }
.term .tab i { margin-left: auto; font: normal calc(8 * var(--u)) "Segoe Fluent Icons", "Segoe MDL2 Assets"; color: #aaa; }
.term .newtab { align-self: center; margin-left: calc(8 * var(--u)); color: #ccc;
  font: normal calc(10 * var(--u)) "Segoe Fluent Icons", "Segoe MDL2 Assets"; }
.term .screen { height: calc(526 * var(--u)) !important; background: #0c0c0c;
  font-family: "Cascadia Mono", Consolas, monospace !important; }
"""


def windows_desk(markup):
    markup = re.sub(r'(<div class="win safari">\s*<div class="bar">)\s*<span class="lights">.*?</svg>',
                    r"\1", markup, count=1, flags=re.S)
    refresh = ('<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">'
               '<path d="M19 12a7 7 0 1 1-2.05-4.95M19 4.5v4h-4"/></svg>')
    markup = replace(markup, '<span class="url">', refresh + '<span class="url">')
    markup = re.sub(r'(<span class="url">[^<]*</span>)\s*<svg.*?</svg>\s*<svg.*?</svg>', r"\1" + CAPTION,
                    markup, count=1, flags=re.S)
    terminal = ('<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" '
                'stroke-linejoin="round"><path d="m5 8 4 4-4 4M11 16h7"/></svg>')
    markup = re.sub(r'<span class="lights"><i></i><i></i><i></i></span>\s*<span class="term-title">[^<]*</span>',
                    f'<span class="tab">{terminal}Claude Code<i>\ue8bb</i></span><span class="newtab">\ue710</span>'
                    + CAPTION, markup, count=1)
    markup = replace(markup, "**In:** Safari", "**In:** Microsoft Edge")
    return replace(markup, "Built-in Retina Display, 1512 × 945 pt, @2x", "Generic Monitor, 1512 × 945 pt")


if WINDOWS:
    style += WINDOWS_STYLE
    desk = windows_desk(desk)

# The demo's last frame: the aside in Claude Code's chat, in Terminal beside Safari, still in front.
sent = desk
# Before it: nothing sent yet. The real overlay draws on it.
safari = re.sub(r'<p class="you aside">.*?</p>\s*', "", desk, flags=re.S)
safari = re.sub(r'<p class="reply">.*?</p>\s*', "", safari, flags=re.S)

# Where the demo's styles put things, in points: after changing the demo, look at the frames
# (--keep).
APP = "Microsoft Edge" if WINDOWS else "Safari"
page_of = {"app": APP, "window": "Checkout · meadow", "url": "localhost:5173/checkout"}
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
# How long a replay waits for the overlay's windows first (a debug build on Windows takes longer).
START = 1500 if WINDOWS else 600
# Between the steps of a drag: measuring works out the edges as it goes, slower in Windows' debug build.
DRAG_WAIT = 500 if WINDOWS else 120


def drag(path, element=None, held=None):
    """Drags along `path`; `held`: a capture before letting go (measuring, the size drawn shows then)."""
    steps = [{"move": path[0]}, {"down": path[0]}, *({"drag": p, "wait": DRAG_WAIT} for p in path[1:])]
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
        {"wait": START}, {"recording": 2}, {"move": REST, "wait": 200}, {"capture": "basics-0.png"},
        *click(CARD, card), {"capture": "basics-1.png"},
        *drag(AREA, pay), {"capture": "basics-2.png"},
        {"recording": 9}, {"level": 0.55}, {"move": REST, "wait": 250}, {"capture": "basics-3.png"},
        *CLOSE,
    ]),
    "target": ("safari", [
        {"wait": START}, {"recording": 3}, {"move": REST, "wait": 200}, {"capture": "target-0.png"},
        {"key": "tab", "wait": 500}, {"capture": "target-1.png"},
        {"move": NAME, "toolbar": True, "wait": 500}, {"capture": "target-2.png"},
        *CLOSE,
    ]),
    "card": ("safari", [
        {"wait": START}, {"recording": 4}, *click(CARD, card), {"move": REST, "wait": 200}, {"capture": "card-0.png"},
        *click(CARD), {"capture": "card-1.png"},
        {"type": "Give it room under the header", "wait": 300}, {"capture": "card-2.png"},
        {"key": "cmd+2", "wait": 300}, {"capture": "card-3.png"},
        *CLOSE,
    ]),
    "tools": ("safari", [
        # Listening, as asides opens by default: muted, the circle drawn would open its card.
        {"wait": START}, {"recording": 5}, {"move": REST}, {"key": "m", "wait": 200}, {"move": FIELD, "wait": 300},
        {"capture": "tools-0.png"},
        *drag(FIELD_DRAG, held="tools-1.png"),
        {"key": "d", "wait": 200}, *drag(CIRCLE), {"move": REST, "wait": 300}, {"capture": "tools-2.png"},
        *CLOSE,
    ]),
    "keys": ("safari", [
        {"wait": START}, {"move": REST, "wait": 200}, {"key": "?", "wait": 800 if WINDOWS else 400}, {"capture": "keys-0.png"},
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
        {"image": "card-3", "key": "Ctrl+2" if WINDOWS else "⌘2", "hold": 2600},
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
    source.write_text(html(markup), encoding="utf-8")
    subprocess.run(
        [FIREFOX, "--headless", "--no-remote", "--profile", str(profile),
         f"--window-size={WIDTH * 2},{HEIGHT * 2}", "--screenshot", str(out), source.as_uri()],
        check=True, capture_output=True,
    )


def run(name, screen, replay, tmp):
    run_dir = tmp / name
    run_dir.mkdir()
    (run_dir / "screen-0.rgba").write_bytes(Image.open(tmp / f"{screen}.png").convert("RGBA").tobytes())
    home = r"C:\Users\me\code" if WINDOWS else "/Users/me/Developer"
    apps = ["Windows Terminal", "VS Code", "Windows Terminal"] if WINDOWS else ["Terminal", "Ghostty", "Diri"]
    spec = {
        "screens": [{
            "display": 1, "screen": {"name": "Generic Monitor" if WINDOWS else "Built-in Retina Display",
                                     "width": WIDTH, "height": HEIGHT, "scale": 2},
            "origin": [0, 0], "frame": {"x": 0, "y": 0, "width": WIDTH, "height": HEIGHT},
            "pixels": "screen-0.rgba", "width": WIDTH * 2, "height": HEIGHT * 2,
            "place": f"{APP} · \"Checkout · meadow\"", "menu_bar": MENU_BAR,
        }],
        "color": "#FF2D55", "tolerance": 2, "identify": True,
        "record_shortcut": "Ctrl+Alt+Space" if WINDOWS else "⌃⌥Space",
        "freeze_shortcut": "Ctrl+Alt+P" if WINDOWS else "⌃⌥P", "listen": True,
        "model": {"ready": True, "label": "English, 190 MB"},
        "targets": {
            "sessions": [
                {"id": "200", "label": "Claude Code (shop)", "app": apps[0]},
                {"id": "201", "label": "Codex (api)", "app": apps[1]},
                {"id": "202", "label": "OpenCode (blog)", "app": apps[2]},
            ],
            "folders": [{"path": os.path.join(home, name) if WINDOWS else f"{home}/{name}", "name": name}
                        for name in ("shop", "api", "blog")],
            "agents": ["claude", "codex", "opencode"], "chosen": {"kind": "session", "id": "200"},
        },
        # The window shows the part of the screen it covers: all of it, at the screen's origin.
        "windowed": {"x": 0, "y": 0, "width": WIDTH, "height": HEIGHT},
        "replay": replay,
    }
    (run_dir / "session.json").write_text(json.dumps(spec, ensure_ascii=False), encoding="utf-8")
    # The overlay dark, whatever the Mac's appearance, as the clips always were.
    subprocess.run([OVERLAY, run_dir], check=True, timeout=60,
                   stdout=subprocess.DEVNULL, env={**os.environ, "ASIDES_WINDOW_APPEARANCE": "dark"})
    for png in run_dir.glob("*.png"):
        shutil.copy(png, tmp / "frames" / png.name)


def pill_box(frame, desk):
    """The pill on `frame` (pixels): where it differs from the desktop, near the top, as tall as it is
    from its bottom (over a dark sky, its top edge hardly differs)."""
    scale = frame.width / WIDTH
    top = (0, 0, frame.width, round(200 * scale))
    frame, threshold = frame.crop(top), 24
    if desk.size != (frame.width, round(HEIGHT * scale)):
        # Scaled down by the overlay and by Pillow, the desktop differs a little everywhere: blurred,
        # only the pill stands out.
        desk = desk.resize((frame.width, round(HEIGHT * scale)), Image.LANCZOS)
        frame, desk, threshold = frame.filter(ImageFilter.GaussianBlur(1)), desk.filter(ImageFilter.GaussianBlur(1)), 40
    desk = desk.crop(top)
    changed = ImageChops.difference(frame, desk).convert("L").point(lambda v: 255 if v > threshold else 0)
    x0, _, x1, y1 = changed.getbbox()
    return x0 / scale, y1 / scale - PILL_HEIGHT, (x1 - x0) / scale, PILL_HEIGHT


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
            images = [Image.open(tmp / "frames" / f"{f['image']}.png").convert("RGB") for f in frames]
            # The overlay's captures are at the display's scale, Firefox's at 2×: all at the least.
            scale = min(image.width / WIDTH for image in images)
            for i, (frame, image) in enumerate(zip(frames, images)):
                out = f"{name}-{i}.webp" if len(frames) > 1 else f"{name}.webp"
                at = image.width / WIDTH
                image = image.crop((round(x * at), round(y * at), round((x + w) * at), round((y + h) * at)))
                image = image.resize((round(w * scale), round(h * scale)), Image.LANCZOS) if at != scale else image
                if options.get("bare"):
                    image = cut_corners(image, options["radius"] * scale)
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
        (OUT / "clips.json").write_text("{\n" + clips + "\n}\n", encoding="utf-8")
    total = sum(f.stat().st_size for f in OUT.iterdir())
    print(f"Wrote {OUT} ({total // 1024} KB)")


if __name__ == "__main__":
    main()
