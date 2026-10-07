"""The contact-research primitives the brief generator used to get from the Hermes agent.

The `job-application-run` skill (`references/contact-research.md`) says how to find a published
work address: the team page that owns the posting, the manager's public profile, the company's
own public code (commit metadata carries `author_email`), the posting or a hiring post, and the
recruiter last. Hermes executed that with its `web_search`/`web_extract` tools, which fall back to
a keyless ring of public MCP endpoints (Exa, Parallel, Firecrawl, Keenable) -- see
`hermes-agent/plugins/web/keyless_mcp.py`. This module copies the parts that need no model and no
API key, so the direct DeepSeek loop can call them as tools.

Four primitives:

- `search(query)` -- Exa's public MCP endpoint, with Parallel as a fallback. No key.
- `fetch(url)`    -- the raw page, its text, and every address published in it. Cloudflare hides
                     addresses in `data-cfemail`, so an address only visible to a text reader is
                     missed; the raw HTML is decoded here, matching the skill's rule.
- `browser_fetch(url)` -- the same page through a real local Chromium (`server/brief/browser_fetch.mjs`),
                     for the pages that render client-side and whose `mailto:` links only exist
                     after scripts run. Local Playwright, no vendor and no key.
- `github_commit_emails(repo)` -- commit authors for a public repo, from the GitHub API (the
                     `gh` token is used when present for rate limits, else anonymous).

Nothing here guesses an address: it returns only strings that appear verbatim in a source.
"""
from __future__ import annotations

import glob
import json
import os
import re
import shutil
import subprocess
import urllib.error
import urllib.request

EXA_MCP_URL = "https://mcp.exa.ai/mcp"
PARALLEL_MCP_URL = "https://search.parallel.ai/mcp"
# A browser-shaped UA. Some company pages and Cloudflare in front of them answer a bot UA with a
# challenge, which reads as "no address" for a page that publishes one.
UA = ("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
      "Chrome/124.0 Safari/537.36")
TIMEOUT = 30

# Addresses that are never an outreach target, however often they appear. Mirrors the scanner's
# junk list: assets, trackers, and placeholder domains.
_EMAIL_JUNK = re.compile(
    r"(?:no-?reply|donotreply|do-not-reply|example\.(?:com|org)|sentry|wixpress|"
    r"users\.noreply\.github\.com|\.png|\.jpe?g|\.gif|\.svg|\.webp|\.css|\.js$|@2x)", re.I)
_EMAIL = re.compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")


class ResearchError(RuntimeError):
    """A primitive failed; the tool result reports it rather than aborting the loop."""


# --- web search: the keyless MCP ring --------------------------------------
def _mcp_call(url, tool, arguments, timeout=TIMEOUT):
    """POST one JSON-RPC `tools/call` and return the text content.

    Copied from Hermes' `keyless_mcp.py`: Exa answers with SSE `data:` frames, Parallel with a
    plain JSON envelope, and both are handled here.
    """
    payload = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
               "params": {"name": tool, "arguments": arguments}}
    request = urllib.request.Request(
        url, data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json",
                 "Accept": "application/json, text/event-stream", "User-Agent": UA})
    try:
        body = urllib.request.urlopen(request, timeout=timeout).read().decode("utf-8", "replace")
    except urllib.error.HTTPError as exc:
        raise ResearchError(f"HTTP {exc.code}: {exc.read()[:200].decode('utf-8', 'replace')}") from exc
    except Exception as exc:  # noqa: BLE001
        raise ResearchError(f"request failed: {exc}") from exc

    candidates = [body.strip()] if body.strip().startswith("{") else []
    candidates += [line[len("data: "):] for line in body.splitlines()
                   if line.startswith("data: ")]
    for candidate in candidates:
        try:
            data = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        result = data.get("result") or {}
        texts = [c.get("text", "") for c in result.get("content") or [] if isinstance(c, dict)]
        if result.get("isError"):
            raise ResearchError(" ".join(t for t in texts if t) or "MCP tool call failed")
        found = next((t for t in texts if t), None)
        if found:
            return found
    raise ResearchError("unrecognized MCP response")


