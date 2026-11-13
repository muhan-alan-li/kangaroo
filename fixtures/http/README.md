# Saved source responses

One real response per endpoint family the scan reads, for the S4/S5 parser tests
(`docs/rust-app-plan.md` §16.3). Collected 2026-11-13, 17 requests in the order below,
about 90 seconds end to end.

- One attempt per request, no retries, at least one second between requests (17 of the
  25-request budget).
- All requests were plain unauthenticated `curl 8.7.1` fetches with a normal Chrome
  user-agent string. No cookies were sent or stored, no sign-in, no CAPTCHA, no redirect
  was followed (every response below was final at the URL shown).
- No response carried a `Content-Encoding`, so the saved bytes are the served bytes.
  Nothing was reformatted, truncated (except the three 4 MiB caps noted below) or
  pretty-printed.
- "UA only" in the headers column means the browser User-Agent plus curl's default
  `Accept: */*`.
- The fixture tree carries a `.gitattributes` with `* -text`, so git stores every body
  exactly. This matters: the startups.gallery page uses CRLF line endings, and the
  repo's `core.autocrlf=input` deleted 279 bytes from it before the attribute was added.
- Fetch times are the client clock (UTC) at the moment each request started.
- No row is "not fetched": all 17 were requested and all answered.

| # | Name | Method | URL | Headers that matter | Status | Content-Type | Bytes saved | Fetched (UTC) |
|---|---|---|---|---|---|---|---|---|
| 1 | `simplify_json` | GET | `https://raw.githubusercontent.com/SimplifyJobs/New-Grad-Positions/dev/.github/scripts/listings.json` | UA only | 200 | `text/plain; charset=utf-8` | 4,194,304 (of 12,939,458 — capped) | 2026-11-13T22:20:29Z |
| 2 | `simplify_list` | GET | `https://raw.githubusercontent.com/SimplifyJobs/New-Grad-Positions/dev/README.md` | UA only | 200 | `text/plain; charset=utf-8` | 815,879 | 2026-11-13T22:20:44Z |
| 3 | `jobbank_search` | GET | `https://www.jobbank.gc.ca/jobsearch/jobsearch?searchstring=software%20engineer&locationstring=Vancouver%2C%20BC&sort=M` | UA only | 200 | `text/html;charset=UTF-8` | 286,493 | 2026-11-13T22:20:49Z |
| 4 | `jobbank_detail` | GET | `https://www.jobbank.gc.ca/jobsearch/jobposting/50720915` | UA only | 200 | `text/html;charset=UTF-8` | 193,268 | 2026-11-13T22:20:55Z |
| 5 | `ats_greenhouse` | GET | `https://boards-api.greenhouse.io/v1/boards/databricks/jobs` | UA only | 200 | `application/json` | 768,040 | 2026-11-13T22:21:05Z |
| 6 | `ats_ashby` | GET | `https://api.ashbyhq.com/posting-api/job-board/openai` | UA only | 200 | `application/json; charset=utf-8` | 4,194,304 (of 14,082,202 — capped) | 2026-11-13T22:21:09Z |
| 7 | `ats_lever` | GET | `https://api.lever.co/v0/postings/palantir?mode=json` | UA only | 200 | `application/json; charset=utf-8` | 4,194,304 (of 5,840,709 — capped) | 2026-11-13T22:21:15Z |
| 8 | `json_workable` | GET | `https://apply.workable.com/api/v1/widget/accounts/accora?details=true` | UA only | 200 | `application/json; charset=utf-8` | 28,828 | 2026-11-13T22:21:20Z |
| 9 | `json_smartrecruiters` | GET | `https://api.smartrecruiters.com/v1/companies/3SBusinessCorporationInc1/postings` | UA only | 200 | `application/json; charset=utf-8` | 60,158 | 2026-11-13T22:21:23Z |
| 10 | `json_rippling` | GET | `https://ats.rippling.com/api/v1/board/aalyria-careers/jobs` | UA only | 200 | `application/json` | 10,224 | 2026-11-13T22:21:28Z |
| 11 | `json_bamboohr` | GET | `https://adidev.bamboohr.com/careers/list` | UA only | 200 | `application/json` | 7,838 | 2026-11-13T22:21:32Z |
| 12 | `json_recruitee` | GET | `https://1x.recruitee.com/api/offers/` | UA only | **404** | `application/json; charset=utf-8` | 21 | 2026-11-13T22:21:35Z |
| 13 | `json_breezy` | GET | `https://hmt-tank.breezy.hr/json` | UA only | 200 | `application/json; charset=utf-8` | 2 | 2026-11-13T22:21:42Z |
| 14 | `json_teamtailor` | GET | `https://noda-1720000970.teamtailor.com/jobs.json` | UA only | 200 | `application/feed+json; charset=utf-8` | 196 | 2026-11-13T22:21:46Z |
| 15 | `workday_board` | POST | `https://globalhr.wd5.myworkdayjobs.com/wday/cxs/globalhr/rec_rtx_ext_gateway/jobs` | `Content-Type: application/json`; `Accept: application/json`; `Referer: https://globalhr.wd5.myworkdayjobs.com/rec_rtx_ext_gateway` | **521** | `text/plain; charset=UTF-8` | 16 | 2026-11-13T22:21:51Z |
| 16 | `startups_gallery` | GET | `https://startups.gallery/jobs` | UA only | 200 | `text/html` | 444,676 | 2026-11-13T22:21:55Z |
| 17 | `workatastartup` | GET | `https://www.workatastartup.com/jobs` | UA only; `Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8`; `Accept-Language: en-CA,en;q=0.9` | 200 | `text/html; charset=utf-8` | 88,238 | 2026-11-13T22:21:59Z |

