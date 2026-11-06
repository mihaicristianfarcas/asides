"""Renders docs/og.png, the image shown when the website's link is shared, and the README's picture:
the page's hero as its opening ends, asides open over the heading. It's docs/index.html itself, with
JavaScript off (which shows that frame), the rest of the page left out and the hero fitted to the
image (STYLE), zoomed so that what asides shows stays legible when the image is shown small. It's
1200 × 630 at 2×, for the README on a Retina screen. It needs Firefox (its headless screenshots
ignore a scale set in its preferences).

Usage: python3 scripts/make-og.py
"""

import os
import pathlib
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ROOT / "docs"
FIREFOX = os.environ.get("FIREFOX", "/Applications/Firefox.app/Contents/MacOS/firefox")
WIDTH, HEIGHT = 2400, 1260

# In CSS pixels of the page zoomed 1.25×, 960 × 504; the scale then doubles it without touching the
# layout (a zoom of 2.5× would: the system font's letters and tracking change with their size, and
# small text came out tighter). The mark goes beside the pill, where the header's links were; the
# heading, the text under it and the button are one block, in the middle.
STYLE = """
<style>
html { zoom: 1.25; width: 960px; transform: scale(2); transform-origin: 0 0; }
html, body { height: 504px; overflow: hidden; }
.top-bar { position: absolute; inset: 0 0 auto; background: none; -webkit-backdrop-filter: none; backdrop-filter: none; }
.top { max-width: none; height: 84px; padding: 0 40px; font-size: 16px; }
.top nav, .also, .flow, .section, .last, .foot { display: none; }
.page { max-width: none; padding: 0 40px; }
.hero { height: 504px; min-height: 0; padding: 20px 0 24px; }
.tool { margin-bottom: 0; }
:root { --text-display: 72px; }
.hero-main { width: 700px; margin-inline: auto; }
.lede { max-width: none; }
.cta { margin-top: 22px; }
.say { margin-top: 0; font-size: 15px; white-space: nowrap; }
</style>
"""

page = (DOCS / "index.html").read_text()
# For every system: the download as it's shown where the page can't tell which.
mac = '<use href="#for-mac"></use></svg><span data-download-label>Download for Mac</span>'
assert mac in page
page = page.replace(mac, '<use href="#for-any"></use></svg><span data-download-label>Download</span>')
page = page.replace("<head>", f'<head>\n<base href="{DOCS.as_uri()}/">', 1).replace("</head>", f"{STYLE}</head>", 1)

with tempfile.TemporaryDirectory() as tmp:
    tmp = pathlib.Path(tmp)
    (tmp / "og.html").write_text(page)
    (tmp / "profile").mkdir()
    (tmp / "profile" / "user.js").write_text('user_pref("javascript.enabled", false);\n')
    shot = tmp / "og.png"
    subprocess.run(
        [FIREFOX, "--headless", "--no-remote", "--profile", str(tmp / "profile"),
         f"--window-size={WIDTH},{HEIGHT}", "--screenshot", str(shot),
         (tmp / "og.html").as_uri()],
        check=True, capture_output=True,
    )
    shutil.copy(shot, DOCS / "og.png")
print(f"Wrote {DOCS / 'og.png'}")
