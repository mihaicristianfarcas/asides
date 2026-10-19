"""Renders docs/og.png, the 1200 × 630 image shown when the website's link is shared.

The picture is the website's own demo, frozen mid-recording: this script takes the demo's markup
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


# The frame: testing the checkout page in Safari, the screen frozen under Asides' toolbar, seven
# seconds into talking.
words = "The summary card is jammed under the header."
device = replace(device, '<div class="desk" data-app="textedit"', '<div class="desk frozen" data-app="safari"')
device = replace(device, '<div class="win safari inactive">', '<div class="win safari">')
device = replace(device, '<div class="win editor open">', '<div class="win editor">')
device = replace(device, '<span class="tb tb-mic">', '<span class="tb tb-mic rec">')
device = replace(device, '<span class="tb-time"></span>', '<span class="tb-time">0:07</span>')
device = replace(device, '<div class="pointer">', '<div class="pointer on crosshair" style="--x: 1000; --y: 250">')
device = replace(device, '<p class="say" aria-live="polite"></p>', f'<p class="say">{words}</p>')

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
  <p>Transcribed on your Mac, into a Markdown note in your repo, and straight to your coding agent.</p>
</div>
<div class="og-device">{device}</div>
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
