# Source register

Every job source this workspace has considered, and what happened to it. The standing
instruction from the user (2026-09-30) is: **always be on the lookout for new sources, and
incorporate each one long term** — which means adding it to `config/sources.yaml` *and*
documenting it here, not just using it once.

A source is not incorporated until all four are true:

1. It is reachable from this machine, verified by an actual request (not assumed).
2. It has an entry in `config/sources.yaml`, with `enabled: true` and a comment saying what
   it is and why it is trusted.
3. If it revealed employer boards, those slugs have been probed by `server/jobs/probe_ats.py` and
   folded into `config/ats_boards.yaml`. **A feed that hands over boards is only half-used
   until this step runs.**
4. It is written up in the `job-application-run` skill's `references/scanning-and-tiers.md`,
   and `AGENTS.md` still describes the source list correctly.

## Adopted

| id | added | what it is |
| --- | --- | --- |
| `jobbank` | 2026-09-23 | Job Bank Canada. Official, local, carries a real posted date. |
| `newgrad_list` | 2026-09-23 | Simplify new-grad feed, read as HTML from the repo README. |
| `ats_boards` | 2026-09-23 | Board APIs: Greenhouse/Ashby/Lever slugs (164 as of 2026-10-05; `server/jobs/probe_ats.py` grows it and the scan prints a line when a harvested board is missing), 1,328 Workday boards, 513 JSON-vendor boards, 247 iCIMS hosts. Strongest source. |
| `linkedin` | 2026-09-23 | LinkedIn public guest search. Read-only, never signed in. |
| `startups_gallery` | 2026-09-28 | startups.gallery. ~20 newest early-stage postings, and the boards behind them. |
| `newgrad_json` | 2026-09-30 | The Simplify repo's own `listings.json`, which its README is generated from. |
| `workatastartup` | 2026-10-04 | Work at a Startup (Y Combinator). The anonymous feed, read out of the page's Inertia payload. 285 distinct roles per sweep, and no publish dates at all. |

## Rejected, with the reason

- `indeed` — 403 from this machine. Use the browser by hand.
- `eluta.ca` — CAPTCHA wall.
- `careerjet.ca` — JavaScript only, no server-rendered content.
- `talent.com`, `ziprecruiter` — 403.
- `swelist.com` — **verified redundant, 2026-09-30.** Not a scraper and not a second source:
  its Next.js page fetches `SimplifyJobs/New-Grad-Positions/dev/.github/scripts/listings.json`
  client-side and renders it. Its own counter ("Found 3056 new-grad tech jobs") equals that
  feed's `active and is_visible` count exactly. Reading the JSON directly gives the same data
  without the middle layer, so this is covered by `newgrad_json`.
- `SimplifyJobs/Summer2026-Internships` — same JSON shape (4,428 active), but it is
  internships. Not applicable to a 2026 graduate. Revisit only if that changes.

## Vendor endpoints, verified 2026-09-30

Probed from this machine with real slugs taken out of the Simplify feed, not guesses. The URL
count is how many of that feed's 19,676 links point at the vendor — the size of the prize, not
a count of live openings.

### Scannable — verified with a real response

| vendor | endpoint | verified |
| --- | --- | --- |
| Workday | `POST https://{tenant}.{wd}.myworkdayjobs.com/wday/cxs/{tenant}/{site}/jobs` | RTX 4,866 · HPE 1,542 · HP 1,249 · Duke Energy 101 |
| SmartRecruiters | `https://api.smartrecruiters.com/v1/companies/{slug}/postings?limit=100&offset=N` | ServiceNow 708 · AECOM2 5,263 |
| iCIMS | `https://{host}.icims.com/jobs/search?ss=1&in_iframe=1` | **ADOPTED** — needs browser TLS impersonation; see below |
| Workable | `https://apply.workable.com/api/v1/widget/accounts/{slug}?details=true` | thorlabs 85 · scitec 40 · grayce 35 |
| Rippling | `https://ats.rippling.com/api/v1/board/{slug}/jobs` | momentumcareers 110 · positron 30 |
| BambooHR | `https://{company}.bamboohr.com/careers/list` | tricrobotics · fispan |
| Recruitee | `https://{company}.recruitee.com/api/offers/` | 1x · hardrockdigital |
| Breezy HR | `https://{company}.breezy.hr/json` | vantage-point-solutions-inc 18 |
| Teamtailor | `https://{company}.teamtailor.com/jobs.json` | noda-1720000970 |