def _exa_search(query, limit):
    text = _mcp_call(EXA_MCP_URL, "web_search_exa", {"query": query, "numResults": limit})
    hits = []
    for block in text.split("\n---\n"):
        title = url = ""
        snippet = []
        in_highlights = False
        for line in (s.strip() for s in block.splitlines()):
            for label, key in (("Title:", "t"), ("URL:", "u")):
                if line.startswith(label):
                    if key == "t":
                        title = line[len(label):].strip()
                    else:
                        url = line[len(label):].strip()
            if line.startswith("Highlights:"):
                in_highlights = True
            elif in_highlights and line and not line.startswith(
                    ("Title:", "URL:", "Published:", "Author:")):
                snippet.append(line)
        if url:
            hits.append({"title": title, "url": url, "snippet": " ".join(snippet)[:600]})
        if len(hits) >= limit:
            break
    return hits


def _parallel_search(query, limit):
    text = _mcp_call(PARALLEL_MCP_URL, "web_search",
                     {"objective": query, "search_queries": [query]})
    results = (json.loads(text) or {}).get("results") or []
    return [{"title": r.get("title") or "", "url": r.get("url") or "",
             "snippet": " ".join(r.get("excerpts") or [])[:600]}
            for r in results[:limit]]


def search(query, limit=5):
    """Public web results as `[{title, url, snippet}]`, Exa first then Parallel."""
    errors = []
    for name, fn in (("exa", _exa_search), ("parallel", _parallel_search)):
        try:
            hits = fn(query, limit)
            if hits:
                return hits
            errors.append(f"{name}: no results")
        except Exception as exc:  # noqa: BLE001
            errors.append(f"{name}: {exc}")
    raise ResearchError("; ".join(errors) or "no search backend answered")


# --- page fetch, raw-HTML address mining -----------------------------------
def decode_cfemail(encoded):
    """Decode Cloudflare's `data-cfemail` obfuscation: first byte is the XOR key, hex-encoded."""
    try:
        key = int(encoded[:2], 16)
        return "".join(chr(int(encoded[i:i + 2], 16) ^ key)
                       for i in range(2, len(encoded), 2))
    except (ValueError, IndexError):
        return ""


def emails_in(text):
    """Addresses literally present in a block of text, junk removed, order preserved."""
    seen, out = set(), []
    for hit in _EMAIL.finditer(text or ""):
        addr = hit.group(0).strip().strip(".,;:()<>\"'")
        low = addr.lower()
        if low in seen or _EMAIL_JUNK.search(low):
            continue
        # A trailing path or template means an asset or placeholder, not a mailbox.
        if (text or "")[hit.end():hit.end() + 1] in ("/", "{", "}"):
            continue
        seen.add(low)
        out.append(addr)
    return out


def _html_to_text(html):
    text = re.sub(r"(?is)<(script|style|noscript|svg)[^>]*>.*?</\1>", " ", html)
    text = re.sub(r"(?s)<[^>]+>", " ", text)
    text = (text.replace("&nbsp;", " ").replace("&amp;", "&")
                .replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'")
                .replace("&quot;", '"'))
    return re.sub(r"[ \t\r\f\v]+", " ", text).strip()


def fetch(url, timeout=TIMEOUT):
    """One public URL: `{url, status, title, text, emails}`.

    `text` is for the model to read; `emails` also includes addresses decoded out of
    `data-cfemail`, which a text-only scan never sees. Nothing is executed and nothing is sent.
    """
    request = urllib.request.Request(
        url, headers={"User-Agent": UA,
                      "Accept": "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8"})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read()
            status = response.status
            ctype = response.headers.get("Content-Type", "")
    except urllib.error.HTTPError as exc:
        raise ResearchError(f"HTTP {exc.code} fetching {url}") from exc
    except Exception as exc:  # noqa: BLE001
        raise ResearchError(f"fetch failed: {exc}") from exc

    charset = "utf-8"
    if "charset=" in ctype.lower():
        charset = ctype.lower().split("charset=", 1)[1].split(";", 1)[0].strip() or "utf-8"
    html = raw.decode(charset, errors="replace")
    title_match = re.search(r"(?is)<title[^>]*>(.*?)</title>", html)
    text = _html_to_text(html)
    emails = emails_in(text)
    for encoded in re.findall(r'data-cfemail=["\']([0-9a-fA-F]+)["\']', html):
        decoded = decode_cfemail(encoded)
        if decoded and _EMAIL.fullmatch(decoded) and decoded.lower() not in {e.lower() for e in emails}:
            emails.append(decoded)
    return {"url": url, "status": status,
            "title": (title_match.group(1).strip() if title_match else ""),
            "text": text[:8000], "emails": emails[:20]}


