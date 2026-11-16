"""The scan's egress guard: DNS caching, DNS pacing, a resolver pause, and a ceiling on
open sockets.

The scan talks to a few thousand distinct ATS hosts and, on the way, asks the resolver the
same question over and over. `api.smartrecruiters.com` is one name, but it used to be
resolved once per board -- several hundred times in a single run; `boards-api.greenhouse.io`,
`api.ashbyhq.com`, `apply.workable.com` and `ats.rippling.com` behave the same way. urllib
does no connection reuse and holds no DNS cache, so every `urlopen` is a fresh lookup.

That firehose is what tripped the local resolver's per-client rate limit (Pi-hole's default is
1000 queries in 60 seconds, answered REFUSED). The failure then cascaded: once REFUSED, every
later lookup failed too -- including a Workday tenant's one and only name -- so a whole run
lost all 1,288 boards, and the sources' retries kept asking, which is exactly what makes
Pi-hole extend the block. A 3-minute scan became a 900-second washout.

This module fixes it at the process boundary, so no source has to remember to be polite:

  * a TTL cache, so a name is resolved once and reused. This is the big one: it removes
    the duplicate lookups that actually caused the block;
  * a token bucket, so `getaddrinfo` calls leave the process no faster than the resolver
    can absorb them, however many worker threads are asking;
  * a resolver pause (circuit breaker), so a run of *temporary* failures stops the queries
    instead of feeding the block. Pi-hole lengthens its block while a client keeps asking;
  * stale-on-error, so a name that ever resolved keeps working through a resolver wobble;
  * a semaphore capping requests in flight, so the process cannot open hundreds of sockets
    at once no matter how the sources' per-source pools add up.

Installed from `scan/core.py` at import, because every source and scheduled job reaches the
network through that module. `net.line()` reports what a run did.
"""

from __future__ import annotations

import os
import socket
import sys
import threading
import time
import urllib.request


# --------------------------------------------------------------------------- settings

def _load_settings():
    """`data/config/sources.yaml`'s `network:` block, then the environment, then defaults.

    The YAML is the documented, reviewable place to tune these; the environment variables
    exist so a shell can run one scan with different brakes without editing the file.
    """
    cfg = {}
    try:
        import yaml
        root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        with open(os.path.join(root, "data", "config", "sources.yaml"), encoding="utf-8") as fh:
            cfg = (yaml.safe_load(fh) or {}).get("network") or {}
    except Exception:  # noqa: BLE001 -- a missing config is not a reason to fail
        cfg = {}

    def number(key, env, default, cast):
        value = cfg.get(key)
        if value is None:
            value = os.environ.get(env)
        try:
            return cast(value)
        except (TypeError, ValueError):
            return default

    return {
        "max_sockets": max(1, number("max_sockets", "JOBSCAN_MAX_SOCKETS", 32, int)),
        "dns_cache_seconds": max(0.0, number("dns_cache_seconds", "JOBSCAN_DNS_TTL",
                                             300.0, float)),
        "dns_nx_seconds": max(1.0, number("dns_nx_seconds", "JOBSCAN_DNS_NX_TTL",
                                          15.0, float)),
        "dns_per_second": max(0.1, number("dns_per_second", "JOBSCAN_DNS_RATE",
                                          10.0, float)),
        "dns_burst": max(1.0, number("dns_burst", "JOBSCAN_DNS_BURST", 20.0, float)),
        "dns_breaker_failures": max(1, number("dns_breaker_failures",
                                              "JOBSCAN_DNS_BREAKER_FAILURES", 12, int)),
        "dns_breaker_seconds": max(1.0, number("dns_breaker_seconds",
                                               "JOBSCAN_DNS_BREAKER_SECONDS", 60.0, float)),
    }


_SETTINGS = _load_settings()

# The real implementations. `install()` swaps the module-level names the rest of the process
# looks up, so a caller that wrote `urllib.request.urlopen(...)` gets the gated one.
_ORIG_GETADDRINFO = socket.getaddrinfo
_ORIG_URLOPEN = urllib.request.urlopen

_installed = False
_bucket = None            # the DNS token bucket
_slots = None             # the in-flight socket semaphore

_lock = threading.Lock()  # guards everything below
_installed_lock = threading.Lock()
_cache = {}               # key -> [expiry, addrs|None, errno|None]
_host_locks = {}          # key -> Lock, the per-name single-flight gate
_fail_streak = 0
_breaker_until = 0.0

_counters = {
    "hits": 0,            # lookups answered from the cache (no query)
    "misses": 0,          # real queries sent to the resolver
    "stale": 0,           # answered from an expired entry after a failed refresh
    "failures": 0,        # temporary resolver failures
    "breaker_trips": 0,   # times the resolver pause opened
    "breaker_skips": 0,   # lookups refused while the pause was open
}