Notes that matter when implementing any of these:

- **Workday is 46% of the whole feed** — 8,980 of 19,676 links. The body is
  `{"appliedFacets":{},"limit":20,"offset":0,"searchText":""}`; the response carries `total`
  and `jobPostings[]`. It needs a **tenant + wd-number + site triple per company**, and all
  three are harvestable from the feed's own URLs. Paginate on `offset`. Workday also answers
  with `startDate` per posting, which is the real publish date.
- **iCIMS looks dead and is not.** The outer `/jobs/search` page is a JavaScript shell with
  zero job data in it, which is why it reads as unscannable. The page's own iframe URL —
  the same one with `&in_iframe=1` appended — server-renders the table. `&mobile=true` works
  too. Parse carefully: the title cell carries a leading column label.
- **SmartRecruiters returns `totalFound: 0` for some slugs** that clearly have jobs, so treat
  zero as "check the slug casing" rather than "no jobs".
- **Breezy returns an empty array for a valid but empty board.** Empty is not broken.
- **Rippling slugs often carry a `-careers` suffix** (`momentumcareers`, `camlin-careers`,
  `riot-platforms-careers`), so do not strip it.

### Adopted 2026-09-30: Workday

Workday is the first of the vendors above to be wired in, because it is worth more than all
the others combined. It lives in `config/workday_boards.yaml`, read through
`companies_files` in `sources.yaml`, and is fetched by `src_ats` as one parallel batch.

- **The board list is harvested, not hand-written.** `server/jobs/harvest/workday.py` pulls the
  tenant + wd number + career site out of every Workday link in the Simplify feed, which is
  how the list reached 1,328 boards. Every entry is a company that has already posted a
  new-grad role, so the list is self-selecting.
- **One page per board, two only when needed.** The board API is date-sorted newest first
  and `postedOn` is a real publish field, so the first page holds everything recent. CVS
  alone reports 19,392 open reqs, and paging boards like that would cost hundreds of
  thousands of requests a day to reach postings that are months old. A second page is
  fetched only when the first was entirely inside the window.
- **The user's basic filter runs inside the source**: engineering roles, Canada or the
  United States, posted within 7 days. Measured on a 2,926-posting sample, 8% survive. The
  region test (`workday_region_ok`) drops only what clearly names a foreign place and keeps
  anything ambiguous, because the location tiers downstream are stricter and know the real
  targets — a false drop loses a posting silently, a false keep costs nothing.
- **Cost, measured end to end on 2026-09-30**: `--source ats_boards` went from 269s to
  **133s** while collecting *more* postings (16,939 vs 12,312 before Workday, against
  45,860 unfiltered). Workers 8 → 277s, 16 → 145s on a 200-board sample with identical
  yield, so the pool was the whole cost. `max_pages: 1` would halve the requests again but
  lose 44% of postings.
- **A failed board is retried, then counted.** Workday throttles by IP: the same board list
  lost about 50 boards at 8 workers off-peak and about 150 at 16 workers during US business
  hours, which is how the missing retry was noticed. `_workday_page` now retries three times,
  and the run reports how many boards still failed. Roughly 50 boards answer 422 because the
  harvested site name is wrong; those are genuine.

### Adopted 2026-10-01: the uniform JSON set

Seven vendors that share one shape: a slug in a URL template that answers with JSON. They
live in `config/json_boards.yaml` (**513 boards**), regenerated by `server/jobs/harvest/jsonboards.py`
from the same feed that fills the Workday list, and are fetched by `src_ats` as one parallel
batch next to Workday.

| vendor | boards | publish date in the payload |
| --- | --- | --- |
| SmartRecruiters | 234 | `releasedDate`, date-sorted newest first |
| Workable | 124 | `published_on` |
| Rippling | 111 | **none** |
| BambooHR | 33 | **none** |
| Breezy HR | 8 | `published_date` |
| Recruitee | 2 | `published_at` |
| Teamtailor | 1 | `date_published` |

