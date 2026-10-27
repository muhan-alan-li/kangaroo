"""I/O and workspace paths: the HTTP getter, the YAML reader, and the small
HTML helpers every source uses."""

from __future__ import annotations

import html
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

try:
    from ..fsutil import atomic_write  # re-exported: every state writer imports it from here
except ImportError:  # the smoke tests import `scan` as a top-level package with server/ on sys.path
    from fsutil import atomic_write
from . import net
from .net import is_dns_error

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.stderr.write("the scanner needs PyYAML (pip install pyyaml)\n")
    raise SystemExit(2)

# This file lives at server/scan/core.py, so it walks up three levels to the workspace root:
# core.py -> scan/ -> server/ -> root.
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/120.0 Safari/537.36")

# Every source and every scheduled job reaches the network through this module, so this is
# the one place the DNS cache/socket governor has to be switched on. It patches
# `socket.getaddrinfo` and `urllib.request.urlopen` for the process; see net.py for why.
net.install()


# --------------------------------------------------------------------------- io

def load_yaml(*parts):
    path = os.path.join(ROOT, *parts)
    with open(path, encoding="utf-8") as fh:
        return yaml.safe_load(fh) or {}


def role_regex(opts):
    """The engineering-title matcher from a source's `title_words`, or the default."""
    words = tuple(opts.get("title_words") or WORKDAY_ROLE_WORDS)
    return re.compile(r"\b(" + "|".join(re.escape(w) for w in words) + r")\b", re.I)


def http_get(url, timeout=30, retries=2):
    """GET a URL, return text or None. Never raises -- one dead source must not
    kill the whole scan.

    A name-resolution failure is not retried: the resolver already refused, and asking again
    is what makes the resolver extend the block. Fail fast and let the next host try.
    """
    last = None
    for attempt in range(retries + 1):
        try:
            req = urllib.request.Request(url, headers={
                "User-Agent": UA,
                "Accept-Language": "en-CA,en;q=0.9",
                "Accept": "*/*",
            })
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.read().decode("utf-8", "replace")
        except Exception as exc:  # noqa: BLE001
            last = exc
            if is_dns_error(exc) or attempt >= retries:
                break
            time.sleep(1.5 * (attempt + 1))
    sys.stderr.write(f"[scan] fetch failed: {url} ({last})\n")
    return None


def strip_tags(fragment):
    return html.unescape(re.sub(r"\s+", " ", re.sub(r"<[^>]+>", " ", fragment))).strip()


def first_href(fragment, skip=("imgur.com", "simplify.jobs/c/", "github.com")):
    for href in re.findall(r'href=["\']([^"\']+)["\']', fragment):
        if not any(s in href for s in skip):
            return html.unescape(href)
    return None


# The user's ask for this source: engineering roles only, Canada or the United States, and
# posted within the window. "engineering roles" here means the title names an engineering
# job -- it is deliberately loose, because the title filters downstream are the strict
# pass and this one only exists to keep the volume sane.
WORKDAY_ROLE_WORDS = ("engineer", "engineering", "developer", "programmer", "swe", "sde")
