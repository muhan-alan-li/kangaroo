# S4a task: Save one real response per source family

This slice writes fixtures only. Do not write Rust, do not change `server/`, `client/` or
`data/`.

Read `docs/rust-app-plan.md` §16.3 first. The parsers for S4 and S5 need a saved response per
endpoint family. One response each is enough.

## Fetch list

Make these requests in this order, one at a time, at least one second apart. One attempt each.
A 403 or 404 is a finding, not an error: record it and move on. Never retry.

| # | Name | Request |
|---|---|---|
| 1 | `simplify_json` | GET `https://raw.githubusercontent.com/SimplifyJobs/New-Grad-Positions/dev/.github/scripts/listings.json` |
| 2 | `simplify_list` | GET `https://raw.githubusercontent.com/SimplifyJobs/New-Grad-Positions/dev/README.md` |
| 3 | `jobbank_search` | GET `https://www.jobbank.gc.ca/jobsearch/jobsearch?searchstring=software%20engineer&locationstring=Vancouver%2C%20BC&sort=M` |
| 4 | `jobbank_detail` | Take one numeric job id from the `id="article-<id>"` marker in file 3 and GET `https://www.jobbank.gc.ca/jobsearch/jobposting/<id>` |
| 5 | `ats_greenhouse` | GET `https://boards-api.greenhouse.io/v1/boards/databricks/jobs` |
| 6 | `ats_ashby` | GET `https://api.ashbyhq.com/posting-api/job-board/openai` |
| 7 | `ats_lever` | GET `https://api.lever.co/v0/postings/palantir?mode=json` |
| 8 | `json_workable` | GET `https://apply.workable.com/api/v1/widget/accounts/accora?details=true` |
| 9 | `json_smartrecruiters` | GET `https://api.smartrecruiters.com/v1/companies/3SBusinessCorporationInc1/postings` |
| 10 | `json_rippling` | GET `https://ats.rippling.com/api/v1/board/aalyria-careers/jobs` |
| 11 | `json_bamboohr` | GET `https://adidev.bamboohr.com/careers/list` |
| 12 | `json_recruitee` | GET `https://1x.recruitee.com/api/offers/` |
| 13 | `json_breezy` | GET `https://hmt-tank.breezy.hr/json` |
| 14 | `json_teamtailor` | GET `https://noda-1720000970.teamtailor.com/jobs.json` |
| 15 | `workday_board` | POST `https://globalhr.wd5.myworkdayjobs.com/wday/cxs/globalhr/rec_rtx_ext_gateway/jobs` with body `{"appliedFacets":{},"limit":20,"offset":0,"searchText":""}`, `Content-Type: application/json`, `Accept: application/json`, `Referer: https://globalhr.wd5.myworkdayjobs.com/rec_rtx_ext_gateway` |
| 16 | `startups_gallery` | GET `https://startups.gallery/jobs` |
| 17 | `workatastartup` | GET `https://www.workatastartup.com/jobs` with `Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8` and `Accept-Language: en-CA,en;q=0.9` |

Use a normal browser user agent string. Save the body byte for byte, with the extension the
content type implies (`.json`, `.html`). Do not reformat, truncate or pretty-print a body.

Write each body to `fixtures/http/<name>/<file>`. If a body is over 4 MiB, save the first 4 MiB
and say so in the README.

## README

Write `fixtures/http/README.md` with one row per request: name, URL, method, the headers that
matter, HTTP status, content type, saved byte count, fetch time in UTC, and one line on what the
parser must read from it. Say "not fetched" and the reason for any row you could not complete.

## Rules

- Maximum 25 requests in total.
- No cookies, no sign-in, no CAPTCHA, no retry on a refusal.
- Do not save anything that identifies a person.
- One commit at the end: `test: Save one response per source family`.

## Acceptance

- `fixtures/http/README.md` has a row for all 17 names.
- Every row is either a saved file with a matching byte count, or "not fetched" with a status.
- `git status` is clean after the commit.
