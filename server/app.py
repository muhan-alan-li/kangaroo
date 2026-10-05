"""The HTTP surface: JSON API routes plus the static client.

The API is three routes, and the static handler serves everything else out of `client/`. The
handler is deliberately thin -- the state work lives in `store.py` and `briefs.py` -- so the
routes are readable as routes.
"""
from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler
from urllib.parse import unquote

from .briefs import brief_for, start_brief, start_contacts
from .config import CLIENT, MIME
from .store import collect, description_for, find_row


class Handler(BaseHTTPRequestHandler):
    server_version = "jobscan-dashboard"
    # In production `tailscale serve` hosts `client/` on the tailnet and this process is
    # loopback-only, so `APIHandler` turns static serving off. The default serves both, which
    # is what local development wants.
    serve_client = True

    def _json(self, payload, code=200):
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if body:
            self.wfile.write(body)

    def _static(self, path, head_only=False):
        """Serve one file out of client/.

        `path` has already had its query string removed. The real path is resolved and checked
        against the client root, so a `..` segment -- however it is percent-encoded -- cannot
        climb out of the directory.
        """
        rel = unquote(path).lstrip("/") or "index.html"
        try:
            root = os.path.realpath(CLIENT)
            target = os.path.realpath(os.path.join(root, rel))
        except (ValueError, OSError):
            self.send_error(400, "bad path")
            return
        if target != root and not target.startswith(root + os.sep):
            self.send_error(403, "forbidden")
            return
        if os.path.isdir(target):
            target = os.path.join(target, "index.html")
        if not os.path.isfile(target):
            self.send_error(404, "not found")
            return
        try:
            with open(target, "rb") as fh:
                body = fh.read()
        except OSError:
            self.send_error(404, "not found")
            return
        ctype = MIME.get(os.path.splitext(target)[1].lower(), "application/octet-stream")
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        # no-store: a redeployed client must show up on the next reload, not be pinned in a
        # phone's cache. The files are small and the page is refreshed by hand anyway.
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        if body and not head_only:
            self.wfile.write(body)

    def _detail(self, pid):
        """Everything the detail view shows for one posting. Read-only, no network."""
        row = find_row(pid)
        if not row:
            return {"error": f"no posting {pid}"}, 404
        text = description_for(pid, row)
        state = brief_for(pid)
        return {
            "posting": row,
            # Trimmed: the panel scrolls, and a 30 KB description would be sent on every poll.
            "description": text[:12000],
            "description_length": len(text),
            "brief": state["brief"],
            "running": state["running"],
            "stale": state["stale"],
            "started_at": state["started_at"],
            "has_script": state["has_script"],
        }, 200

    def do_GET(self):  # noqa: N802 -- BaseHTTPRequestHandler's spelling
        path = self.path.split("?")[0]
        if path == "/api/dashboard":
            self._json(collect())
            return
        if path.startswith("/api/posting/"):
            payload, code = self._detail(unquote(path[len("/api/posting/"):]))
            self._json(payload, code)
            return
        if path.startswith("/api/"):
            self.send_error(404, "not found")
            return
        if not self.serve_client:
            self.send_error(404, "not found")
            return
        self._static(path)

    def do_POST(self):  # noqa: N802
        """Start a brief. The only route that writes anything, and it writes one file.

        The generation is an agent run, so this cannot happen inside the request: the POST
        launches `bin/posting_brief.py` detached and answers at once, and the page polls
        GET /api/posting/<id> for the result.
        """
        path = self.path.split("?")[0]
        for prefix, starter in (("/api/brief/", start_brief),
                                ("/api/deep-contacts/", start_contacts)):
            if not path.startswith(prefix):
                continue
            pid = unquote(path[len(prefix):]).strip()
            if not pid or any(c not in "0123456789abcdefABCDEF-" for c in pid):
                self._json({"error": "bad posting id"}, 400)
                return
            ok, message = starter(pid)
            self._json({"started": ok, "message": message, "running": brief_for(pid)["running"]})
            return
        self.send_error(404)

    def do_HEAD(self):  # noqa: N802
        """Answer HEAD. Without it the server returns 501, and a client or a proxy that probes
        with HEAD reads that as the service being down."""
        path = self.path.split("?")[0]
        if path.startswith("/api/"):
            self.send_error(404)
            return
        if not self.serve_client:
            self.send_error(404)
            return
        self._static(path, head_only=True)

    def log_message(self, fmt, *args):  # quiet by default; the service log is enough
        pass


class APIHandler(Handler):
    """The API alone, for when `tailscale serve` hosts `client/`.

    Same routes as `Handler`; every non-API path, including `/`, answers 404 because this
    process is not on the network path for the page.
    """

    serve_client = False