- **One request per board for six of the seven.** These endpoints hand back the whole board
  in one response, so there is nothing to page. SmartRecruiters is the exception: it pages at
  100 and is date-sorted, so it stops at the window edge exactly as Workday does.
- **The keys are board-scoped, unlike every vendor before them.** A Greenhouse id is global,
  but a BambooHR id is a small integer unique only inside one company ("52", "448"). Keying
  those as `ats|<id>` would collide two employers' unrelated postings onto one ledger row and
  lose one silently. These use `ats|<vendor>:<slug>:<id>`, and the new `apply_signature()`
  builds that key for the board fetchers and the feed sources alike so the two cannot drift.
  The Greenhouse, Ashby, Lever and Workday keys were left byte-identical on purpose; the
  whole change re-keyed exactly **1** ledger row.
- **Group 2 of each pattern is the id in the vendor's own job link**, not the id its API
  returns next to it. Breezy returns `friendly_id` as `{id}-{slug}` while its links carry the
  bare id, and Teamtailor's feed id is a UUID while its links carry a number. Using the API
  field would mean the board path and the feed path could never agree on what is one posting.
- **Where a vendor states the country, that field is authoritative.** SmartRecruiters returns
  a bare two-letter code, and those codes collide head-on with the US state abbreviations the
  text filter hunts for: "Ludwigshafen, RP, **de**" reads as Delaware, "Barcelona, **CT**,
  es" reads as Connecticut. In a 60-board sample **581 overseas postings** survived that way.
  `_row_region_ok()` now uses the structured code when there is one and only falls back to
  the text heuristic when there is not. After the fix: 1,138 overseas postings dropped from
  the same sample, **0** North American ones lost.
- **Rippling and BambooHR carry no date at all**, in the board payload or anywhere else. Their
  postings arrive with `age_days: None` and `date_missing: True` instead of being treated as
  fresh. There is nothing to age them by, so the ledger reports each once and then absorbs
  them -- 912 of the 1,393 postings the set contributes on a first run.
- **A dead board is counted, not swallowed.** The batch reports how many boards failed, and
  how many postings were undated, because 4 of 513 failing quietly is how a source rots.

### Adopted 2026-10-01: iCIMS

247 customer hosts, harvested by `server/jobs/harvest/icims.py` into `config/icims_boards.yaml`. The
board is `https://{host}/jobs/search`.

**It is behind a bot wall, and that shapes everything about it.** Every host sits behind an
AWS WAF that answers a plain HTTP client with `405` and a "Human Verification" page —
urllib and curl alike, with a full set of browser headers, on every path including
`/sitemap.xml` and `/api/jobs`. A real browser is served the page normally, and it holds
**no `aws-waf-token` cookie**, so the WAF is judging the TLS fingerprint and there is no
token to borrow. `curl_cffi` impersonating a browser fingerprint is the only way through:
`chrome124` returns 200 where curl returns 405. Verified across profiles — chrome, chrome120,
chrome124 and safari17_0 pass; chrome110 and edge101 do not.

- **It therefore runs as a subprocess, not as a source function.** curl_cffi does not belong
  in the scan's interpreter, so `server/scan/icims_fetch.py` lives under `.venv-icims` and speaks JSON
  over stdin/stdout. The scan skips the source with a message when that venv is missing
  rather than failing the run. Rebuild it with `uv venv .venv-icims --python 3.13` then
  `uv pip install --python .venv-icims/bin/python curl_cffi`.
- **The search view has no date.** Not on the card, not in the header fields (only Position
  Type and Remote), and the sort control offers ID, Title, Location, Position Type, Remote
  and Job Description with no date option.
- **The default order is not a recency proxy.** This was worth checking rather than assuming:
  `careers-ice` lists ids descending, but `careers-peraton`'s most recent postings sit on its
  **last** page and `careers-sig` does not order by id at all. Checked against the feed's own
  `date_posted` values. Capping pages on the assumption that the first page is the newest
  would have silently missed recent postings on most boards.
- **The detail page does carry a date.** Each job page embeds schema.org JSON-LD with a real
  `datePosted`. So the list view enumerates cheaply and only postings surviving the title and
  country pre-filter get a detail fetch. `?in_iframe=1` is **mandatory** on that fetch —
  without it the page is a 129 KB JavaScript shell with no JSON-LD, with it 38 KB with the
  date.
