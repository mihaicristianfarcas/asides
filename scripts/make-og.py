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


# The frame: testing the checkout page in Safari, frozen under asides' pill, the gap under the sticky
# header measured, seven seconds into talking.
words = "The summary card is jammed under the header."
# Nothing in the agent's chat yet.
device = replace(device, '<p class="you aside">', '<p class="you aside" hidden>')
device = replace(device, '<p class="reply">', '<p class="reply" hidden>')
device = replace(device, '<div class="apill">', '<div class="apill on measuring">')
device = replace(device, '<span class="ap-mic">', '<span class="ap-mic listening">')
device = replace(device, '<i class="level"></i>', '<i class="level" style="height: 60%"></i>')
device = replace(device, '<div class="ruler"><i></i></div>', '<div class="ruler on"><i>6</i></div>')
device = replace(device, '<div class="pointer">', '<div class="pointer on crosshair">')
device = replace(device, '<p class="say" aria-live="polite"></p>', f'<p class="say">{words}</p>')

# The ruler and the pointer go in the gap, wherever the page's styles put it (on the desktop's
# 1512-point grid, as the page's demo places them).
place = """<script>
const desk = document.querySelector('.desk');
const d = desk.getBoundingClientRect();
const k = 1512 / d.width;
const edge = (sel, side) => (desk.querySelector(sel).getBoundingClientRect()[side] - d[side === 'bottom' || side === 'top' ? 'top' : 'left']) * k;
// (Not `top`: a page's script can't declare the window's own.)
const [header, heading, x] = [edge('.site-head', 'bottom'), edge('.card h5', 'top'), edge('.card', 'left') + 14];
const set = (sel, props) => Object.entries(props).forEach(([p, v]) => desk.querySelector(sel).style.setProperty(p, v));
set('.ruler', { '--bx': x, '--by': header, '--bh': heading - header });
set('.pointer', { '--x': x, '--y': (header + heading) / 2 });
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
  <h1>Report bugs out loud while you test.</h1>
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
    # The page in dark mode, whatever this Mac's appearance.
    (tmp / "profile" / "user.js").write_text(f'user_pref("layout.css.devPixelsPerPx", "{SCALE}");\n'
                                             'user_pref("ui.systemUsesDarkTheme", 1);\n')
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