# errno values that mean "the resolver is unhappy, not this name" -- the ones worth pausing
# for. EAI_NONAME (a genuinely bad host) is a per-host fact and must not trip the breaker.
_TEMPORARY_GAI = tuple(
    value for value in (
        getattr(socket, "EAI_AGAIN", None),
        getattr(socket, "EAI_FAIL", None),
        getattr(socket, "EAI_TRYAGAIN", None),
        getattr(socket, "EAI_SYSTEM", None),
    ) if value is not None
)


# --------------------------------------------------------------------------- token bucket

class _TokenBucket:
    """A leaky allowance of `rate` permits per second, with a `capacity` burst."""

    def __init__(self, rate, capacity):
        self.rate = float(rate)
        self.capacity = float(capacity)
        self.tokens = float(capacity)
        self.updated = time.monotonic()
        self.lock = threading.Lock()

    def take(self):
        while True:
            with self.lock:
                now = time.monotonic()
                self.tokens = min(self.capacity,
                                  self.tokens + (now - self.updated) * self.rate)
                self.updated = now
                if self.tokens >= 1.0:
                    self.tokens -= 1.0
                    return
                wait = (1.0 - self.tokens) / self.rate
            # Sleep outside the lock so waiters can recompute; cap the nap so a long wait
            # still notices a settings change or a clock jump.
            time.sleep(min(wait, 1.0))


# --------------------------------------------------------------------------- classification

def is_dns_error(exc):
    """True when an exception is a name-resolution failure, however urllib wrapped it.

    Retrying a lookup that the resolver just refused does not help -- it feeds the rate
    limit. Callers use this to fail fast instead of burning their retry budget.
    """
    seen = set()
    while exc is not None and id(exc) not in seen:
        seen.add(id(exc))
        if isinstance(exc, socket.gaierror):
            return True
        inner = getattr(exc, "reason", None)
        if inner is None:
            inner = getattr(exc, "__cause__", None)
        exc = inner
    return False


def _is_ip_literal(host):
    if not isinstance(host, str):
        return False
    for family in (socket.AF_INET, socket.AF_INET6):
        try:
            socket.inet_pton(family, host)
            return True
        except OSError:
            continue
    return False


# --------------------------------------------------------------------------- the DNS cache

def _cache_raw(key):
    with _lock:
        return _cache.get(key)


def _serve_fresh(entry):
    """A fresh cached entry: return its addresses, or raise its cached negative."""
    with _lock:
        _counters["hits"] += 1
    if entry[1] is None:
        raise socket.gaierror(entry[2] if entry[2] is not None else socket.EAI_AGAIN,
                              "cached name-resolution failure")
    return list(entry[1])


def _note_failure_locked(exc):
    """Count a failure and open the pause once the streak is long enough. Caller holds `_lock`."""
    global _fail_streak, _breaker_until
    if getattr(exc, "errno", None) not in _TEMPORARY_GAI:
        _fail_streak = 0
        return
    _fail_streak += 1
    if _fail_streak >= _SETTINGS["dns_breaker_failures"]:
        _fail_streak = 0
        _breaker_until = time.monotonic() + _SETTINGS["dns_breaker_seconds"]
        _counters["breaker_trips"] += 1
        sys.stderr.write(
            "[scan] net: the resolver refused %d lookups in a row; pausing DNS for %.0fs "
            "so the rate limit can clear\n"
            % (_SETTINGS["dns_breaker_failures"], _SETTINGS["dns_breaker_seconds"]))


def _note_success_locked():
    """A real answer clears the streak and closes the pause. Caller holds `_lock`."""
    global _fail_streak, _breaker_until
    _fail_streak = 0
    _breaker_until = 0.0


