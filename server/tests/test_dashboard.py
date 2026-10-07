#!/usr/bin/env python3
"""Assertions for the dashboard server: the client is served, the API answers, and the
static handler cannot be walked out of `client/`.

The page used to be embedded in the server as a Python string. The server now lives in the
`server/` package and serves `client/` verbatim, exposing the data as JSON, so the things
worth pinning are the routes and the path check -- a refactor that broke the package path
would otherwise surface only as a blank page in a browser.

    /usr/bin/python3 server/tests/test_dashboard.py

The test starts the real handler on an ephemeral loopback port and drives it over HTTP, so it
exercises routing and MIME types rather than importing a helper and hoping.
"""
import http.client
import json
import os
import sys
import threading
from http.server import ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
from server.api.app import APIHandler, Handler  # noqa: E402

FAILURES = []


def ok(label, cond, detail=""):
    if not cond:
        FAILURES.append(f"{label} {detail}")


def main():
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=30)

    def request(path, method="GET"):
        conn.request(method, path)
        resp = conn.getresponse()
        body = resp.read()
        return resp.status, dict((k.lower(), v) for k, v in resp.getheaders()), body

    try:
        # --- the client is served -----------------------------------------
        status, headers, body = request("/")
        ok("GET / is 200", status == 200, status)
        ok("GET / is HTML", "text/html" in headers.get("content-type", ""),
           headers.get("content-type"))
        ok("GET / serves the client shell", b'id="rows"' in body and b"/js/main.js" in body)

        # Every asset the module graph imports must be reachable and typed correctly, or the
        # browser silently refuses to run it.
        for path, ctype in [("/styles.css", "text/css"),
                            ("/js/main.js", "text/javascript"),
                            ("/js/render.js", "text/javascript"),
                            ("/js/detail.js", "text/javascript"),
                            ("/js/table.js", "text/javascript"),
                            ("/js/api.js", "text/javascript"),
                            ("/js/util.js", "text/javascript"),
                            ("/js/health.js", "text/javascript")]:
            status, headers, _ = request(path)
            ok(f"GET {path} is 200", status == 200, status)
            ok(f"GET {path} is {ctype}", ctype in headers.get("content-type", ""),
               headers.get("content-type"))

        # The health page is a separate document from the postings table (AGENTS.md). It has
        # to be served and must be the one the nav links to, or the split quietly regresses.
        status, headers, body = request("/health.html")
        ok("GET /health.html is 200", status == 200, status)
        ok("GET /health.html is HTML", "text/html" in headers.get("content-type", ""),
           headers.get("content-type"))
        ok("GET /health.html loads health.js", b"/js/health.js" in body)
        _, _, index_body = request("/")
        ok("the postings page links to the health page", b'/health.html' in index_body)

        # --- HEAD answers, so a health probe is not told the service is down
        status, headers, body = request("/", method="HEAD")
        ok("HEAD / is 200", status == 200, status)
        ok("HEAD / has no body", body == b"", body[:40])

        # --- the API answers ----------------------------------------------
        status, headers, body = request("/api/dashboard")
        ok("GET /api/dashboard is 200", status == 200, status)
        ok("GET /api/dashboard is JSON", "application/json" in headers.get("content-type", ""),
           headers.get("content-type"))
        try:
            data = json.loads(body)
            for key in ("ran_at", "matched", "postings", "tiers", "boards_total",
                        "source_seconds", "sweep", "window"):
                ok(f"/api/dashboard carries {key!r}", key in data)
            ok("/api/dashboard postings is a list", isinstance(data.get("postings"), list))
        except Exception as exc:  # noqa: BLE001
            FAILURES.append(f"/api/dashboard is not valid JSON: {exc}")

        # --- a posting detail, using a real id from the feed ---------------
        postings = data.get("postings") or []
        if postings:
            pid = postings[0]["id"]
            status, headers, body = request("/api/posting/" + pid)
            ok("detail of a real posting is 200", status == 200, status)
            detail = json.loads(body)
            for key in ("posting", "description", "brief", "running", "stale", "has_script"):
                ok(f"detail carries {key!r}", key in detail)
            ok("detail echoes the posting id", str((detail.get("posting") or {}).get("id")) == str(pid))
        else:
            FAILURES.append("no postings in the portal feed; cannot exercise /api/posting")

        status, _, _ = request("/api/posting/deadbeefdeadbeef")
        ok("unknown posting 404s", status == 404, status)

        # --- unknown routes -------------------------------------------------
        status, _, _ = request("/api/nope")
        ok("unknown API route 404s", status == 404, status)
        status, _, _ = request("/does-not-exist.css")
        ok("unknown static file 404s", status == 404, status)

        # --- the deep-contact route answers, and refuses what it should ----------
        # An unknown posting must not spawn the generator, so this exercises the route without
        # launching a Hermes run.
        status, _, body = request("/api/deep-contacts/deadbeefdeadbeef", method="POST")
        ok("deep-contacts answers for an unknown posting", status == 200, status)
        try:
            ok("deep-contacts refuses an unknown posting",
               json.loads(body).get("started") is False)
        except Exception as exc:  # noqa: BLE001
            FAILURES.append(f"deep-contacts body is not JSON: {exc}")
        status, _, _ = request("/api/deep-contacts/bad!id", method="POST")
        ok("deep-contacts rejects a bad id", status == 400, status)

        # --- the static handler cannot escape client/ -----------------------
        for path in ("/../server/api/app.py", "/%2e%2e%2fserver/api/app.py",
                     "/js/../../server/api/app.py"):
            status, _, _ = request(path)
            ok(f"path walk {path!r} is refused", status in (403, 404), status)

        # --- API-only mode: no client, the API still answers --------------
        # This is the production split: tailscale serve hosts client/, and this process is
        # loopback-only. Every non-API path must 404 rather than leak the client.
        api_server = ThreadingHTTPServer(("127.0.0.1", 0), APIHandler)
        api_thread = threading.Thread(target=api_server.serve_forever, daemon=True)
        api_thread.start()
        api_conn = http.client.HTTPConnection("127.0.0.1", api_server.server_address[1], timeout=30)
        try:
            for path in ("/", "/index.html", "/styles.css", "/js/main.js"):
                api_conn.request("GET", path)
                resp = api_conn.getresponse(); resp.read()
                ok(f"api-only: {path} 404s", resp.status == 404, resp.status)
            api_conn.request("GET", "/api/dashboard")
            resp = api_conn.getresponse(); body = resp.read()
            ok("api-only: /api/dashboard is 200", resp.status == 200, resp.status)
            ok("api-only: /api/dashboard is JSON", b'"postings"' in body)
        finally:
            api_conn.close()
            api_server.shutdown()

        # --- the brief normalizer gives `reword` one shape ------------------
        # New briefs write objects; briefs cached before that schema hold free text. The
        # detail view renders both, so the split has to be pinned here rather than discovered
        # by opening a stale posting.
        from server.api.briefs import normalize_brief, normalize_reword
        rw = normalize_reword(
            'Apera AI bullet. Change "old wording here" to "new wording here." Reason sentence.')
        ok("reword where is split off", rw["where"] == "Apera AI bullet", rw["where"])
        ok("reword before is quoted", rw["before"] == "old wording here", rw["before"])
        ok("reword after is quoted", rw["after"] == "new wording here.", rw["after"])
        arrow = normalize_reword("Skills line: 'Tools: Git, Docker' -> 'Tools: Git, Docker, AWS.'")
        ok("arrow reword splits into before/after",
           arrow["before"] == "Tools: Git, Docker" and arrow["after"] == "Tools: Git, Docker, AWS.",
           arrow)
        advice = normalize_reword(
            'Apera AI bullet: name both sides - "React/TypeScript front end".')
        ok("advice-style rewrites keep a left-hand side",
           advice["where"] == "Apera AI bullet" and advice["advice"] == "name both sides"
           and advice["after"] == "React/TypeScript front end", advice)
        solo = normalize_reword("Skills, Languages line: keep the C++ line where it is")
        ok("an item with no quote splits at its first colon",
           solo["where"] == "Skills, Languages line"
           and solo["advice"] == "keep the C++ line where it is" and solo["after"] == "", solo)
        struct = normalize_brief(
            {"tailoring": {"reword": [{"where": "w", "before": "b", "after": "a"}]}})
        ok("object rewrites pass through",
           struct["tailoring"]["reword"][0] == {"where": "w", "before": "b",
                                                  "after": "a", "advice": "", "note": ""},
           struct)

        # --- the DeepSeek reply parser tolerates real-world slack -------------
        # JSON mode usually returns one clean object, but it has returned a fenced object and
        # an object followed by more text. A strict json.loads throws on both and the whole
        # brief was lost to an "Extra data" error.
        from server.brief.llm import parse_json
        ok("parse_json unwraps a code fence",
           parse_json('```json\n{"a": 1}\n```') == {"a": 1})
        ok("parse_json ignores trailing content",
           parse_json('{"a": 1}{"b": 2}') == {"a": 1})
        ok("parse_json finds the object inside prose",
           parse_json('Sure, here it is: {"a": 1} — done.') == {"a": 1})

        # --- the copied contact-research primitives --------------------------
        # The direct engine's address-verification guard rests on these: an address is only
        # returned if it appears verbatim in a tool result, and Cloudflare hides addresses from a
        # text-only read.
        from server.brief.research import decode_cfemail, emails_in, _node_bin
        from server.brief.generate import CONTACT_TOOLS, _emails_from, build_contact_agent_prompt
        email = "jane.doe@acme.com"
        key = 0x5A
        encoded = format(key, "02x") + "".join(format(ord(c) ^ key, "02x") for c in email)
        ok("cloudflare data-cfemail decodes", decode_cfemail(encoded) == email, decode_cfemail(encoded))
        ok("emails_in keeps real addresses and drops assets and no-reply",
           emails_in("write jane@acme.com, logo@2x.png, no-reply@acme.com") == ["jane@acme.com"],
           emails_in("write jane@acme.com, logo@2x.png, no-reply@acme.com"))
        ok("emails are found anywhere in a nested tool result",
           "jane@acme.com" in _emails_from({"authors": [{"email": "jane@acme.com"}],
                                            "text": "or bob@acme.com"}))
        tool_names = {tool["function"]["name"] for tool in CONTACT_TOOLS}
        ok("the contact loop exposes the local browser",
           tool_names == {"web_search", "web_fetch", "browser_fetch", "github_commit_emails"},
           tool_names)
        deep = build_contact_agent_prompt(
            {"id": "abc123", "company": "Acme", "title": "SWE",
             "location": "Vancouver", "url": "https://example.test/job"}, "the posting text")
        ok("the deep-contact prompt names its output path", "abc123.agent.json" in deep)
        ok("the deep-contact prompt forbids sending", "Do not send any email" in deep)
        ok("the deep-contact prompt carries the contact rules",
           "never guess an address" in deep)
        root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        ok("the browser helper exists",
           os.path.exists(os.path.join(root, "server", "brief", "browser_fetch.mjs")))
        # Under the service's minimal PATH `which node` fails, so the generator resolves node by
        # absolute path; the test shell has node on PATH, so this only pins that a path is found.
        ok("node is resolvable for the browser helper", bool(_node_bin()), _node_bin())
    finally:
        conn.close()
        server.shutdown()

    if FAILURES:
        print(f"{len(FAILURES)} failure(s):\n")
        for f in FAILURES:
            print("  - " + f)
        return 1
    print("dashboard test: all assertions passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
