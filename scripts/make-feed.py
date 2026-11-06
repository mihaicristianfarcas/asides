#!/usr/bin/env python3
"""The update feed (appcast.json, an asset of every release; asides_core::update reads it): this
release's files for each system, then the newest few releases before it, as published.

    scripts/make-feed.py VERSION NOTES DIST [PREVIOUS] > DIST/appcast.json

NOTES is the release's notes (Markdown, What's New shows them), DIST the folder with the release's
files, PREVIOUS the feed published so far (a file; by default the latest release's, from GitHub).
The Mac's zip stays at the top level of each release, where copies from before Windows and Linux
read it; Windows' installers and Linux's archives are under `windows` and `linux`, by architecture.
"""

import datetime
import hashlib
import json
import os
import plistlib
import sys
import urllib.error
import urllib.request

REPO = "mihaicristianfarcas/asides"
# Windows 10 2004 (the installer's MinVersion): the pill left out of captures.
OLDEST_WINDOWS = "10.0.19041"
# Releases kept in the feed besides the new one: What's New shows their notes.
KEEP = 4


def file(dist, name, oldest=None):
    path = os.path.join(dist, name)
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    entry = {
        "url": f"https://github.com/{REPO}/releases/download/v{VERSION}/{name}",
        "size": os.path.getsize(path),
        "sha256": digest.hexdigest(),
    }
    if oldest:
        entry["minimum_system_version"] = oldest
    return entry


def previous(source):
    try:
        if source:
            with open(source, encoding="utf-8") as f:
                return json.load(f).get("releases", [])
        url = f"https://github.com/{REPO}/releases/latest/download/appcast.json"
        with urllib.request.urlopen(url, timeout=30) as response:
            return json.load(response).get("releases", [])
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return []
        raise


if __name__ == "__main__":
    if len(sys.argv) not in (4, 5):
        sys.exit(__doc__.strip())
    VERSION, notes, dist = sys.argv[1:4]
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    with open(os.path.join(root, "packaging", "Info.plist"), "rb") as f:
        oldest_macos = plistlib.load(f)["LSMinimumSystemVersion"]
    mac = file(dist, f"asides-{VERSION}.zip")
    release = {
        "version": VERSION,
        "url": mac["url"],
        "size": mac["size"],
        "sha256": mac["sha256"],
        "minimum_system_version": oldest_macos,
        "published": datetime.datetime.now(datetime.timezone.utc).date().isoformat(),
        "notes": open(notes, encoding="utf-8").read().strip(),
        "windows": {
            arch: file(dist, f"asides-{VERSION}-windows-{arch}-setup.exe", OLDEST_WINDOWS) for arch in ("arm64", "x64")
        },
        "linux": {arch: file(dist, f"asides-{VERSION}-linux-{arch}.tar.gz") for arch in ("aarch64", "x86_64")},
    }
    before = [r for r in previous(sys.argv[4] if len(sys.argv) == 5 else None) if r.get("version") != VERSION]
    print(json.dumps({"releases": [release] + before[:KEEP]}, indent=2, ensure_ascii=False))