def _cached_getaddrinfo(host, port, family=0, type=0, proto=0, flags=0):
    """`socket.getaddrinfo`, cached, paced, and paused when the resolver is refusing.

    The signature mirrors the stdlib's, argument names included, because callers pass the
    family/type/proto/flags positionally.
    """
    if not host or flags or _is_ip_literal(host):
        return _ORIG_GETADDRINFO(host, port, family, type, proto, flags)

    key = (host, port, family, type, proto)
    now = time.monotonic()
    entry = _cache_raw(key)
    if entry is not None and entry[0] > now:
        return _serve_fresh(entry)

    # Single-flight: one thread resolves a name; the others wait and then hit the cache.
    # Without this, 32 workers that all miss on `api.smartrecruiters.com` at once issue 32
    # identical queries, which is the very burst this module exists to remove.
    with _lock:
        host_lock = _host_locks.setdefault(key, threading.Lock())
    with host_lock:
        now = time.monotonic()
        entry = _cache_raw(key)
        if entry is not None and entry[0] > now:
            return _serve_fresh(entry)
        stale = entry          # expired, but usable if the refresh fails

        with _lock:
            paused = now < _breaker_until
        if paused:
            with _lock:
                _counters["breaker_skips"] += 1
            if stale is not None and stale[1] is not None:
                # Serve the old address and remember it for the pause, so the next caller
                # does not ask again and deepen the block.
                with _lock:
                    _counters["stale"] += 1
                    _cache[key] = [time.monotonic() + _SETTINGS["dns_nx_seconds"],
                                   stale[1], None]
                return list(stale[1])
            raise socket.gaierror(
                socket.EAI_AGAIN,
                "name resolution paused: the resolver was refusing lookups")

        _bucket.take()
        with _lock:
            _counters["misses"] += 1
        try:
            addrs = _ORIG_GETADDRINFO(host, port, family, type, proto, flags)
        except socket.gaierror as exc:
            with _lock:
                _counters["failures"] += 1
                _note_failure_locked(exc)
            if stale is not None and stale[1] is not None:
                # Stale-on-error: a name that resolved before keeps working. This is what
                # turns a resolver wobble from "lose every board" into "lose nothing".
                with _lock:
                    _counters["stale"] += 1
                    _cache[key] = [time.monotonic() + _SETTINGS["dns_nx_seconds"],
                                   stale[1], None]
                return list(stale[1])
            with _lock:
                _cache[key] = [time.monotonic() + _SETTINGS["dns_nx_seconds"], None,
                               getattr(exc, "errno", None)]
            raise
        with _lock:
            _cache[key] = [time.monotonic() + _SETTINGS["dns_cache_seconds"], addrs, None]
            _note_success_locked()
        return list(addrs)


# --------------------------------------------------------------------------- the socket gate

class _GatedResponse:
    """A urlopen response that returns its in-flight slot exactly once, on close or GC."""

    __slots__ = ("_response", "_release", "_released")

    def __init__(self, response, release):
        self._response = response
        self._release = release
        self._released = False

    def _finish(self):
        if self._released:
            return
        self._released = True
        try:
            self._response.close()
        finally:
            self._release()

    def close(self):
        self._finish()

    def __enter__(self):
        return self

    def __exit__(self, *exc_info):
        self._finish()
        return False

    def __getattr__(self, name):
        return getattr(self._response, name)

    def __del__(self):
        try:
            self._finish()
        except Exception:  # noqa: BLE001 -- interpreter shutdown, nothing to report
            pass


def _gated_urlopen(*args, **kwargs):
    """`urllib.request.urlopen` behind the process-wide socket semaphore.

    The slot is held for the life of the response -- `with urlopen(...) as resp` releases it
    on exit, and a response nobody closes releases it when it is collected.
    """
    _slots.acquire()
    try:
        response = _ORIG_URLOPEN(*args, **kwargs)
    except BaseException:
        _slots.release()
        raise
    return _GatedResponse(response, _slots.release)


# --------------------------------------------------------------------------- install

def install():
    """Patch the process once. Safe to call from every importer."""
    global _installed, _bucket, _slots
    with _installed_lock:
        if _installed:
            return
        _bucket = _TokenBucket(_SETTINGS["dns_per_second"], _SETTINGS["dns_burst"])
        _slots = threading.BoundedSemaphore(_SETTINGS["max_sockets"])
        socket.getaddrinfo = _cached_getaddrinfo
        urllib.request.urlopen = _gated_urlopen
        _installed = True


# --------------------------------------------------------------------------- reporting

def stats():
    with _lock:
        out = dict(_counters)
    out["installed"] = _installed
    out["max_sockets"] = _SETTINGS["max_sockets"]
    out["dns_per_second"] = _SETTINGS["dns_per_second"]
    return out


def line():
    s = stats()
    return ("net: %d lookups, %d cached, %d stale, %d failed, %d resolver pauses "
            "(<= %d sockets, %.0f DNS/s)"
            % (s["misses"], s["hits"], s["stale"], s["failures"], s["breaker_trips"],
               s["max_sockets"], s["dns_per_second"]))


def reset():
    """Clear the cache, counters and pause, and refill the bucket. Tests only."""
    global _fail_streak, _breaker_until
    with _lock:
        _cache.clear()
        _host_locks.clear()
        _fail_streak = 0
        _breaker_until = 0.0
        for key in _counters:
            _counters[key] = 0
    if _bucket is not None:
        with _bucket.lock:
            _bucket.tokens = _bucket.capacity
            _bucket.updated = time.monotonic()
