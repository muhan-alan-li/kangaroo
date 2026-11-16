#!/usr/bin/env python3
"""Assertions for the scan's egress guard: the DNS cache, the resolver pause, stale-on-error,
and the in-flight socket gate.

These are the brakes that keep the scan from tripping the local resolver's per-client rate
limit, and every one of them is invisible when it works. The failure they prevent is loud and
total -- a scan that loses all 1,288 Workday boards and takes 900 seconds -- so the pieces
are pinned here.

    /usr/bin/python3 server/tests/test_net.py
"""

from __future__ import annotations

import os
import socket
import sys
import time
import urllib.error

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))  # server/, so the `scan` package resolves

from scan import net  # noqa: E402

failures = []
checks = 0


def check(label, got, want):
    global checks
    checks += 1
    if got != want:
        failures.append(label)
        print(f"  MISMATCH {label}\n    got : {got!r}\n    want: {want!r}")


# Install with a forgiving bucket and a short breaker so the test does not sleep. Settings
# are read at install() time, so they go in first.
net._SETTINGS.update({
    "max_sockets": 3,
    "dns_cache_seconds": 300.0,
    "dns_nx_seconds": 15.0,
    "dns_per_second": 1_000_000.0,
    "dns_burst": 1_000_000.0,
    "dns_breaker_failures": 3,
    "dns_breaker_seconds": 30.0,
})
net.install()

# Swap the real resolver for a counter, so no test ever touches the network.
calls = []


def resolver(host, port, family=0, type=0, proto=0, flags=0):
    calls.append(host)
    if host.startswith("bad"):
        raise socket.gaierror(socket.EAI_AGAIN, "temporary failure in name resolution")
    if host.startswith("gone"):
        raise socket.gaierror(socket.EAI_NONAME, "no such host")
    return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("127.0.0.1", port))]


net._ORIG_GETADDRINFO = resolver

ADDR = [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("127.0.0.1", 443))]


def repeat(host, times=1):
    out = []
    for _ in range(times):
        out.append(net._cached_getaddrinfo(host, 443))
    return out


def raises(host):
    try:
        net._cached_getaddrinfo(host, 443)
    except socket.gaierror as exc:
        return exc.errno
    return None


# ---- the cache: a name is resolved once -----------------------------------------------
net.reset()
check("a fresh name is a query", repeat("api.smartrecruiters.com"), [ADDR])
check("the same name is a cache hit", repeat("api.smartrecruiters.com", 5), [ADDR] * 5)
check("...and only one query left the process", calls, ["api.smartrecruiters.com"])
check("a different name is a new query", repeat("boards-api.greenhouse.io"), [ADDR])
check("two names, two queries", calls, ["api.smartrecruiters.com", "boards-api.greenhouse.io"])
check("hits are counted", net.stats()["hits"], 5)
check("the port is part of the key",
      net._cached_getaddrinfo("api.smartrecruiters.com", 8443),
      [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("127.0.0.1", 8443))])
check("an IP literal skips the cache", repeat("127.0.0.1"), [ADDR])

# ---- negative caching: a refusal is not asked again immediately -----------------------
net.reset()
calls.clear()
check("a temporary failure surfaces", raises("bad-a.example"), socket.EAI_AGAIN)
check("the retry is answered from the negative cache",
      raises("bad-a.example"), socket.EAI_AGAIN)
check("...so the resolver was asked once", calls.count("bad-a.example"), 1)

# EAI_NONAME is a per-host fact, not the resolver being unhappy...
net.reset()
calls.clear()
check("a permanent failure surfaces", raises("gone.example"), socket.EAI_NONAME)
check("a permanent failure is also cached", raises("gone.example"), socket.EAI_NONAME)
check("...and asked once", calls.count("gone.example"), 1)

# ---- stale-on-error: a name that resolved keeps working -------------------------------
net.reset()
calls.clear()
check("a good name resolves", repeat("stale.example"), [ADDR])
# Expire the entry without waiting, then make the resolver refuse.
with net._lock:
    net._cache[("stale.example", 443, 0, 0, 0)][0] = time.monotonic() - 1
net._ORIG_GETADDRINFO = lambda *a, **k: (_ for _ in ()).throw(
    socket.gaierror(socket.EAI_AGAIN, "refused"))
check("an expired name is served stale when the resolver refuses",
      repeat("stale.example"), [ADDR])
check("the stale answer is counted", net.stats()["stale"], 1)
check("the stale answer is reused, not re-queried",
      net._cache[("stale.example", 443, 0, 0, 0)][1], ADDR)

# ---- the resolver pause: repeated temporary failures stop the questions ----------------
net._ORIG_GETADDRINFO = resolver
net.reset()
calls.clear()
check("failure one", raises("bad-1.example"), socket.EAI_AGAIN)
check("failure two", raises("bad-2.example"), socket.EAI_AGAIN)
check("failure three trips the pause", raises("bad-3.example"), socket.EAI_AGAIN)
check("the pause opened", net.stats()["breaker_trips"], 1)
before = len(calls)
check("a new name is refused without asking", raises("new.example"), socket.EAI_AGAIN)
check("...and the resolver was not asked", len(calls), before)
check("the refusal is counted as a skip", net.stats()["breaker_skips"], 1)

# ---- is_dns_error unwraps what urllib wraps -------------------------------------------
check("a bare gaierror is a DNS error", net.is_dns_error(socket.gaierror(-3, "x")), True)
check("urllib's wrapper is a DNS error",
      net.is_dns_error(urllib.error.URLError(socket.gaierror(-3, "x"))), True)
check("a timeout is not a DNS error",
      net.is_dns_error(urllib.error.URLError(TimeoutError("timed out"))), False)
check("a plain exception is not a DNS error", net.is_dns_error(RuntimeError("boom")), False)

# ---- the socket gate: a slot comes back exactly once ----------------------------------
released = []


class FakeResponse:
    def __init__(self):
        self.closed = 0

    def close(self):
        self.closed += 1


body = FakeResponse()
gated = net._GatedResponse(body, lambda: released.append(1))
with gated as resp:
    check("the wrapper delegates reads", hasattr(resp, "close"), True)
check("the slot is released on exit", released, [1])
gated.close()
check("releasing twice does not double-free", released, [1])
check("the underlying response was closed once", body.closed, 1)

# A response nobody closes still releases when it is collected.
released.clear()
body2 = FakeResponse()
leaked = net._GatedResponse(body2, lambda: released.append(1))
del leaked
check("an unclosed response releases on collection", released, [1])

print(f"\n{checks} checks, {len(failures)} mismatches")
if failures:
    print("FAILED:")
    for f in failures:
        print(f"  {f}")
    sys.exit(1)
