# S3 task: Network guard

This slice builds the Rust scan engine's egress guard. It is the one part of the port that
decides whether the app can scan at all on the owner's network, so parity here is not
cosmetic: a deviation re-creates the production failure in §1.

Read these first, in this order:

1. `docs/rust-app-plan.md` §10.1 and §16.1.
2. `server/scan/net.py` — the specification. The comments explain the intent; the code is
   the contract. Where they disagree, the code wins.
3. `server/tests/test_net.py` — the parity oracle. It is 34 `check()` assertions and touches
   no network. Run it once with `/usr/bin/python3 server/tests/test_net.py`.
4. `server/scan/core.py` lines 33-39 and 56-79 — `UA`, `net.install()`, and `http_get`.
5. `data/config/sources.yaml` lines 14-28 — the `network:` block.

Work in this directory only. Do not modify `server/`, `client/` or `data/`. One commit at
the end: `feat: Add the network guard`. Never push.

## 1. The production failure this prevents

The scan talks to a few thousand distinct ATS hosts and, on the way, asks the resolver the
same question over and over. `api.smartrecruiters.com` is one name, but it used to be
resolved once per board — several hundred times in a single run; `boards-api.greenhouse.io`,
`api.ashbyhq.com`, `apply.workable.com` and `ats.rippling.com` behave the same way. `urllib`
does no connection reuse and holds no DNS cache, so every `urlopen` is a fresh lookup.

That firehose tripped the local resolver's per-client rate limit: **Pi-hole's default is 1000
queries in 60 seconds; above that it answers REFUSED.** The failure then cascaded. Once
REFUSED, every later lookup failed too — including a Workday tenant's one and only name — so
a whole run lost all **1,288** Workday boards, and the sources' retries kept asking, which is
exactly what makes Pi-hole extend the block. A **3-minute** scan became a **900-second**
washout.

The guard fixes this at the process boundary so no source has to remember to be polite. Each
piece exists for a distinct reason, and each is stated in §4:

| Piece | Why it exists |
|---|---|
| TTL cache | Removes the duplicate lookups that actually caused the block. The big one. |
| Negative cache | A refused name must not be asked again immediately; the retry feeds the block. |
| Token bucket | Keeps `getaddrinfo` calls under the resolver's rate however many workers ask. |
| Consecutive-failure breaker | Stops the questions during a refusal storm so the rate limit can clear. |
| Stale-on-error | A name that ever resolved keeps working through a resolver wobble. |
| Process-wide socket cap | Per-source pools cannot add up to hundreds of open sockets. |
| No immediate DNS retry | Asking again after a refusal is what makes the resolver extend the block. |

The scheduled scan runs in a container whose `dns:` points at 1.1.1.1 / 8.8.8.8, so that run
never touches Pi-hole. The desktop app has no such bypass: it uses the Mac's system resolver,
which asks Pi-hole. The guard is the app's only protection. When it works, nothing is
visible; the `net:` line (§4.9) is the only evidence.

## 2. The contract: one client, one guard

Python installs the guard by patching the process. `server/scan/core.py:39` calls
`net.install()` at import, and `install()` (`net.py:353-363`) sets, once, under
`_installed_lock`:

```python
socket.getaddrinfo = _cached_getaddrinfo
urllib.request.urlopen = _gated_urlopen
```

Because every source and scheduled job reaches the network through `urllib`, every path gets
both the DNS guard and the socket gate — whether it calls `core.http_get`, or writes its own
`urlopen` with its own headers (`sources/workatastartup.py:64`), its own retry loop
(`sources/workday.py:119-132`), or is a job (`jobs/feed.py:36`, `jobs/probe_ats.py:281`). A
source cannot forget the guard.

`install()` is idempotent: a second call returns without recreating the bucket or the
semaphore, so the counters and the bucket's level survive. In Rust, constructing the one
client once is the equivalent.

Rust has no monkey-patching. The equivalent contract is:

- Construct exactly **one** HTTP client for the scan engine, with one `NetworkGuard` inside.
- Every source receives that client through `ScanContext`. No source may call
  `reqwest::get`, open a `TcpStream`, or use any other socket path. Plan §10.1: "The engine
  must use one HTTP client."
- The client's DNS stage calls the guard for every hostname, including each redirect hop.
  IP literals take the guard's bypass (§4.1).
- The client's request path acquires a socket slot before connecting and holds it until the
  response is finished — closed or dropped — not merely until the headers arrive (§4.7).
- Per-request headers are allowed and must not bypass the guard. `workatastartup` answers
  406 to a non-browser-shaped `Accept`; it sends its own (see `sources/workatastartup.py:19-21`
  and `:59-64`), and that request still goes through the guard.
- The guard is per process, not per source. A second client would split the cache and double
  the cap.

If the chosen client cannot take a custom resolver at its DNS stage, it is the wrong client;
§10.1 requires one. Connection pooling is welcome but must not be relied on: the guard must
behave identically with or without reuse. Python has none.

### Signatures to mirror

The Python names are internal (`_`-prefixed); visibility in Rust is yours. The arguments,
defaults and semantics are not.

```python
def _load_settings()
def install()
def stats()
def line()
def reset()                                    # tests only
def is_dns_error(exc)
def _is_ip_literal(host)
def _cached_getaddrinfo(host, port, family=0, type=0, proto=0, flags=0)
def _serve_fresh(entry)
def _note_failure_locked(exc)
def _note_success_locked()
def _gated_urlopen(*args, **kwargs)

class _TokenBucket:
    def __init__(self, rate, capacity)
    def take(self)

class _GatedResponse:
    def __init__(self, response, release)
    def _finish(self)
    def close(self)
    def __enter__(self)
    def __exit__(self, *exc_info)
    def __getattr__(self, name)
    def __del__(self)

def http_get(url, timeout=30, retries=2)       # core.py
```