- **`pr` is a 0-based page index, not an offset.** `pr=0` is the first page, `pr=4` the last
  on a 5-page board; `pr=50` and `pr=100` return zero cards, which is how the offset reading
  was ruled out.
- **A vendor-stated country beats the location text**, as with SmartRecruiters. Locations
  arrive as `US-FL-Jacksonville`, so the country token is read directly. A location in a
  format that names no country (some sites write "IN-Hyderabad") falls through to the text
  filter instead of being guessed at here.
- **Measured across all 247 hosts**: 447 postings inside 7 days, 444 of them dated, 4 boards
  that did not answer, 34 that filled every page requested. 179s with the keyword backfill,
  135s without it (401 postings). The truncation count is the honest limit of this source:
  it flags the plain pass, and a keyword backfill mitigates but does not remove it.
- **A keyword search narrows server-side** and is used only for boards that hit the page cap:
  on `careers-ice` the plain listing needs 5 pages while `searchKeyword=engineer` needs 2.

### Adopted 2026-10-04: Work at a Startup (Y Combinator)

`https://www.workatastartup.com` — the candidate asked for this board by name on 2026-10-04, from
a filtered URL (`role=eng&jobType=fulltime&minExperience=0&usVisaNotRequired=true`). It is a source
of its own (`kind: workatastartup`, `server/scan/sources/workatastartup.py`), not a board file.

**It is an Inertia app, and reading that is the whole scraper.** Every page embeds its props as
JSON in a `data-page` attribute on the root element. Both obvious alternatives fail:

- **The JSON API needs a version header.** `X-Inertia: true` alone answers **409** with an empty
  body; it needs `X-Inertia-Version` to match the current build hash, and that hash changes
  whenever YC rebuilds the front end. The HTML carries the same payload with no header at all.
- **`core.http_get` cannot reach this host.** It answers **406 Not Acceptable** to any request
  whose `Accept` header is not browser-shaped, and `http_get` sends `Accept: */*`. The source
  does its own fetching with `Accept: text/html,application/xhtml+xml,...`. Still no cookie, no
  token, no account. **Do not route this source through `http_get`.**

**The anonymous feed is a teaser and the query-string filters do nothing.** A page returns 30
jobs; `?page=2` returns the same 30; and `role`, `minExperience`, `usVisaNotRequired`, `jobType`,
`sortBy` and `query` all leave the result identical for a logged-out visitor. `totalJobsCount`
reports 2,897 open roles. What *does* differ is **which** 30, per page type, so the source sweeps
twelve paths (`/jobs`, `/companies?role=eng`, and the ten `/jobs/l/<role>` pages). Measured
2026-10-04: **285 distinct postings, 66 engineering-titled, 19 past the workspace filters**, in
3.2s. Never add a `?page=` path to the sweep.

- **No publish date exists anywhere on the site** — not in the list payload, not on the job page.
  Every posting is tagged `date_missing`, the same treatment Rippling and BambooHR get (which the
  candidate chose to keep). The sweep is served newest-first, but that is not a date and the
  source does not pretend it is: **the 7-day rule cannot be applied to these postings.**
- **Applying needs a YC account.** `applyUrl` points at
  `account.ycombinator.com/authenticate?...signup_job_id=<id>`, so these postings are read-only
  here and the application is the candidate's to make by hand. Treat them like LinkedIn postings.
- **The job page carries two filters the list does not**: `minExperience` and `sponsorsVisa`,
  plus `equityRange`, `skills` and the description. That is one request per job, so it is not
  fetched today — the list sweep is twelve requests total. Worth adding if the visa question ever
  needs answering per posting: a role marked "US citizen/visa only" is a wasted fill for a
  Canadian applicant, and he is TN-eligible rather than a citizen.
- **`board_slug` must not be reused for this.** The list gives a `companySlug`, which is a **YC**
  company slug, not an ATS board slug. It is stored as `company_slug`. Putting it in
  `board_slug` would feed `harvest_boards()` names that are not ATS boards and send
  `server/jobs/probe_ats.py` chasing them.

### Not scannable from this machine