The Workday body was exactly
`{"appliedFacets":{},"limit":20,"offset":0,"searchText":""}` sent with
`--data-binary` so nothing was re-encoded.

## What each parser must read

| Name | File | Parser must read |
|---|---|---|
| `simplify_json` | `simplify_json/listings.json` | A JSON array; each item carries `company_name`, `title`, `locations`, `url`, `date_posted` (epoch), `active`, `is_visible`, `sponsorship`, `category`, `id`. Read `active`/`is_visible` — the flags are why this source exists. The file is the first 4 MiB of a 12,939,458-byte body, cut mid-object, so a test must stop at the last complete object. |
| `simplify_list` | `simplify_list/README.html` | The derived Markdown/HTML: live postings are `<tr>`/`<td>` rows in the top table (2,599 `<tr>` here); closed postings sit in 84 collapsed `<details>` blocks that still hold tables. Read company, title, location, the apply `href` and the age text from the top table only. Saved as `.html` because the scan parses this body as HTML, not as prose. |
| `jobbank_search` | `jobbank_search/jobsearch.html` | One `<article id="article-<id>">` per result (25 here), containing `<span class="noctitle">` (title), `<li class="business">` (employer), `<li class="date">` (publish date text), `<li class="salary">` and `<li class="location">`. |
| `jobbank_detail` | `jobbank_detail/jobposting-50720915.html` | The posting page for id `50720915`: `<span class="source-title-inner">` (real title), `property="description"` (body), `property="datePosted"`, `property="validThrough"`, `property="employmentType"`, and `id="externalJobLink"` (the employer's own apply URL). |
| `ats_greenhouse` | `ats_greenhouse/databricks.json` | `jobs[]`, each with `id`, `title`, `absolute_url`, `location.name` and `first_published` (never `updated_at`, which is a rescore timestamp). |
| `ats_ashby` | `ats_ashby/openai.json` | `jobs[]`, each with `id`, `title`, `location`, `jobUrl` and `publishedAt`; drop entries with `isListed: false`. First 4 MiB of a 14,082,202-byte body, cut mid-array. |
| `ats_lever` | `ats_lever/palantir.json` | A JSON array; each posting has `id`, `text` (title), `categories.location`, `hostedUrl` and `createdAt`. First 4 MiB of a 5,840,709-byte body, cut mid-array. |
| `json_workable` | `json_workable/accora.json` | `jobs[]`, each with `shortcode`, `title`, `city`/`state`/`country`, `application_url` and `published_on`. |
| `json_smartrecruiters` | `json_smartrecruiters/3SBusinessCorporationInc1.json` | `content[]`, paged via `offset`/`limit`/`totalFound`; each item has `id`, `name`, `releasedDate` and a structured `location.country` ISO code (use it instead of the location text — "de"/"CT" collide with US state abbreviations). |
| `json_rippling` | `json_rippling/aalyria-careers.json` | A JSON array; each job has `uuid`, `name`, `workLocation` and `url`. There is no publish date anywhere in the payload — every posting is undated (`age_days: None`). |
| `json_bamboohr` | `json_bamboohr/careers-list.json` | `result[]` (with `meta.totalCount`); each item has `id`, `jobOpeningName`, `location {city, state}` and `atsLocation`. No publish date anywhere — undated like Rippling. |
| `json_recruitee` | `json_recruitee/offers.json` | Nothing usable: **404** with the body `{"error":"Not Found"}` (21 bytes). The host answered; the slug `1x` has no public offers endpoint. Recorded as the finding, not retried. |
| `json_breezy` | `json_breezy/hmt-tank.json` | A JSON array, empty at fetch time (`[]`, 2 bytes). The parser must treat an empty board as zero postings, not a failure. |
| `json_teamtailor` | `json_teamtailor/noda-1720000970.json` | A JSON Feed 1.1 document; postings live in `items[]`, empty at fetch time (`items: []`, 196 bytes). Same empty-board rule as Breezy. |
| `workday_board` | `workday_board/jobs.json` | Nothing usable: **521** with the body `error code: 521` (16 bytes) — Cloudflare reported the Workday origin unreachable. Recorded as the finding, not retried. A successful response would carry `jobPostings[]` with `title`, `externalPath`, `locationsText` and `postedOn`. |
| `startups_gallery` | `startups_gallery/jobs.html` | 20 `<a class="framer-zpdfqp" ... href="<ATS apply URL>">` job cards; the `<h2>` is the title, the text after it splits on `·` into company then locations. **Drift:** the date is now a `<time datetime="2026-11-13T00:00:00.000Z">· Nov 13, 2026</time>` child and the old `Posted on …` text is gone, so the current `gallery.py` regex finds no date; the Rust parser should read `<time datetime>`. |
| `workatastartup` | `workatastartup/jobs.html` | A single HTML-escaped Inertia `data-page="…"` attribute; JSON-decode it and read `props.jobs[]` (30 jobs) for `id`, `title`, `location`, `companyName`, `companySlug` and `applyUrl`. The browser-shaped `Accept` header is required — the site answers 406 otherwise. |

## Findings

- **`json_recruitee` — 404.** `https://1x.recruitee.com/api/offers/` answered
  `{"error":"Not Found"}`. No retry per the task rules. If the slug is wrong, the
  parser test should use a different board, not this body.
- **`workday_board` — 521.** The CXS endpoint answered `error code: 521` (16 bytes),
  Cloudflare's "origin unreachable". No retry per the task rules; the body is not a
  board payload and cannot feed a parser test.
- **`json_breezy` and `json_teamtailor` — empty boards.** Both answered 200 with a
  structurally valid, empty listing. They pin the empty-board case rather than a
  populated one; a populated response for either shape still needs capturing.
- **`startups_gallery` date drift.** The saved page dates its cards with
  `<time datetime="…">` and no longer writes `Posted on Sep 28, 2026`, so the existing
  Python regex (`r"Posted on (.+)$"`) records no publish date for any card on this page.
  The fixture still exercises the card split, h2 title and apply-URL harvest.
- **Three bodies were capped at 4 MiB** (`simplify_json`, `ats_ashby`, `ats_lever`). The
  original `Content-Length` is in the table; the files are exact 4,194,304-byte prefixes
  and end mid-token, so the Rust tests must not expect parseable JSON from them.
- **Personal data:** nothing saved identifies a person. The only addresses in the bodies
  are company mailboxes (`accommodations@palantir.com`, `info@accora.care`,
  `workatastartup@ycombinator.com`); no phone numbers, no candidate data, no cookies.