A suggested Rust shape (names may follow crate convention, but the tests below call this
surface, so rename only if every test is updated):

```rust
pub trait Resolver {
    fn getaddrinfo(&self, host: &str, port: u16,
                   family: i32, sock_type: i32, proto: i32, flags: i32)
        -> Result<Vec<SocketAddr>, DnsError>;
}

pub fn new(settings: NetworkSettings,
           resolver: Arc<dyn Resolver + Send + Sync>,
           clock: Arc<dyn Clock + Send + Sync>) -> Self;

pub async fn resolve(
    &self, host: &str, port: u16,
    family: i32, sock_type: i32, proto: i32, flags: i32,
) -> Result<Vec<SocketAddr>, DnsError>;

pub fn stats(&self) -> Stats;
pub fn line(&self) -> String;
pub fn reset(&self);                           // #[cfg(test)] or doc-hidden
```

`resolve` is async because it awaits the bucket and the per-key lock. The raw
`getaddrinfo` call is blocking and must not run on the async executor's core thread: the
production `Resolver` wraps `libc::getaddrinfo` on a blocking task; the test fake records
calls. The clock exposes a monotonic `Duration` and an awaitable `sleep(duration)` so the
tests can advance time and record sleeps without wall-clock waits. The guard is shared by all
source tasks, so it must be `Send + Sync`.

## 3. Configuration

`_load_settings()` (`net.py:44-81`) resolves seven values from
`data/config/sources.yaml`'s `network:` block, then the environment, then defaults. The
current file (lines 14-28) carries exactly the default values:

```yaml
network:
  max_sockets: 32
  dns_cache_seconds: 300
  dns_nx_seconds: 15
  dns_per_second: 10
  dns_burst: 20
  dns_breaker_failures: 12
  dns_breaker_seconds: 60
```

| YAML key | Env var | Default | Clamp in Python | Consumed by |
|---|---|---|---|---|
| `max_sockets` | `JOBSCAN_MAX_SOCKETS` | `32` | `max(1, int(...))` | socket semaphore |
| `dns_cache_seconds` | `JOBSCAN_DNS_TTL` | `300.0` | `max(0.0, float(...))` | positive cache TTL |
| `dns_nx_seconds` | `JOBSCAN_DNS_NX_TTL` | `15.0` | `max(1.0, float(...))` | negative cache TTL, stale re-cache TTL |
| `dns_per_second` | `JOBSCAN_DNS_RATE` | `10.0` | `max(0.1, float(...))` | bucket refill rate |
| `dns_burst` | `JOBSCAN_DNS_BURST` | `20.0` | `max(1.0, float(...))` | bucket capacity |
| `dns_breaker_failures` | `JOBSCAN_DNS_BREAKER_FAILURES` | `12` | `max(1, int(...))` | consecutive temporary failures to trip |
| `dns_breaker_seconds` | `JOBSCAN_DNS_BREAKER_SECONDS` | `60.0` | `max(1.0, float(...))` | pause length |

Resolution rules, in order and exactly:

1. Read `<root>/data/config/sources.yaml`, take `.get("network") or {}`. Any failure to
   read, parse, or import YAML becomes `{}` — a missing config is not a reason to fail.
2. For each key: use the YAML value if it is not `None`; otherwise read the env var.
   **An invalid YAML value never falls through to the env var** — it falls through to the
   default.
3. Cast with Python `int` / `float`. `TypeError` and `ValueError` yield the default. For an
   int key, a YAML float truncates toward zero (`int(32.9) == 32`); a numeric string with an
   integer form parses (`int("16") == 16`), while `"32.9"` raises and falls back to the
   default.
4. Apply the clamp shown above.
5. Load once. `install()` reads the settings to build the bucket and semaphore; the TTLs and
   breaker thresholds are read on use.

The clamps exist so a typo cannot disable a brake. `max_sockets` can never be 0 (a
self-deadlock), `dns_nx_seconds` can never be below 1.0 (negative caching cannot be
disabled), and so on. The one deliberate exception is `dns_cache_seconds`, whose floor is
`0.0`: setting it to 0 disables the positive cache but leaves negative caching and
stale-on-error working.

In Rust, resolve the seven values once when constructing the guard. Keep the resolver pure
(input: a parsed YAML mapping plus an env lookup) so the settings tests do not mutate the
real environment. Production `data/config/sources.yaml` may be absent; code the defaults,
do not rely on the file.

## 4. Behaviours to port

### 4.1 The lookup entry point and its bypasses

```python
def _cached_getaddrinfo(host, port, family=0, type=0, proto=0, flags=0):
    if not host or flags or _is_ip_literal(host):
        return _ORIG_GETADDRINFO(host, port, family, type, proto, flags)
```

A lookup bypasses the guard entirely — no cache read or write, no token, no breaker check,
no counters — when any of these is true:

- `host` is falsy (empty string or `None`);
- `flags` is non-zero;
- `host` is an IPv4 or IPv6 literal. `_is_ip_literal(host)` (`net.py:171-180`) tries
  `socket.inet_pton(AF_INET, host)` and `socket.inet_pton(AF_INET6, host)`, returns False for
  a non-string, and catches `OSError`. In Rust, `host.parse::<IpAddr>()` is the equivalent;
  do not use a looser parser.

**Porting trap:** a non-zero `flags` silently disables the whole cache. The HTTP client
adapter must call the guard with `flags = 0`, exactly as `urllib` does. Do not pass
`AI_ADDRCONFIG` (or any other hint) through this parameter — if the production resolver
needs a hint, apply it inside the guard's resolver implementation, not at the guard's
bypass check. Otherwise every lookup misses, the bucket paces them, and the cache does
nothing.

The cache key is exactly:

```python
key = (host, port, family, type, proto)
```

`flags` is absent because a non-zero value bypasses the cache. The host is the exact string
passed in: Python does not lowercase it or IDNA-normalise it, so `Example.COM` and
`example.com` are two entries. Do not add normalisation.

Return a **copy** of the addresses (`list(addrs)` in Python), never the cache's own vector.
Return every address `getaddrinfo` produced, in the original order; the client chooses among
them.

The resolver must surface the raw `getaddrinfo` error code (`EAI_*`), because §4.3 and §4.5
classify on it. `std::net::ToSocketAddrs` may collapse it into an `io::Error` and lose the
code; use `libc::getaddrinfo` on a blocking task, or a resolver API that preserves it. A
flattened error also breaks `is_dns_error` (§4.8).

### 4.2 TTL cache and single-flight

Cache entry shape (`net.py:97`):

```python
_cache = {}               # key -> [expiry, addrs|None, errno|None]
```

- A successful lookup is stored as `[now + dns_cache_seconds, addrs, None]`
  (`net.py:289-291`). `now` is `time.monotonic()`; use a monotonic clock, never wall time.
- An entry is **fresh only when `expiry > now`** (strict). At equality it is expired.
- `_serve_fresh(entry)` increments `hits`. If the entry has addresses, it returns a copy. If
  it is a negative entry (`addrs is None`), it raises the cached error (§4.3).
- The cache has no size limit and no eager eviction; an entry is replaced on the next
  lookup of the same key. The bound is the number of distinct
  `(host, port, family, type, proto)` tuples, which is a few thousand.

**Single-flight** (`net.py:239-248`) is the other half of the cache. On a miss, the caller
takes a per-key lock from `_host_locks`; the lock is created under the global `_lock` via
`setdefault`, then acquired outside it. Inside the lock it checks freshness **again** and
serves the entry if another thread resolved it while this one waited. The comment is the
requirement:

> Without this, 32 workers that all miss on `api.smartrecruiters.com` at once issue 32
> identical queries, which is the very burst this module exists to remove.

Consequences to preserve:

- A waiter that finds the entry fresh counts as a `hit`, not a second `miss`.
- The per-key lock is held across the real resolver call and the cache write, so one name
  resolves once; different names proceed in parallel (subject to the bucket).
- Host locks are never evicted until `reset()`. That is acceptable; the key set is bounded.
  Rust may reference-count and prune, but must preserve one query per key under contention.
- In async Rust, the per-key lock must be an async mutex; never hold the state mutex across
  an `.await`. Take a copy of whatever the state mutex protects, release it, then await.

**Why:** this is the fix for the duplicate lookups that caused the block. One name, one
query per TTL, however many boards and workers ask.

### 4.3 Negative cache

On a real resolver `socket.gaierror` with no stale addresses available:

```python
_cache[key] = [time.monotonic() + _SETTINGS["dns_nx_seconds"], None,
               getattr(exc, "errno", None)]
raise
```

Only a `socket.gaierror` enters the cache, the counters, the breaker and the stale logic.
Any other exception from the resolver propagates untouched and leaves the counters alone.

The original error is re-raised. A later lookup of the same key within the negative TTL is
served from the cache (`_serve_fresh`), increments `hits`, and raises:

```python
socket.gaierror(entry[2] if entry[2] is not None else socket.EAI_AGAIN,
                "cached name-resolution failure")
```

If the stored errno is `None`, the cached error uses `EAI_AGAIN`.

The negative cache holds **both** temporary and permanent failures. `EAI_NONAME` is cached
for the same 15 seconds, and the Python test pins it (lines 108-110). A genuinely bad host
is not asked again immediately either.

The same `dns_nx_seconds` TTL is used when a stale positive answer is re-cached (§4.5 and
§4.6). The negative TTL is not used for a fresh success.

**Why:** a retry of a name the resolver just refused asks the same refused question again,
and that is what makes Pi-hole extend the block. Fifteen seconds is long enough to stop a
retry storm and short enough to recover. The floor of 1.0 means this brake cannot be
configured to zero.

### 4.4 Token-bucket pacing

```python
class _TokenBucket:
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
            time.sleep(min(wait, 1.0))
```

Exact properties:

- The bucket starts full: `tokens = capacity` (the burst allowance).
- Each take refills by elapsed time × rate, capped at `capacity`, then removes exactly one
  token if it has one.
- If it has none, it computes `wait = (1.0 - tokens) / rate` and sleeps `min(wait, 1.0)` —
  capped at one second — outside the lock. It loops and recomputes. The cap is so a long
  wait still notices a clock jump; do not sleep the whole wait in one call.
- There is **one bucket per guard**, process-wide.
- A token is taken **only immediately before a real resolver query**: after the breaker
  check, before the `misses` counter and the `getaddrinfo` call (`net.py:268-272`). Fresh
  hits, negative hits, stale answers, breaker skips and bypassed lookups consume no tokens.
- No fairness is promised. Python's lock is not FIFO; waiters race and recompute. Test the
  aggregate rate, not the order.

**Why:** it caps queries per second regardless of how many worker threads or source pools
ask. The defaults are 10.0 per second with a burst of 20.0. Pi-hole's limit is 1000 per 60
seconds (about 16.7 per second); `sources.yaml` says the values are "Kept well under
1000/60s so two passes back to back (a sweep and the scan it feeds) do not add up."

For tests, inject the clock and the sleep. Do not use the wall clock, and do not actually
sleep for the pacing test.

### 4.5 The consecutive-failure breaker

State is two values plus the counters, guarded by the same lock as the cache
(`net.py:99-100`):

```python
_fail_streak = 0
_breaker_until = 0.0
```

The temporary set (`net.py:111-120`) is built from the platform's defined constants:

```python
_TEMPORARY_GAI = tuple(
    value for value in (
        getattr(socket, "EAI_AGAIN", None),
        getattr(socket, "EAI_FAIL", None),
        getattr(socket, "EAI_TRYAGAIN", None),
        getattr(socket, "EAI_SYSTEM", None),
    ) if value is not None
)
```

`EAI_NONAME` is deliberately absent: the source comment reads "EAI_NONAME (a genuinely bad
host) is a per-host fact and must not trip the breaker." Use the named constants, never
numeric literals; include only those the platform defines.

`_note_failure_locked(exc)` (`net.py:200-214`) runs on every real resolver `gaierror`,
before the stale check:

- If `exc.errno` is **not** in `_TEMPORARY_GAI` (including `None` and `EAI_NONAME`):
  **reset `_fail_streak` to 0** and return. It does not just skip the increment; it breaks
  the streak.
- Otherwise increment the streak. When it reaches `dns_breaker_failures` (12): set the
  streak back to 0, set `_breaker_until = now + dns_breaker_seconds` (60 s), increment
  `breaker_trips`, and write this exact line to stderr:

```
[scan] net: the resolver refused 12 lookups in a row; pausing DNS for 60s so the rate limit can clear
```

The Python format string is
`"[scan] net: the resolver refused %d lookups in a row; pausing DNS for %.0fs so the rate limit can clear\n"`,
substituting the threshold and the pause seconds.

`_note_success_locked()` (`net.py:217-221`) sets both values back to 0. It runs after any
successful real query, under the same lock as the cache write (`net.py:289-291`). A late
success therefore closes a pause another thread opened; the breaker is not latched.

The pause is checked **after** single-flight and only on a cache miss (`net.py:251-253`):

```python
paused = now < _breaker_until
```

Strict: at equality the pause is over. If paused:

- increment `breaker_skips`;
- if the expired entry has addresses, increment `stale`, re-cache the old addresses for
  `dns_nx_seconds`, and return them — no query, no token;
- otherwise raise
  `socket.gaierror(socket.EAI_AGAIN, "name resolution paused: the resolver was refusing lookups")`.

Consequences to preserve:

- A **fresh** cache hit is served while the pause is open. The pause is only consulted on
  the miss path. This is intentional: the cache is the point.
- A cached **negative** entry is likewise served while paused.
- The failure that reaches the threshold opens the pause; the pause applies to subsequent
  lookups. (If that failure has a stale entry, it serves the stale address instead of
  raising, as usual.)
- The breaker is global, not per host. That is why one Workday tenant's name fails when
  the resolver is refusing everything.
- A failure that is answered stale still counts toward the streak (§4.6). Do not return
  stale before `_note_failure_locked`.
- A non-temporary error resets the streak, so a single `NONAME` between temporary failures
  prevents a trip.

**Why:** Pi-hole lengthens its block while a client keeps asking. Twelve consecutive
temporary failures is a storm; stopping for 60 seconds lets the rate window clear instead of
feeding the block.

### 4.6 Stale-on-error

On a real resolver `gaierror`, after the failure has been counted and the streak updated, if
the expired entry has addresses:

```python
with _lock:
    _counters["stale"] += 1
    _cache[key] = [time.monotonic() + _SETTINGS["dns_nx_seconds"],
                   stale[1], None]
return list(stale[1])
```

The caller gets the old addresses; the error is not raised. The entry is re-cached with the
**negative** TTL (15 s), not the positive TTL, so the stale answer is served without a query
for 15 seconds.

- Only a positive entry qualifies (`stale[1] is not None`). A negative entry has nothing to
  serve; the new error is cached and raised (§4.3).
- This applies to any `socket.gaierror`, temporary or permanent.
- The failed refresh still counted as a `miss`, a `failure`, and possibly a breaker trip.
  Stale-on-error does not hide the failure from the breaker.
- A stale answer is counted as `stale`, not `hit` or `miss`; the failed refresh already
  counted the miss.

**Why:** a name that ever resolved keeps working through a resolver wobble. This turns
"lose every board" into "lose nothing". In the Python test, `stale.example` resolves, is
force-expired, the resolver starts refusing, and the old address is still returned.

### 4.7 The process-wide socket cap

```python
_slots = threading.BoundedSemaphore(_SETTINGS["max_sockets"])   # default 32
```

`_gated_urlopen(*args, **kwargs)` (`net.py:336-348`):

1. `_slots.acquire()`;
2. call the original `urlopen`;
3. on `BaseException`, release and re-raise;
4. return `_GatedResponse(response, _slots.release)`.

`_GatedResponse` (`net.py:297-333`) holds the slot for the **life of the response**, not just
until headers. `_finish()` is idempotent:

```python
if self._released:
    return
self._released = True
try:
    self._response.close()
finally:
    self._release()
```

It implements `close()`, the context-manager pair, `__getattr__` delegation to the wrapped
response, and `__del__` calling `_finish()` with exceptions swallowed. `__exit__` returns
False, so an exception in the `with` body propagates.

Properties to preserve:

- A slot is released exactly once: on `close()`, on leaving the `with` block, or on
  collection. A double close is a no-op.
- If `urlopen` raises, the slot is released before the error propagates.
- The cap is on **responses in flight**, not on TCP sockets. Python holds it until the
  response object is finished, even if the connection closed earlier. Replicate that; it is
  more conservative than counting live sockets.
- `BoundedSemaphore` means an over-release could not inflate the cap. In Rust, ownership of
  an `OwnedSemaphorePermit` (or equivalent) inside the response wrapper gives the same
  guarantee. Do not release the permit after headers or on a background task.
- The cap is per guard, and the scan has one guard, so it is process-wide across all
  sources. Per-source pools add up in Python; this is the ceiling.

