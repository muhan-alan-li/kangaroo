"""Command line: bind the addresses and serve.

    python3 -m server                 # the Tailscale address and loopback, port 8787
    python3 -m server --port 9000
    python3 -m server --host 127.0.0.1   # this machine only

Two binds by default, and both matter. Loopback is what `tailscale serve` proxies to, so
without it the nice hostname cannot be set up. The Tailscale address is what works before the
proxy exists, and stays as the fallback if the proxy is ever torn down. Neither of them is
0.0.0.0, so the LAN still sees nothing.
"""
from __future__ import annotations

import argparse
import sys
import threading
import time
from http.server import ThreadingHTTPServer

from .app import APIHandler, Handler
from .net import tailnet_ip


def main(argv=None):
    ap = argparse.ArgumentParser(prog="server", description="the job-hunt dashboard server")
    ap.add_argument("--host", action="append", default=None,
                    help="bind address; repeatable (default: the Tailscale address and loopback, "
                         "or loopback alone with --api-only)")
    ap.add_argument("--wait-for-tailnet", type=float, default=90,
                    help="seconds to wait for a Tailscale address before falling back to loopback")
    ap.add_argument("--port", type=int, default=8787)
    ap.add_argument("--api-only", action="store_true",
                    help="serve only /api/* and bind loopback, for when tailscale serve hosts client/")
    args = ap.parse_args(argv)

    handler = APIHandler if args.api_only else Handler
    # Two binds by default, and both matter. Loopback is what `tailscale serve` proxies to, so
    # without it the nice hostname cannot be set up. The Tailscale address is what works before
    # the proxy exists, and stays as the fallback if the proxy is ever torn down. Neither of
    # them is 0.0.0.0, so the LAN still sees nothing. --api-only binds loopback alone: the API
    # is reached through the tailnet proxy, never directly.
    if args.host:
        hosts = args.host
    elif args.api_only:
        hosts = ["127.0.0.1"]
    else:
        hosts = [tailnet_ip(args.wait_for_tailnet), "127.0.0.1"]

    servers = []
    for host in hosts:
        try:
            servers.append((host, ThreadingHTTPServer((host, args.port), handler)))
        except OSError as exc:
            sys.stderr.write(f"could not bind {host}:{args.port}: {exc}\n")
    if not servers:
        sys.stderr.write("nothing bound, giving up\n")
        return 1

    mode = "API only; client/ is hosted by tailscale serve" if args.api_only else "client/ + API"
    for host, _srv in servers:
        print(f"job scan dashboard on http://{host}:{args.port}/  [{mode}]")
    print("  reading data/state/ on every request", file=sys.stderr)

    threads = []
    for _host, srv in servers:
        t = threading.Thread(target=srv.serve_forever, daemon=True)
        t.start()
        threads.append((t, srv))
    try:
        # Stay up while any bind is still serving. Each server runs in its own thread, so the
        # main thread has nothing to do but wait and keep the process alive.
        while any(worker.is_alive() for worker, _srv in threads):
            time.sleep(1)
    except KeyboardInterrupt:
        pass
    for _t, srv in threads:
        srv.shutdown()
    return 0
