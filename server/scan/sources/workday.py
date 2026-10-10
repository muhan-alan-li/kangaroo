"""Workday boards, read through the board API. The largest source, and the
one that throttles by IP, so its worker pool is fixed deliberately."""

from __future__ import annotations

from concurrent import futures
from datetime import datetime, timezone
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

from .. import boardstate
from ..core import UA, WORKDAY_ROLE_WORDS, is_dns_error
from ..filters import region_signals
from ..posting import make_posting


# Workday's `locationsText` is free-form and frequently names no country at all
# ("Greater Chicago Area", "New York Remote Work, More...", "12 Locations"), so this is a
# filter for the obviously foreign rather than an assertion of North-America-ness. See
# workday_region_ok below for why ambiguity is kept rather than dropped.
FOREIGN_PLACES = re.compile(
    r"\b(australia|austria|bangalore|bengaluru|belgium|bolivia|brazil|bulgaria|cambodia|"
    r"chile|china|colombia|costa rica|croatia|czech|denmark|dominican republic|ecuador|"
    r"egypt|el salvador|estonia|finland|france|germany|greece|guatemala|honduras|hong kong|"
    r"hungary|iceland|india|indonesia|ireland|israel|italy|japan|kazakhstan|kenya|korea|"
    r"latvia|lithuania|luxembourg|malaysia|mexico|morocco|netherlands|new zealand|"
    r"nicaragua|nigeria|norway|pakistan|panama|paraguay|peru|philippines|poland|portugal|"
    r"qatar|romania|russia|saudi|singapore|slovakia|slovenia|south africa|spain|sweden|"
    r"switzerland|taiwan|thailand|turkey|ukraine|united arab emirates|united kingdom|"
    r"uruguay|venezuela|vietnam|england|scotland|wales|uk|edinburgh|glasgow|belfast|cardiff|"
    r"london|dublin|munich|berlin|paris|amsterdam|zurich|geneva|barcelona|madrid|milan|"
    r"rome|warsaw|krakow|prague|budapest|bucharest|sofia|belgrade|zagreb|"
    r"hyderabad|mumbai|pune|chennai|gurgaon|noida|kolkata|ahmedabad|kochi|"
    r"tokyo|osaka|seoul|shanghai|beijing|shenzhen|guangzhou|taipei|"
    r"tel aviv|haifa|sydney|melbourne|brisbane|perth|adelaide|auckland|wellington|"
    r"kuala lumpur|jakarta|manila|bangkok|ho chi minh|hanoi|"
    r"dubai|abu dhabi|istanbul|ankara|cairo|nairobi|lagos|johannesburg|cape town|"
    r"sao paulo|mexico city|guadalajara|monterrey|bogota|lima|santiago|buenos aires|"
    r"san jose, costa rica|gurugram|bundesweit|crawley|maidenhead|derrimut)\b", re.I)


# Workday also writes bare three-letter ISO codes next to a city ("IND - Coimbatore (708)",
# "Maidenhead (GBR)"), which name no country in English. US and Canadian codes are absent on
# purpose. Ambiguous ones are left out too: "BEL" would drop Bel Air, Maryland.
FOREIGN_CODES = re.compile(
    r"\b(ind|gbr|aus|deu|fra|sgp|jpn|nld|esp|ita|pol|isr|zaf|phl|mys|tha|vnm|idn|twn|kor|"
    r"tur|egy|nga|ken|mex|bra|chn|hkg|irl|nzl|dnk|swe|nor|fin|aut|cze|hun|rou|grc|prt|"
    r"svk|svn|hrv|bgr|ltu|lva|est|srb|ukr)\b", re.I)


def workday_region_ok(location):
    """True when a Workday location is Canada or the US, or is genuinely ambiguous.

    A false drop loses a real posting silently, which is worse than a false keep that the
    location tiers discard one step later: those tiers are stricter and know the
    candidate's actual targets. So a location carrying any Canadian or American signal is
    kept, a location naming no country at all is kept, and only a location that names a
    foreign country or city is dropped.
    """
    if not location or not location.strip():
        return True                 # nothing to judge; the tiers decide
    ca, us = region_signals(location)
    if ca or us:
        return True
    if FOREIGN_PLACES.search(location):
        return False
    return not FOREIGN_CODES.search(location)