**Why:** the module comment: "a semaphore capping requests in flight, so the process cannot
open hundreds of sockets at once no matter how the sources' per-source pools add up."
`data/config/sources.yaml`: "Simultaneous outbound requests, process-wide, however the
per-source pools add up." Plan §10.1: "The engine must also limit the number of open
sockets. The default limit is 32. The limit comes from `sources.yaml`."

### 4.8 DNS-error classification and the no-retry rule

```python
def is_dns_error(exc):
    """True when an exception is a name-resolution failure, however urllib wrapped it."""
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
```

It walks `reason` first, then `__cause__`, and returns True if **any** layer is a
`socket.gaierror`. It does not look at the errno: `EAI_NONAME` is a DNS error too. The
seen-set only prevents a cycle.

`http_get` (`server/scan/core.py:56-79`):

```python
def http_get(url, timeout=30, retries=2):
    last = None
    for attempt in range(retries + 1):
        try:
            req = urllib.request.Request(url, headers={...})
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.read().decode("utf-8", "replace")
        except Exception as exc:
            last = exc
            if is_dns_error(exc) or attempt >= retries:
                break
            time.sleep(1.5 * (attempt + 1))
    sys.stderr.write(f"[scan] fetch failed: {url} ({last})\n")
    return None
```

So a DNS error gets **one attempt and no sleep** — not an immediate retry, and not a delayed
one. The call breaks out of the retry loop on the first DNS error and returns `None` after
logging. A non-DNS error gets up to `retries + 1 == 3` attempts, sleeping 1.5 s after the
first failure and 3.0 s after the second; the third failure does not sleep. The body decodes
as UTF-8 with replacement (`errors="replace"`).

The comment states the rule: "A name-resolution failure is not retried: the resolver already
refused, and asking again is what makes the resolver extend the block. Fail fast and let the
next host try." `sources/workday.py:128-131` applies the same rule in its own retry loop:

```python
# A DNS failure is the resolver refusing, not the board misbehaving. Retrying
# asks the same refused question again and feeds the resolver's block.
if is_dns_error(exc):
    break
```

**Porting note:** `sources/workatastartup.py:66-69` does not check `is_dns_error`; its
retries are absorbed by the negative cache (the resolver is not asked again), but they still
sleep and waste the budget. The Rust port should apply the break rule uniformly. This is a
safe simplification, not a change at the resolver.

In Rust, `is_dns_error` must walk the error's `source()` chain and recognise the guard's DNS
error type through the client's wrappers. If the client stringifies errors instead of
preserving them as sources, the classifier cannot work and the client is unsuitable. Return
false for a timeout and true for any DNS error, including `NONAME`. Guard against a cyclic
chain with a visited set or a small depth cap (Python uses the `id()` seen-set).

**Why:** retrying a refused lookup is the behaviour that extends Pi-hole's block. The
negative cache enforces the same rule across calls for 15 seconds; the retry rule enforces
it within a call.

### 4.9 Counters, `stats()`, `line()` and `reset()`

The counters (`net.py:102-109`) are:

```python
_counters = {
    "hits": 0,            # lookups answered from the cache (no query)
    "misses": 0,          # real queries sent to the resolver
    "stale": 0,           # answered from an expired entry after a failed refresh
    "failures": 0,        # temporary resolver failures
    "breaker_trips": 0,   # times the resolver pause opened
    "breaker_skips": 0,   # lookups refused while the pause was open
}
```

Two of those comments are incomplete; the code is the contract:

- `failures` increments on **every** real resolver `gaierror`, permanent or temporary,
  including a failure that is then answered stale. Only the breaker streak distinguishes
  temporary errors.
- `stale` increments both when an expired entry is served after a failed refresh and when
  one is served while the breaker is open. `breaker_skips` also increments in that second
  case.

Exact counter effects by path:

| Path | `hits` | `misses` | `stale` | `failures` | `breaker_trips` | `breaker_skips` |
|---|---|---|---|---|---|---|
| Fresh positive entry served | +1 | | | | | |
| Fresh negative entry served | +1 | | | | | |
| Real query succeeds | | +1 | | | | |
| Real query fails, stale served | | +1 | +1 | +1 | +1 if the streak trips | |
| Real query fails, no stale | | +1 | | +1 | +1 if the streak trips | |
| Paused, stale served | | | +1 | | | +1 |
| Paused, refused | | | | | | +1 |
| Bypass (empty host / flags / IP literal) | | | | | | |

`stats()` (`net.py:368-374`) returns a copy of the six counters plus `installed`,
`max_sockets`, and `dns_per_second`. `installed` is a Python artifact of monkey-patching;
the Rust guard may omit it or report `true`, but the six counters and the two settings are
the contract.

`line()` (`net.py:377-382`) is exact:

```python
return ("net: %d lookups, %d cached, %d stale, %d failed, %d resolver pauses "
        "(<= %d sockets, %.0f DNS/s)"
        % (s["misses"], s["hits"], s["stale"], s["failures"], s["breaker_trips"],
           s["max_sockets"], s["dns_per_second"]))
```

Order: misses, hits, stale, failures, breaker_trips, max_sockets, dns_per_second. Note that
"lookups" is `misses` (real queries) and "cached" is `hits`. `dns_per_second` is formatted
with `%.0f`. With misses 1234, hits 5678, stale 9, failures 10, breaker trips 1, sockets 32
and 10.0 DNS/s, the string is:

```
net: 1234 lookups, 5678 cached, 9 stale, 10 failed, 1 resolver pauses (<= 32 sockets, 10 DNS/s)
```

`server/scan/engine.py:140-146` prints it once at the end of a scan, prefixed with
`[scan] `, and only when `misses` or `hits` is non-zero. Reproduce that: one line per scan,
same content, same prefix.