- **Eightfold** — `https://{host}.eightfold.ai/api/apply/v2/jobs` is blocked or moved:
  qualcomm and johndeere return **403** (WAF), aexp **404**, zebra 200 with `count=0`. Their
  career URLs do exist in the feed (109 of them), so this is a browser-only source for now,
  not a dead one. Revisit with the visible browser.
- **JazzHR** (`applytojob.com`, 179 links) — `/apply/` returns 119–124 KB of HTML and there is
  no JSON at that path. Unverified as parseable; treat as HTML scraping if attempted.
- **Oracle Recruiting Cloud** (`*.fa.*.oraclecloud.com`, 1,362 links) — untouched. Oracle
  publishes an HCM REST API (`/hcmRestApi/resources/latest/recruitingCEJobRequisitions`);
  not yet tested.
- **Proprietary career sites** — `jobs.apple.com`, `amazon.jobs`, `jobs.bytedance.com`,
  `lifeattiktok.com`, `jobs.l3harris.com`. Each is its own API needing separate work.

`APPLY_SIGNATURES` in `server/scan/engine.py` currently matches only greenhouse, ashby and lever, so
every posting from all of the vendors above currently arrives with no `ats` tag and no board
coverage.

## Still untested

- `remoteok.com/api` — a public JSON feed, often cited, never tested here.
- `weworkremotely.com` category RSS feeds.
- HN "Who is hiring" through `hn.algolia.com/api/v1` — monthly thread, structured comments.

## The lesson from startups.gallery

The feed itself was adopted quickly. Its real value was not the ~20 postings it renders per
run but the employer boards every card names, which the scan harvests into
`state/sg_boards.json`. That file accumulated **45 boards, and 42 of them were still absent
from `config/ats_boards.yaml`** — 1,597 open roles the scan could not see — until 2026-09-30,
because `server/jobs/probe_ats.py` had never been run against them.

**It happens again on its own.** The scan now says so every run, which is the only reliable
trigger, and the line is an instruction rather than a note:

    [scan] 14 harvested boards are not in ats_boards.yaml -- run server/jobs/probe_ats.py: ashby/conduct, ...

That is `server/scan/harvest.py::unprobed_boards()`, printed by `cli.py` right after the
harvest. It covers only the kinds the probe can read (Greenhouse, Ashby, Lever, Workable) so
the iCIMS and SmartRecruiters boards do not appear as permanent noise.

Re-probed twice since, both 0 dropped:

| when | file went | added | yield measured |
| --- | --- | --- | --- |
| 2026-10-03 | 123 → 137 | 14 | 365 raw postings, **1** past the filters |
| 2026-10-05 | 137 → 164 | 28 | 1,314 raw postings, **1** past the filters |

**Expect the yield per re-probe to be small, and do not read that as failure.** The first pass
(42 boards on 2026-09-30) was the exception, not the rule: it landed 1,597 roles in one go
because those boards had never been probed at all. The later additions are mostly small,
senior-heavy startups — 1Password, Sentry, Writer and TRM Labs are the size of company that
posts one or two junior roles a year. The boards are worth having for what they will post, and
the cost is a 90-second probe, so re-run it whenever the scan prints the line. Check the yield
before calling a re-probe a win.

`server/jobs/probe_ats.py` regenerates the whole file from its candidate list and never reads the
existing one, so a board that fails one transient request drops out. Back it up first
(`state/ats_boards.backup-<timestamp>.yaml`), run the probe, then diff old against new and
confirm any drop is a real 404 rather than a blip. On 2026-10-03 the probe added 14 and dropped
0 — the boards it printed `dead` were candidate-list entries that had never made the file, so
the "dead" list is not a list of losses. Diff before believing either way.

**Slugs are case-sensitive, and are not always the company name.** Lever needs `Flex` with a
capital F — `flex` answers 404 — and Ashby's Heidi Health is `heidihealth.com.au`. Keep exactly
what the feed harvested and never lowercase a slug on the way into a request; lowercasing is
for comparison only.

**What the 14 bought, measured:** 365 raw postings, **1 past all the filters** (Duna, Backend
Engineer, Remote, 5d). Most of these startups are not Canadian or American — Heidi Health is
Australian, Langdock and Taktile are German — and the rest are senior. That is not a reason to
drop them: a board is worth having for what it will post, not what it holds today. It is a
reason to check the yield before calling a re-probe a win.
