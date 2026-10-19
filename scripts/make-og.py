"""Renders docs/og.png, the 1200 × 630 image shown when the website's link is shared.

The picture is the website's own demo, frozen with a pin on the page: this script takes the demo's markup
and styles from docs/index.html, so the image keeps up with the page. It needs Firefox.

Usage: python3 scripts/make-og.py
"""

import os
import pathlib
import re
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
FIREFOX = os.environ.get("FIREFOX", "/Applications/Firefox.app/Contents/MacOS/firefox")
WIDTH, HEIGHT, SCALE = 1200, 630, 2

page = (DOCS / "index.html").read_text()
style = re.search(r"<style>(.*?)</style>", page, re.S).group(1)
device = re.search(r'<figure class="device" id="demo">.*?</figure>', page, re.S).group(0)
mark = re.search(r'<a class="mark" href="./">\s*(<svg.*?</svg>)', page, re.S).group(1)


def replace(text, old, new):
    assert old in text, f"not in the demo markup any more: {old}"
    return text.replace(old, new, 1)


# The frame: testing the checkout page in Safari, frozen seven seconds ago, the summary card pinned.
words = "The summary card needs more padding under the sticky header."
device = replace(device, '<div class="desk" data-app="terminal"', '<div class="desk" data-app="safari"')
device = replace(device, '<div class="win safari inactive">', '<div class="win safari">')
device = replace(device, '<div class="win term front">', '<div class="win term inactive">')
# The aside isn't in the agent's chat yet.
device = replace(device, '<p class="you aside">', '<p class="you aside" hidden>')
device = replace(device, '<p class="reply">', '<p class="reply" hidden>')
device = replace(device, '<div class="pill">', '<div class="pill on">')
device = replace(device, '<span class="time">0:00</span>', '<span class="time">0:07</span>')
device = replace(device, '<div class="pin p1">', '<div class="pin p1 on">')
device = replace(device, '<div class="pointer">', '<div class="pointer on">')
device = replace(device, '<p class="say" aria-live="polite"></p>', f'<p class="say">{words}</p>')

# The pin and the pointer go on the card, wherever the page's styles put it (on the
# desktop's 1024-unit grid, as the page's demo places them).
place = """<script>
const desk = document.querySelector('.desk');
const d = desk.getBoundingClientRect();
const r = desk.querySelector('.card').getBoundingClientRect();
const k = 1024 / d.width;
const [x, y, w, h] = [(r.left - d.left) * k, (r.top - d.top) * k, r.width * k, r.height * k];
const set = (sel, props) => Object.entries(props).forEach(([p, v]) => desk.querySelector(sel).style.setProperty(p, v));
set('.p1', { '--x': x + 0.62 * w, '--y': y + 0.06 * h });
set('.pointer', { '--x': x + 0.8 * w, '--y': y + 0.3 * h });
</script>"""

html = f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<base href="{DOCS.as_uri()}/">
<style>
{style}
html, body {{ width: {WIDTH}px; height: {HEIGHT}px; overflow: hidden; }}
*, *::before, *::after {{ transition: none !important; }}
.og-copy {{ position: absolute; top: 64px; left: 72px; width: 380px; height: {HEIGHT - 128}px;
  display: flex; flex-direction: column; }}
.og-copy .mark {{ font-size: 26px; gap: 12px; }}
.og-copy .mark svg {{ width: 28px; height: 28px; }}
.og-copy h1 {{ margin-top: auto; font-size: 46px; line-height: 1.06; font-weight: 700; letter-spacing: -0.035em; }}
.og-copy p {{ margin-top: 24px; font-size: 21px; line-height: 1.4; color: var(--muted); }}
.og-device {{ position: absolute; top: 64px; left: 492px; width: 800px; }}
/* Shared links show the image small: larger subtitles than on the page. */
.og-device .say {{ font-size: 19px; }}
</style>
</head>
<body>
<div class="og-copy">
  <span class="mark">{mark} asides</span>
  <h1>Talk through bugs while you test.</h1>
  <p>Click what's wrong, say why, and it's in your coding agent's chat. Transcribed on your Mac.</p>
</div>
<div class="og-device">{device}</div>
{place}
</body>
</html>
"""

with tempfile.TemporaryDirectory() as tmp:
    tmp = pathlib.Path(tmp)
    (tmp / "profile").mkdir()
    (tmp / "profile" / "user.js").write_text(f'user_pref("layout.css.devPixelsPerPx", "{SCALE}");\n')
    source = tmp / "og.html"
    source.write_text(html)
    shot = tmp / "og.png"
    subprocess.run(
        [FIREFOX, "--headless", "--no-remote", "--profile", str(tmp / "profile"),
         f"--window-size={WIDTH},{HEIGHT}", "--screenshot", str(shot), source.as_uri()],
        check=True, capture_output=True,
    )
    # Rendered at 2× and scaled down, for crisp small text.
    subprocess.run(["sips", "-z", str(HEIGHT), str(WIDTH), str(shot)], check=True, capture_output=True)
    shutil.copy(shot, DOCS / "og.png")
print(f"Wrote {DOCS / 'og.png'}")
