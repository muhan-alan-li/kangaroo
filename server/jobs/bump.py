"""Bump the cache-busting version on the dashboard's client assets.

`tailscale serve` sends only `Last-Modified`, so a browser can reuse `client/dist/postings.js`
or `health.js` for a while after a rebuild. The bundles have fixed names, so a changed entry
file needs a changed URL: this rewrites the `?v=` on both entry scripts and on `styles.css`.
The Preact runtime chunk carries a content hash in its own filename, which Vite regenerates on
every build, so it cannot be pinned to an old copy.

    python3 -m server bump            # print the current version
    python3 -m server bump --bump     # today's date plus a letter
    python3 -m server bump --set 20261006a

Run `npm run build` first, then `bump --bump` after any change under `client/src/` or to
`client/styles.css` / the two HTML files.
"""
from __future__ import annotations

import argparse
import os
import re
from datetime import date

from ..config import ROOT
CLIENT = os.path.join(ROOT, "client")
ENTRY_RE = re.compile(r'(src="/dist/[\w.-]+\.js)(\?v=[^"]*)?(")')
CSS_RE = re.compile(r'(href="/styles\.css)(\?v=[^"]*)?(")')


def current():
    html = open(os.path.join(CLIENT, "index.html"), encoding="utf-8").read()
    match = re.search(r"/styles\.css\?v=([\w.-]+)", html)
    return match.group(1) if match else None


def set_version(version):
    changed = []
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
