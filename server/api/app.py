"""The HTTP surface: JSON API routes plus the static client.

The API routes read state and start the two brief jobs; every other path is served from
`client/` when this process is also hosting the page. The handler is deliberately thin --
the state work lives in `store.py`, `triage.py` and `briefs.py` -- so the routes are readable
as routes.
"""
from __future__ import annotations

import json
import os
from http.server import BaseHTTPRequestHandler
from urllib.parse import unquote

from .briefs import brief_for, start_brief, start_contacts
from ..config import CLIENT, MIME
from . import triage
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

    def _body(self):
        """The JSON body of a write request, or None. Bounded, so a bad client cannot make the
        server allocate a request-sized buffer."""
        try:
            length = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            return None
        if length <= 0 or length > 1_000_000:
            return None
        try:
            return json.loads(self.rfile.read(length).decode("utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            return None

    def _mark_applied(self, pid):
        """POST /api/applied/<id>: snapshot the posting into the applied list.

        The answer carries the whole applied list, so the page can repaint the two tables and
        the counts without a second request for the 183 KB dashboard.
        """
        if not pid or any(c not in "0123456789abcdefABCDEF-" for c in pid):
            self._json({"error": "bad posting id"}, 400)
            return
        row = find_row(pid)
        if not row:
            self._json({"error": f"no posting {pid}"}, 404)
            return
        triage.mark(row)
        self._json({"marked": True, "applied": triage.rows()})

    def _remove_applied(self, ids):
        """POST /api/applied/remove: drop a selection, one write for the whole batch."""
        if not isinstance(ids, list):
            self._json({"error": 'expected {"ids": [...]}'}, 400)
            return
        removed = triage.remove(ids)
        self._json({"removed": removed, "applied": triage.rows()})

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
        """The write routes: start a brief, or move a posting on and off the applied list.

        Brief generation runs a model call outside the request: the POST launches
        `server/brief/generate.py` detached and answers at once, and the page polls
        GET /api/posting/<id> for the result. The applied writes are just a file, so they answer
        with what changed.
        """
        path = self.path.split("?")[0]
        if path == "/api/applied/remove":
            body = self._body()
            self._remove_applied((body or {}).get("ids") if isinstance(body, dict) else None)
            return
        if path.startswith("/api/applied/"):
            self._mark_applied(unquote(path[len("/api/applied/"):]).strip())
            return
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
