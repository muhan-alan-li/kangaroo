"""Bump the cache-busting version on the dashboard's client assets.

`tailscale serve` sends only `Last-Modified`, so a browser can reuse an old ES module for a few
minutes. If a changed module imports a name the cached one does not export, the whole module
graph fails and the page renders empty -- the table with no rows. Versioning every relative
import and both entry points (and the stylesheet) forces a fresh fetch.

    python3 -m server bump            # print the current version
    python3 -m server bump --bump     # today's date plus a letter
    python3 -m server bump --set 20261006a

Run `bump --bump` after any change to `client/js/*.js`, `client/styles.css`, or the two HTML files.
"""
from __future__ import annotations

import argparse
import glob
import os
import re
from datetime import date

# This file lives at server/jobs/bump.py, so it walks up three levels to the workspace root:
# bump.py -> jobs/ -> server/ -> root.
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CLIENT = os.path.join(ROOT, "client")
IMPORT_RE = re.compile(r"from '\./([\w-]+)\.js(\?v=[^']*)?'")
ENTRY_RE = re.compile(r'(src="/js/[\w-]+\.js)(\?v=[^"]*)?(")')
CSS_RE = re.compile(r'(href="/styles\.css)(\?v=[^"]*)?(")')


def current():
    html = open(os.path.join(CLIENT, "index.html"), encoding="utf-8").read()
    match = re.search(r"/styles\.css\?v=([\w.-]+)", html)
    return match.group(1) if match else None


def set_version(version):
    changed = []
    for path in glob.glob(os.path.join(CLIENT, "js", "*.js")):
        text = open(path, encoding="utf-8").read()
        new = IMPORT_RE.sub(r"from './\1.js?v=" + version + "'", text)
        if new != text:
            open(path, "w", encoding="utf-8").write(new)
            changed.append(path)
    for name in ("index.html", "health.html"):
        path = os.path.join(CLIENT, name)
        text = open(path, encoding="utf-8").read()
        new = ENTRY_RE.sub(r"\1?v=" + version + r"\3", text)
        new = CSS_RE.sub(r"\1?v=" + version + r"\3", new)
        if new != text:
            open(path, "w", encoding="utf-8").write(new)
            changed.append(path)
    return changed


def main(argv=None):
    ap = argparse.ArgumentParser(description="Show or bump the client cache-busting version.")
    ap.add_argument("--bump", action="store_true", help="today's date plus the next letter")
    ap.add_argument("--set", dest="value", help="use this version string verbatim")
    args = ap.parse_args(argv)

    if args.value:
        version = args.value
    elif args.bump:
        base = date.today().strftime("%Y%m%d")
        now = current() or ""
        if now.startswith(base) and len(now) > len(base) and now[-1].isalpha():
            version = base + chr(ord(now[-1]) + 1)
        else:
            version = base + "a"
    else:
        print(current() or "(unversioned)")
        return 0

    for path in set_version(version):
        print("updated", os.path.relpath(path, ROOT))
    print("version:", version)
    return 0
