"""iCIMS hosts, read through a browser-impersonating subprocess because every
customer host sits behind an AWS WAF."""

from __future__ import annotations

from datetime import datetime
import json
import os
import subprocess
import sys

from ..core import ROOT, WORKDAY_ROLE_WORDS
from ..parsing import parse_age_days
from ..posting import apply_signature, make_posting


def _icims_company(host):
    """A readable employer name from a customer host: careers-ice -> ice."""
    label = (host or "").split(".")[0]
    for prefix in ("careers-", "career-", "jobs-", "apply-", "external-"):
        if label.startswith(prefix):
            return label[len(prefix):]
    return label


def fetch_icims_boards(boards, cfg):
    """Read every iCIMS board through the impersonating fetcher in its own venv.

    iCIMS is the one source the scan cannot read directly. Every customer host sits behind
    an AWS WAF that answers a plain HTTP client with a 405 "Human Verification" page --
    urllib and curl alike, on every path including /sitemap.xml and /api/jobs -- while a real
    browser is served the page normally. The browser holds no aws-waf-token cookie, so the
    WAF is judging the TLS fingerprint and there is no token to borrow. Hence curl_cffi with
    browser impersonation, which does not belong in the scan's interpreter: it lives in
    .venv-icims and server/scan/icims_fetch.py, and this drives it as a subprocess.

    The fetcher hands back rows already carrying `posted` (read from each candidate's detail
    page, since the search view has no date at all). A row without one is kept and flagged
    rather than presented as fresh.
    """
    opts = cfg.get("icims") or {}
    # JOB_SCAN_ICIMS_PYTHON points at an interpreter that already has curl_cffi.
    # It exists for the containerized scan: the image carries curl_cffi in its
    # system python (see docker/compose.yaml) and does not mount .venv-icims.
    # On the host it is unset and the venv below is used, as before.
    python = (os.environ.get("JOB_SCAN_ICIMS_PYTHON")
              or opts.get("python")
              or os.path.join(ROOT, ".venv-icims", "bin", "python"))
    script = os.path.join(ROOT, "server", "scan", "icims_fetch.py")
    if not os.path.exists(python):
        sys.stderr.write(
            f"[scan] icims: skipped, no fetcher venv at {python}\n"
            "        rebuild it with: uv venv .venv-icims --python 3.13 && "
            "uv pip install --python .venv-icims/bin/python curl_cffi\n")
        return []
    if not os.path.exists(script):
        sys.stderr.write(f"[scan] icims: skipped, {script} is missing\n")
        return []

    hosts = [b.get("slug") for b in boards if b.get("slug")]
    spec = {
        "hosts": hosts,
        "max_pages": int(opts.get("max_pages") or 4),
        "workers": int(opts.get("workers") or 8),
        "timeout": int(opts.get("timeout") or 25),
        "window_days": int(opts.get("window_days") or 7),
        "title_words": list(opts.get("title_words") or WORKDAY_ROLE_WORDS),
        "keywords": list(opts.get("keywords") or ("engineer", "developer")),
        "details": bool(opts.get("details", True)),
    }
    try:
        proc = subprocess.run([python, script], input=json.dumps(spec),
                              capture_output=True, text=True,
                              timeout=int(opts.get("total_timeout") or 900))
    except subprocess.TimeoutExpired:
        sys.stderr.write("[scan] icims: the fetcher timed out; skipping this source\n")
        return []
    except OSError as exc:
        sys.stderr.write(f"[scan] icims: could not run the fetcher: {exc}\n")
        return []

    for line in (proc.stderr or "").strip().splitlines():
        sys.stderr.write(line + "\n")
    if proc.returncode != 0 or not (proc.stdout or "").strip():
        sys.stderr.write(f"[scan] icims: the fetcher failed (exit {proc.returncode})\n")
        return []
    try:
        payload = json.loads(proc.stdout)
    except ValueError as exc:
        sys.stderr.write(f"[scan] icims: unreadable fetcher output: {exc}\n")
        return []

    out, undated, truncated = [], 0, 0
    for host, got in (payload or {}).items():
        if got.get("truncated"):
            truncated += 1
        for row in (got.get("rows") or []):
            url = row.get("url")
            _v, _s, _job_id, dedupe_key = apply_signature(url or "")
            posted = None
            if row.get("posted"):
                try:
                    posted = datetime.strptime(row["posted"], "%Y-%m-%d").date()
                except ValueError:
                    posted = None
            extra = {"board": f"icims|{host}"}
            if posted is None:
                undated += 1
                extra["date_missing"] = True
            out.append(make_posting(
                source="ats", company=_icims_company(host), title=row.get("title", ""),
                location=row.get("location") or "", url=url, posted=posted,
                age_days=parse_age_days(None, posted), ats="icims",
                raw_id=row.get("id"), dedupe_key=dedupe_key, extra=extra,
            ))
    if truncated:
        sys.stderr.write(
            f"[scan] icims: {truncated} boards filled every page requested, so a recent "
            "posting could sit past the cap\n")
    return out