`reset()` (`net.py:385-398`) is test-only: it clears the cache and host locks, zeroes the
streak and the pause, zeroes every counter, and refills the bucket to `capacity` with a
fresh `updated`. It does not recreate the bucket or change settings.

### 4.10 Default request headers

`http_get` sets exactly three headers (`core.py:66-72`):

| Header | Value |
|---|---|
| `User-Agent` | `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36` |
| `Accept-Language` | `en-CA,en;q=0.9` |
| `Accept` | `*/*` |

The `UA` constant is at `core.py:33-34`. The default timeout is `30` seconds, per attempt.

These are defaults, not a straitjacket: a source may override `Accept` (workatastartup must,
or it gets a 406), and Workday adds `Accept: application/json`, `Content-Type:
application/json` and a `Referer`. Overrides must not change the defaults for other
requests and must not bypass the guard.

Transport choices that are deliberately not pinned here: TLS stack, proxy handling,
compression, connection reuse, and redirect policy. If the client follows redirects, every
hop's hostname goes through the same guard.

## 5. Tests

The unit tests are the acceptance criterion. They must never touch the network: inject a
fake resolver and a fake transport. The only live test is the manual smoke in §5.4.

### 5.1 Harness

Place the parity tests where they can inspect the cache (A18) — a `#[cfg(test)]` module in
the crate, or a test-only accessor. Do not expose cache internals in the public API.

Mirror `test_net.py:40-49`: construct the guard with these test settings, which are **not**
the defaults:

| Key | Test value | Why |
|---|---|---|
| `max_sockets` | `3` | small enough for the cap test |
| `dns_cache_seconds` | `300.0` | the stale test expires it by hand/clock |
| `dns_nx_seconds` | `15.0` | as production |
| `dns_per_second` | `1_000_000.0` | "so the test does not sleep" |
| `dns_burst` | `1_000_000.0` | same |
| `dns_breaker_failures` | `3` | trips in three failures, not twelve |
| `dns_breaker_seconds` | `30.0` | shorter pause |

The fake resolver mirrors `test_net.py:55-61` and records every call:

- a host starting `bad` raises a temporary DNS error with code `EAI_AGAIN` and message
  `temporary failure in name resolution`;
- a host starting `gone` raises a permanent DNS error with code `EAI_NONAME` and message
  `no such host`;
- any other host returns one address, `127.0.0.1` on the requested port, family `AF_INET`,
  socket type `SOCK_STREAM`, protocol `6` (the Python tuple's `canonname` is `""`; Rust has
  no equivalent and the tests compare the address only).

Helpers: `repeat(host, n)` returns n results; `raises(host)` returns the error code. A fresh
guard per group is cleaner than `reset()`; where state is shared, call `reset()` at the same
boundaries as the Python test (lines 85, 98, 106, 113, 128).

The clock is injectable and manual. The guard must use it for cache expiry, the breaker and
the bucket. Python uses `time.monotonic()` and the test force-expires an entry by writing
`time.monotonic() - 1` into it (lines 117-118); the Rust equivalent is advancing the clock.
The pacing test also needs a fake sleeper so it can assert waits without spending wall time.

### 5.2 Part A — the 34 parity tests

One per Python `check()`, in file order. The labels are quoted from `test_net.py`.

| # | Input | Expected | Mirrors (line) |
|---|---|---|---|
| A1 | Fresh guard; `resolve("api.smartrecruiters.com", 443)` | `[127.0.0.1:443]`; resolver called once | "a fresh name is a query" (86) |
| A2 | Same lookup 5 more times | 5 × `[127.0.0.1:443]`; no new resolver calls | "the same name is a cache hit" (87) |
| A3 | Inspect the resolver's call log | `["api.smartrecruiters.com"]` | "...and only one query left the process" (88) |
| A4 | `resolve("boards-api.greenhouse.io", 443)` | `[127.0.0.1:443]` | "a different name is a new query" (89) |
| A5 | Inspect the log | the two hostnames, in order | "two names, two queries" (90) |
| A6 | `stats().hits` after A2 | `5` | "hits are counted" (91) |
| A7 | `resolve("api.smartrecruiters.com", 8443)` | `[127.0.0.1:8443]`; a new resolver call (port is in the key) | "the port is part of the key" (92-94) |
| A8 | `resolve("127.0.0.1", 443)` | `[127.0.0.1:443]`; the resolver is called directly; `hits` unchanged | "an IP literal skips the cache" (95) |
| A9 | `raises("bad-a.example")` | `EAI_AGAIN` | "a temporary failure surfaces" (100) |
| A10 | Same again, immediately | `EAI_AGAIN`; no new resolver call | "the retry is answered from the negative cache" (101-102) |
| A11 | Count calls for `bad-a.example` | `1` | "...so the resolver was asked once" (103) |
| A12 | `raises("gone.example")` | `EAI_NONAME` | "a permanent failure surfaces" (108) |
| A13 | Same again | `EAI_NONAME`; no new call | "a permanent failure is also cached" (109) |
| A14 | Count calls for `gone.example` | `1` | "...and asked once" (110) |
| A15 | `resolve("stale.example", 443)` | `[127.0.0.1:443]` | "a good name resolves" (115) |
| A16 | Advance 300 s (expire it); resolver raises `EAI_AGAIN`; resolve again | `[127.0.0.1:443]`, no error | "an expired name is served stale when the resolver refuses" (121-122) |
| A17 | `stats().stale` | `1` | "the stale answer is counted" (123) |
| A18 | Inspect the cache entry | addresses `[127.0.0.1:443]`, errno `None`, expiry `now + 15 s` | "the stale answer is reused, not re-queried" (124-125) |
| A19 | Fresh guard; `raises("bad-1.example")` | `EAI_AGAIN` | "failure one" (131) |
| A20 | `raises("bad-2.example")` | `EAI_AGAIN` | "failure two" (132) |
| A21 | `raises("bad-3.example")` | `EAI_AGAIN`; this is the third consecutive temporary failure and trips the pause | "failure three trips the pause" (133) |
| A22 | `stats().breaker_trips` | `1` | "the pause opened" (134) |
| A23 | `raises("new.example")` | `EAI_AGAIN`; message `name resolution paused: the resolver was refusing lookups` | "a new name is refused without asking" (136) |
| A24 | Resolver call count vs. before A23 | unchanged | "...and the resolver was not asked" (137) |
| A25 | `stats().breaker_skips` | `1` | "the refusal is counted as a skip" (138) |
| A26 | `is_dns_error(DnsError(EAI_AGAIN, "x"))` | `true` | "a bare gaierror is a DNS error" (141) |
| A27 | `is_dns_error(Transport(DnsError(EAI_AGAIN, "x")))` | `true` | "urllib's wrapper is a DNS error" (142-143) |
| A28 | `is_dns_error(Transport(Timeout))` | `false` | "a timeout is not a DNS error" (144-145) |
| A29 | `is_dns_error(Other("boom"))` | `false` | "a plain exception is not a DNS error" (146) |
| A30 | Wrap a fake response and read through the wrapper | the response is reachable and readable (Python checks `hasattr(resp, "close")`) | "the wrapper delegates reads" (163) |
| A31 | Leave the wrapper's scope | release count `1` | "the slot is released on exit" (164) |
| A32 | Close again | release count still `1` | "releasing twice does not double-free" (166) |
| A33 | Inspect the fake response's close count | `1` | "the underlying response was closed once" (167) |
| A34 | Create a wrapper and drop it unclosed | release count `1` | "an unclosed response releases on collection" (174) |