# --- a real browser, for the pages a plain GET cannot read -------------------
NODE_PATHS = (
    os.path.expanduser("~/.hermes/node/bin/node"),
    "/usr/bin/node",
)


def _node_bin():
    """The Node binary, by absolute path.

    The dashboard service runs with systemd's minimal PATH, which does not include `~/.local/bin`,
    so `shutil.which("node")` is not enough when this runs from the brief generator.
    """
    override = os.environ.get("BROWSER_NODE") or os.environ.get("NODE")
    if override and os.path.exists(override):
        return override
    found = shutil.which("node")
    if found:
        return found
    for candidate in NODE_PATHS + tuple(sorted(glob.glob(
            os.path.expanduser("~/.hermes/tools/node-*/bin/node")))):
        if os.path.exists(candidate):
            return candidate
    return "node"


def browser_fetch(url, timeout=75):
    """`fetch`, but in a real browser: JavaScript runs and the rendered DOM is read.

    Returns the same shape as `fetch`, plus `final_url` (redirects) and `status`. Raises rather
    than returning a broken page, so the tool loop reports it and the model can try another route.
    """
    helper = os.path.join(os.path.dirname(os.path.abspath(__file__)), "browser_fetch.mjs")
    if not os.path.exists(helper):
        raise ResearchError("browser helper server/brief/browser_fetch.mjs is missing")
    try:
        proc = subprocess.run([_node_bin(), helper, url], capture_output=True, text=True,
                              timeout=timeout)
    except FileNotFoundError as exc:
        raise ResearchError(f"node is not available: {exc}") from exc
    except subprocess.TimeoutExpired as exc:
        raise ResearchError(f"browser timed out after {timeout}s") from exc
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()
        raise ResearchError(f"browser failed: {detail[-300:]}")
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise ResearchError(f"browser returned invalid JSON: {exc}") from exc
    if data.get("error"):
        raise ResearchError(str(data["error"]))
    return data


# --- GitHub commit metadata -------------------------------------------------
def _github_token():
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token:
        return token
    try:
        out = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True, timeout=10)
        return out.stdout.strip() or None
    except Exception:  # noqa: BLE001
        return None


def github_commit_emails(repo, limit=40):
    """`{repo, authors: [{name, email, commit_url}]}` from a public repo's commit metadata.

    This is the skill's "mine the company's own public code" step: commit authors carry work
    addresses the company published. `repo` is `owner/name`.
    """
    repo = (repo or "").strip().strip("/").removeprefix("https://github.com/")
    if repo.count("/") != 1:
        raise ResearchError("repo must look like 'owner/name'")
    url = f"https://api.github.com/repos/{repo}/commits?per_page={max(1, min(100, int(limit)))}"
    headers = {"Accept": "application/vnd.github+json", "User-Agent": UA}
    token = _github_token()
    if token:
        headers["Authorization"] = "Bearer " + token
    request = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
            data = json.load(response)
    except urllib.error.HTTPError as exc:
        raise ResearchError(f"GitHub HTTP {exc.code} for {repo}") from exc
    except Exception as exc:  # noqa: BLE001
        raise ResearchError(f"GitHub request failed: {exc}") from exc
    authors = []
    for commit in data if isinstance(data, list) else []:
        author = ((commit.get("commit") or {}).get("author") or {})
        email = (author.get("email") or "").strip()
        if not email or _EMAIL_JUNK.search(email):
            continue
        authors.append({"name": (author.get("name") or "").strip(),
                        "email": email, "commit_url": commit.get("html_url") or ""})
    return {"repo": repo, "authors": authors[:limit]}