def parse_workday_age(text, posted=None):
    """'Posted Today' / 'Posted 3 Days Ago' -> days, else None.

    Workday publishes a relative string in its board feed rather than a date. It is a
    publish field, not an update field, so it is the right one to age a posting by. It is
    coarse -- there is no hour granularity -- and anything unrecognised returns None so an
    unknown age is never silently treated as fresh.
    """
    if posted:
        return max(0, (datetime.now(timezone.utc).date() - posted).days)
    t = (text or "").strip().lower()
    if not t:
        return None
    if t.endswith("today"):
        return 0
    if t.endswith("yesterday"):
        return 1
    m = re.search(r"(\d+)\s*\+?\s*day", t)
    if m:
        return int(m.group(1))
    m = re.search(r"(\d+)\s*\+?\s*month", t)
    if m:
        return int(m.group(1)) * 30
    return None


def _workday_page(board, page_size, offset, timeout, attempts=3, search_text=""):
    """One page of one board, retried: Workday throttles by IP under load.

    Without the retry every transient refusal silently loses a board for the whole run --
    a run at 16 workers during US business hours was dropping about 150 of 1,328 boards,
    against about 50 at 8 workers off-peak. The retry is what makes the wider pool safe,
    and a board that still fails is reported rather than passed over in silence.
    """
    tenant, wd, site = board["tenant"], board["wd"], board["site"]
    url = f"https://{tenant}.{wd}.myworkdayjobs.com/wday/cxs/{tenant}/{site}/jobs"
    body = json.dumps({"appliedFacets": {}, "limit": page_size, "offset": offset,
                       "searchText": search_text}).encode("utf-8")
    req = urllib.request.Request(url, data=body, headers={
        "User-Agent": UA,
        "Accept": "application/json",
        "Content-Type": "application/json",
        "Referer": f"https://{tenant}.{wd}.myworkdayjobs.com/{site}",
    })
    last = None
    for attempt in range(max(1, attempts)):
        if attempt:
            time.sleep(0.6 * attempt)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return json.loads(resp.read().decode("utf-8", "replace"))
        except Exception as exc:  # noqa: BLE001 -- retried, then reported by the caller
            last = exc
            # A DNS failure is the resolver refusing, not the board misbehaving. Retrying
            # asks the same refused question again and feeds the resolver's block.
            if is_dns_error(exc):
                break
    raise last


def page_signature(posts):
    """The identity of one keyword page: every posting's requisition, in order.

    Not just the first id and the count -- the whole ordered list. A page that reorders, or
    that swaps one posting for another, is not the same page, and treating it as unchanged is
    how a cursor starts losing postings. The list is what gets stored in state/board_health.json
    per board, so it is kept to the identity only.
    """
    ids = []
    for post in posts or []:
        path = (post or {}).get("externalPath") or ""
        ids.append(path.rstrip("/").rsplit("/", 1)[-1] or (post or {}).get("title", ""))
    return ids


def cursor_allows_skip(cursor, signature, verified_age_days, max_age_days):
    """Whether page one being unchanged proves page two holds nothing new.

    It proves it only if page two was actually read recently. The keyword feed is ordered by
    relevance, so an old posting no longer means everything after it is older, and a posting
    that lands past position 20 without displacing anything leaves page one identical. So the
    cursor is bounded by `max_age_days`: past that, the second page is read again whether or
    not page one moved. Coverage of page two is never older than that bound, and the bound is
    inside the posting window.
    """
    if not cursor or not signature:
        return False
    if list(cursor.get("ids") or []) != list(signature):
        return False
    if cursor.get("count") != len(signature):
        return False
    if verified_age_days is None or verified_age_days > max_age_days:
        return False
    return True


