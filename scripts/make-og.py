"""Renders docs/og.png, the 1200 × 630 image shown when the website's link is shared: the page's
opening as it ends, asides open over the heading. The page shows that frame without JavaScript, so
this is docs/index.html itself, in a window the image's size, with JavaScript off. It needs Firefox.

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
WIDTH, HEIGHT = 1200, 630

with tempfile.TemporaryDirectory() as tmp:
    tmp = pathlib.Path(tmp)
    (tmp / "profile").mkdir()
    (tmp / "profile" / "user.js").write_text('user_pref("javascript.enabled", false);\n')
    shot = tmp / "og.png"
    subprocess.run(
        [FIREFOX, "--headless", "--no-remote", "--profile", str(tmp / "profile"),
         f"--window-size={WIDTH},{HEIGHT}", "--screenshot", str(shot), (DOCS / "index.html").as_uri()],
        check=True, capture_output=True,
    )
    shutil.copy(shot, DOCS / "og.png")
print(f"Wrote {DOCS / 'og.png'}")