A30-A34 use `_GatedResponse` directly with a fake response and a release callback, exactly as
the Python test does; they do not need the transport. In Rust, implement `close(&mut self)`
with an idempotent flag (mirroring `_released`) plus `Drop`, so A32 can call `close()` twice.

### 5.3 Part B — behaviour the Python test does not pin

These are required by `docs/rust-app-implementation-plan.md` S3 ("unit tests for cache hits,
negative caching, pacing, breaker open and close, and the socket cap") and by §10.1. The last
two columns name the Python source lines and the Python check each item mirrors; where the
Python suite has no equivalent, the mirror is marked `none` with the reason, and the item is
still a required test.

| # | Input | Expected | Source | Python mirror |
|---|---|---|---|---|
| B1 | At t=0 resolve `ttl.example`; at t=299.999 s resolve; at t=300.000 s resolve | 1 query at t=0, 0 at 299.999 s, a 2nd query at 300.000 s (strict `>` makes equality expired); 2 total | `net.py:236, 290` | `none`; extends the manual expiry setup at `test_net.py:117-118` |
| B2 | At t=0 `bad-a`; at t=14.999 s; at t=15.000 s | error all three; 1 query, then a cached error, then a 2nd query | `net.py:236, 286` | `none`; extends A10-A11 to the TTL boundary |
| B3 | Trip at t=0 with three temporary failures; at t=29.999 s resolve a new name; at t=30.000 s resolve a good name; then two temporary failures; then a third | refused at 29.999 s; a query and success at 30.000 s; `breaker_trips` stays 1; rises to 2 only on the third later failure | `net.py:217-221, 251-253, 289-291` | `none`; extends A19-A25 from open to close (plan S3) |
| B4 | Cache `ok.example`; trip the breaker; resolve `ok.example` | address, `hits` +1, no query | `net.py:236-237, 251-253` | `none`; A23 only covers a new name while paused |
| B5 | `bad-1`, `bad-2`, `gone`, `bad-3` | all four surface; `failures == 4`; `breaker_trips == 0` (the `NONAME` reset the streak) | `net.py:200-205` | `none` |
| B6 | `stale.example` resolves; expire it; resolver always fails; resolve at t=0, t=15 s, t=30 s | stale address each time; `failures == 3`, `stale == 3`, `breaker_trips == 1` | `net.py:273-284` | `none`; extends A16-A18 |
| B7 | Rate 10/s, burst 2, 12 distinct names at t=0 | first 2 no wait; each of the next 10 requests ~100 ms; total ~1.0 s ± 10 ms | `net.py:125-148` | `none`; the oracle sets `1_000_000.0` at `test_net.py:44` to avoid pacing |
| B8 | `max_sockets = 3`; hold 3 responses; start a 4th; close one | the 4th is pending; it completes after one close | `net.py:342-348, 360` | `none`; A31-A34 pin release, not acquire (plan S3) |
| B9 | `max_sockets = 1`; a request errors; start a second | the second completes (the permit was released on the error path) | `net.py:344-347` | `none` |
| B10 | A three-level wrapper around a DNS error; a `NONAME` error | `true`; `true` (classification is by type, not errno) | `net.py:153-168` | extends A27 |
| B11 | GET with a DNS error every time | 1 attempt; no sleeps; `None`; one `[scan] fetch failed` line | `core.py:64-79`; §10.1 | `none`; §10.1 |
| B12 | GET with a timeout every time | 3 attempts; sleeps `[1.5 s, 3.0 s]`; `None` | `core.py:64-79` | `none` |
| B13 | GET: attempt 0 times out, attempt 1 is a DNS error | 2 attempts; sleeps `[1.5 s]`; `None` | `core.py:75-76` | `none` |
| B14 | The settings cases below | exact defaults, clamps and precedence | `net.py:44-81` | `none` |
| B15 | Counters 1234 / 5678 / 9 / 10 / 1, sockets 32, rate 10.0 | the exact string in §4.9 | `net.py:368-382` | `none` |
| B16 | 8 concurrent resolves of one key | 1 resolver call; 7 `hits`; 1 `miss` | `net.py:239-248` | `none`; A2-A3 are sequential, not concurrent |
| B17 | IPv6 literal; `flags = AI_NUMERICHOST`; empty host | each goes straight to the resolver; no counters | `net.py:230-231, 171-180` | extends A8 |
| B18 | Same host/port with `family = AF_INET` then `AF_INET6`; `Example.COM` then `example.com` | 2 queries for the family pair; 2 queries for the case pair (no normalisation) | `net.py:233` | extends A7 |
| B19 | Populate the cache, counters and a tripped breaker; `reset()` | cache empty, counters zero, breaker closed, bucket full | `net.py:385-398` | `none` |
| B20 | Build a default GET | exactly the three headers of §4.10 with those values; timeout 30 s | `core.py:33-34, 66-72` | `none` |

B7's tolerance is deliberate: the refill is floating-point, and a recomputation can request
a sub-millisecond follow-up nap. Assert the aggregate wait, not the exact sleep count. Do
not assert fairness.

B8/B9 need an injectable transport that returns a controllable response object. Hold the
three responses alive to hold the three permits; closing one must wake the 4th. Use a
oneshot/`Notify` and `poll!`/`timeout`, not a real sleep.

B14 cases:

| YAML `network:` | Env | Expected |
|---|---|---|
| `{max_sockets: 0}` | — | `1` |
| `{max_sockets: -5}` | — | `1` |
| `{max_sockets: 32.9}` | — | `32` (int truncates toward zero) |
| `{max_sockets: "16"}` | — | `16` |
| `{max_sockets: "abc"}` | `JOBSCAN_MAX_SOCKETS=8` | `32` (default; a present-but-invalid YAML value does not fall through to env) |
| `{}` | `JOBSCAN_MAX_SOCKETS=8` | `8` |
| `{max_sockets: 16}` | `JOBSCAN_MAX_SOCKETS=8` | `16` (YAML wins) |
| `{dns_per_second: 0}` | — | `0.1` |
| `{dns_per_second: -2}` | — | `0.1` |
| `{dns_cache_seconds: -1}` | — | `0.0` |
| `{dns_nx_seconds: 0}` | — | `1.0` |
| `{dns_burst: 0}` | — | `1.0` |
| `{dns_breaker_failures: 0}` | — | `1` |
| `{dns_breaker_seconds: 0}` | — | `1.0` |
| absent block | — | `32`, `300.0`, `15.0`, `10.0`, `20.0`, `12`, `60.0` |
| `{}` | `JOBSCAN_DNS_RATE=25` | `25.0` |
| `{}` | `JOBSCAN_DNS_RATE=""` | `10.0` (invalid cast → default) |
| `{dns_per_second: null}` | `JOBSCAN_DNS_RATE=25` | `25.0` (null is treated as absent) |

### 5.4 Live smoke (manual, not `cargo test`)

Plan S3 acceptance: "A live smoke test fetches one URL from each host family." Use the URLs
in `docs/tasks/S4a-source-fixtures.md` (or the saved fixtures) — at least Greenhouse, Ashby,
Lever, Workable, SmartRecruiters, Rippling, BambooHR, Recruitee, Breezy, Teamtailor, one
Workday tenant, startups.gallery, and workatastartup with its browser-shaped `Accept`.

Run the list once, print `line()`, then run it again and confirm `hits` rose and `misses`
did not rise as much: that is the cache working. Confirm `breaker_trips == 0`. Record the
line and the socket ceiling. Do not run this in CI or on every build.

## 6. Acceptance

1. `cargo test -p jh-scan` is green, including A1-A34 and B1-B20.
2. No unit test opens a socket or touches DNS; the fake resolver's call log is the only
   evidence of a query.
3. `cargo clippy -- -D warnings` is clean and `cargo fmt` has been run.
4. The live smoke in §5.4 passes: one URL per host family, `breaker_trips == 0`, and a
   second pass shows cache reuse.
5. The scan's end-of-run log carries exactly one `[scan] net: ...` line in the Python
   format, and no source constructs its own client or socket.
6. No deviation from §4 is undocumented. If an exact port is impossible (for example, a
   client that cannot preserve the `EAI_*` code), stop and report it rather than guessing.

## 7. Report

When you finish, print:

- What landed, per file.
- The settings resolution and any deviation from §3.
- The test count and the commands you ran.
- The live smoke line per host family, and the observed hits/misses ratio on the second
  pass.
- Any Python behaviour you could not port exactly, and why. The expected answer is none.

## 8. Rules

- Work in this directory only. Do not modify `server/`, `client/` or `data/`.
- One commit: `feat: Add the network guard`. Never push.
- Do not add a dependency that opens sockets outside the one client.
- Do not rename or reformat Python files.
- If a Python behaviour is unclear, read `net.py` again. Do not guess.

### Common parity traps

- Passing `AI_ADDRCONFIG` (or any non-zero `flags`) into the guard: silently disables the
  cache (§4.1).
- Counting only temporary errors in `failures`: the code counts every real `gaierror`
  (§4.9).
- Returning stale before `_note_failure_locked`: the failure must still hit the streak
  (§4.5, §4.6).
- Resetting the streak only on success: a `NONAME` also resets it (§4.5).
- Releasing the socket permit after headers: it is held for the response's life (§4.7).
- Retrying a DNS error in any loop: one attempt, no sleep (§4.8).
- Using wall time: cache, bucket and breaker all use monotonic time (§4.2, §4.4, §4.5).
- Normalising the host before keying: Python does not (§4.1).
- Losing the `EAI_*` code in a `ToSocketAddrs` wrapper: the breaker and negative cache need
  it (§4.1, §4.8).
- Holding the state mutex across an `.await`: copy out, release, then await (§4.2).