def _workday_board(board, page_size, max_pages, window, timeout, role_re=None,
                   search_text="engineer", err_sink=None, cursor=None, stat_sink=None):
    """Recent postings from one board. The role and region filters run here, not downstream,
    so a board's non-engineering and overseas reqs never become postings at all.

    `searchText` narrows the result set server-side, and that turned out to matter far more
    than the request it saves. Paging the plain listing takes the newest N postings, and on a
    busy board N is not seven days -- CVS alone reports 19,392 open reqs, so forty postings
    can be a day and a half. Asking for the board's engineering postings instead samples the
    whole board. Measured over 40 boards, within-window Canada/US engineering hits:

        2 plain pages    110      2 keyword pages  161      1 keyword page  100

    So the same two requests reach half again as much. The cost is that a keyword feed is
    ordered by relevance, not date, so an old posting no longer proves everything after it
    is older -- age is a filter here, not a reason to stop reading the board.
    """
    rows, offset, dropped = [], 0, 0
    pages_read = 0
    signature = []
    for page_index in range(max(1, max_pages)):
        try:
            data = _workday_page(board, page_size, offset, timeout, search_text=search_text)
        except Exception as exc:  # noqa: BLE001 -- a dead board must not kill the whole scan
            # The board's own failure is reported by the caller, which is where the count
            # and the per-board state are kept: a board that does not answer is a different
            # thing from a board that answers with nothing, and boardstate.json holds both.
            if err_sink is not None:
                err_sink.append(f"{type(exc).__name__}: {exc}")
            return rows, False, dropped
        posts = data.get("jobPostings") or []
        if not posts:
            if stat_sink is not None:
                stat_sink.update({"signature": signature, "pages": pages_read, "unchanged": False})
            return rows, True, dropped
        pages_read += 1
        if page_index == 0:
            signature = page_signature(posts)
        for post in posts:
            age = parse_workday_age(post.get("postedOn"))
            if age is not None and age > window:
                continue
            if role_re is not None and not role_re.search(post.get("title") or ""):
                dropped += 1
                continue
            if not workday_region_ok(post.get("locationsText") or ""):
                dropped += 1
                continue
            rows.append((post, age))
        # `cursor and` must come first: Python evaluates every argument before the call, so
        # cursor.get() below would raise AttributeError on a board with no cursor yet -- which
        # is every board until a sweep has run once. That is a total Workday outage, and the
        # broad handler in _probe_board reports it as 1,272 boards "not answering" rather than
        # as one crash, so it looks like a throttle.
        if page_index == 0 and cursor and cursor_allows_skip(
                cursor, signature, cursor.get("age_days"), cursor.get("max_age_days")):
            # Page one is byte-for-byte what it was when page two was last read, and that read
            # is inside the bound, so page two is not requested at all. The caller swaps this
            # board's postings for the previous sweep's, which are the same postings.
            if stat_sink is not None:
                stat_sink.update({"signature": signature, "pages": 1, "unchanged": True})
            return rows, True, dropped
        if len(posts) < page_size:
            if stat_sink is not None:
                stat_sink.update({"signature": signature, "pages": pages_read, "unchanged": False})
            return rows, True, dropped
        offset += page_size
    if stat_sink is not None:
        stat_sink.update({"signature": signature, "pages": pages_read, "unchanged": False})
    return rows, True, dropped


def _probe_board(board, page_size, max_pages, window, timeout, role_re, search_text,
                 cursor=None):
    """One board's fetch, with its failure and its page signature captured.

    `_workday_board` answers (rows, ok, dropped) and has to swallow the exception, because a
    dead board must never kill a sweep. The retry pass needs the reason, not just the flag, and
    the cursor needs to know how many pages were actually read, so both travel out with the
    result instead of being inferred by the caller.
    """
    errors = []
    stat = {}
    try:
        rows, ok, dropped = _workday_board(board, page_size, max_pages, window, timeout,
                                           role_re, search_text, err_sink=errors,
                                           cursor=cursor, stat_sink=stat)
    except Exception as exc:  # noqa: BLE001
        rows, ok, dropped = [], False, 0
        errors.append(f"{type(exc).__name__}: {exc}")
    return rows, ok, dropped, (errors[0] if errors else None), stat


def fetch_workday_boards(boards, cfg, cursors=None):
    """Read every Workday board in parallel, recent postings only.

    Two pages per board, not a crawl of every board. Paging each one to the end would cost
    hundreds of thousands of requests a day to reach postings that are months old, and CVS
    alone reports 19,392 open reqs.

    The pages are keyword-narrowed (`searchText`) rather than plain. Both cost the same two
    requests, but a plain page returns the board's newest postings whatever they are, and on
    a busy board that is well under a week -- so the window was never really being covered.
    Measured over 40 boards, within-window Canada/US engineering hits went 110 -> 161.

    The pool is what makes this fit a daily run: a thousand-odd boards answered
    sequentially is minutes of nothing but waiting.
    """
    opts = cfg.get("workday") or {}
    page_size = int(opts.get("page_size") or 20)
    max_pages = int(opts.get("max_pages") or 2)
    workers = max(1, int(opts.get("workers") or 8))
    window = int(opts.get("window_days") or 14)
    timeout = int(opts.get("timeout") or 25)
    search_text = str(opts.get("search_text", "engineer"))

    words = tuple(opts.get("title_words") or WORKDAY_ROLE_WORDS)
    role_re = re.compile(r"\b(" + "|".join(re.escape(w) for w in words) + r")\b", re.I)

    out, dead, filtered = [], 0, 0
    cursors = cursors or {}
    with futures.ThreadPoolExecutor(max_workers=workers) as pool:
        pending = {pool.submit(_probe_board, b, page_size, max_pages, window, timeout,
                               role_re, search_text,
                               cursors.get(f"{b.get('tenant')}|{b.get('wd')}|{b.get('site')}")): b
                   for b in boards}
        for future in futures.as_completed(pending):
            board = pending[future]
            try:
                rows, ok, skipped, err, stat = future.result()
            except Exception as exc:  # noqa: BLE001
                rows, ok, skipped, err, stat = [], False, 0, f"{type(exc).__name__}: {exc}", {}
            filtered += skipped
            # Every board's outcome is recorded, including the ones that answered with
            # nothing: "did not answer" and "answered with no recent engineering roles" are
            # different facts, and only the first is worth retrying. The page signature rides
            # along, so the next sweep can skip a board's second page when it has not moved.
            boardstate.note(board, ok, hits=len(rows), err=err, cursor=stat or None)
            if not ok:
                dead += 1
                continue
            tenant, wd, site = board["tenant"], board["wd"], board["site"]
            company = (board.get("name") or tenant).strip()
            for post, age in rows:
                path = post.get("externalPath") or ""
                if not path:
                    continue
                # The last segment is the requisition identity, and it is what the
                # APPLY_SIGNATURES workday entry extracts from a feed's own link, so a
                # posting found by both sources collapses to one report.
                last = path.rstrip("/").rsplit("/", 1)[-1]
                out.append(make_posting(
                    source="ats",
                    company=company,
                    title=post.get("title", ""),
                    location=post.get("locationsText", ""),
                    url=f"https://{tenant}.{wd}.myworkdayjobs.com/en-US/{site}{path}",
                    age_days=age,
                    ats="workday",
                    raw_id=last or None,
                    dedupe_key=f"ats|{last}" if last else None,
                    extra={"board": f"{tenant}|{wd}|{site}",
                           "time_type": post.get("timeType"),
                           "posted_text": post.get("postedOn")},
                ))
    if dead:
        sys.stderr.write(f"[scan] workday: {dead} of {len(boards)} boards did not answer\n")
    sys.stderr.write(
        f"[scan] workday: {len(out)} recent engineering postings in Canada/US "
        f"({filtered} dropped by the role/region filter)\n")
    return out
