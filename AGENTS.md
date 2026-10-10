# Job Hunt Workspace

Job search and application workflow for junior and new-grad software engineering roles in Vancouver, BC.

Owner: Muhan Li. Agent: Hermes. **The procedure lives in the `job-application-run` skill.** This file is the map:
what the files are, how a day runs, and the rules that always apply.

## Layout

    data/candidate/              The candidate's own inputs, and the single source of truth for every
                                     application field -- read this before writing any answer. Holds
                                     candidate_profile.yaml (parsed resume + `answers_bank`), resume-text.md
                                     (text of every PDF; refresh with `python3 -m server resume-text`), data/candidate/resume/
                                     (the PDFs and tailored variants), data/candidate/documents/ (uploads: transcript and
                                     similar), and outreach-voice/ + outreach-samples/ (voice evidence).
    data/config/                 filters.yaml (matches, tiers, comp floor), sources.yaml, ats_boards.yaml,
                                     workday_boards.yaml, json_boards.yaml, source_candidates.md (the register of
                                     every source considered).
    docker/                          The container image and its two entry points: Dockerfile
                                     (python:3.13-slim plus tzdata, pyyaml, curl_cffi, pillow),
                                     compose.yaml (the scan and retry services: server/, data/config/
                                     and data/candidate/ read-only, data/state/ read-write, public DNS
                                     instead of Pi-hole, user 1000:1000), entrypoint.sh
                                     (sweep-if-stale, scan, mark-reported under one flock) and
                                     entrypoint-retry.sh (retry, then prune after a clean run).
    client/                          The dashboard's web client, served by the server/ package. Two pages: index.html
                                     (the postings table) and health.html (scan cost and Workday board health),
                                     sharing styles.css -- the "Lilac" theme, a mix of Ant Design primitives,
                                     Apple softness and terminal chrome -- and js/ (main, health, api, render,
                                     table, detail, util). The detail drawer has Summary / Resumé / Contact
                                     tabs; contact research and resumé-tailoring advice live in the latter
                                     two. Plain HTML/CSS/ES modules -- no build step and no npm; edit and reload.
                                     Client assets carry a `?v=` cache-buster: run `python3 -m server bump --bump`
                                     after editing them, or tailscale serve's Last-Modified-only caching can
                                     leave a browser holding one stale module and break the ES-module graph
                                     (a page with an empty table).
    server/                          The server package: the dashboard API, the scan engine and the
                                     brief/research engine, behind one entry point (`python3 -m server
                                     <command>`; with no command it serves the dashboard). api/ is the
                                     HTTP service (app routes, store reads data/state/, briefs launches a
                                     generation, net binds, serve loops). scan/ is the scan engine
                                     (engine.py plus core, parsing, filters, ledger, posting, pipeline,
                                     boardstate, portal, keywords, harvest, boards and endpoints, and
                                     sources/ with one module per feed). brief/ is the on-demand research engine (generate,
                                     research, llm, and the Node fetch helper). jobs/ holds the
                                     scheduled and maintenance commands: scan/ owns the scan job's
                                     scripts (sweep, mark-reported), retry/ the retry job's (retry,
                                     prune); the rest are hand-run utilities (refresh, probe-ats,
                                     harvest, table, feed, bump, resume-text). config.py holds
                                     the shared paths and constants; tests/ holds the four smoke tests.
                                     Adding a source means a module in scan/sources/, a row in
                                     SOURCE_FUNCS, a block in data/config/sources.yaml, and a row in
                                     source_candidates.md. Local dev: `python3 -m server` serves
                                     client/ + API. Production: `--api-only` drops static serving and
                                     binds loopback, and tailscale serve hosts client/ on 8443 and
                                     proxies /api here.
    data/state/                  seen.jsonl, outreach.jsonl, last_scan.json, board_health.json
                                     (per-board Workday health and its retry schedule),
                                     workday_sweep.json (the cache the daily scan reads instead of
                                     re-sweeping 1,328 boards), sg_boards.json (the employer boards a
                                     feed has revealed, for server/jobs/probe_ats.py), resume-backups/
                                     (every superseded resume PDF), plus scan.log and retry.log (the
                                     two no-agent jobs' captured output).
    systemd/                         The two timers and their services: job-scan.{service,timer}
                                     (Mon-Fri 09:00, 13:00, 17:00) and job-retry.{service,timer}
                                     (Sunday 22:00), symlinked into ~/.config/systemd/user/.

**Auto-apply is retired, and so is its workspace (2026-10-07).** The agent no longer fills or submits
applications. The `drafts/` folders, the application ledger (`data/state/applied.jsonl`) and `reports/` are all gone, so
nothing keeps per-application artifacts any more. `data/state/` is the scan's output and the portal's data, and outreach
is still logged in `data/state/outreach.jsonl`.

## How a day runs

1. **Mon–Fri 09:00, 13:00 and 17:00.** `job-scan.timer` is a systemd user timer that
   starts `job-scan.service`, a oneshot that runs `docker compose -f docker/compose.yaml
   run --rm scan`. The scan runs inside the `job-scan` container (`docker/Dockerfile` +
   `docker/entrypoint.sh`: sweep-if-stale, `server scan --per-tier 5`, mark-reported),
   with `server/`, `data/config/` and `data/candidate/` mounted read-only and `data/state/` read-write,
   so the dashboard reads the results as before. The container's `dns:` points at
   1.1.1.1/8.8.8.8 instead of Pi-hole, so the scan's burst of lookups never touches the
   home network's 1000-queries-per-60s rate limit. The unit files live in `systemd/` and
   are symlinked into `~/.config/systemd/user/`; a missed slot (sleep, off) simply waits
   for the next one (`Persistent=false`). The scan takes an exclusive `flock` on
   `data/state/.scan.lock` itself (`server/scan/engine.py`), so a manual host run and a
   container slot exclude each other; the entrypoint holds the same lock for its whole
   run (sweep, scan, mark-reported) and the scan inherits it via `JOB_SCAN_LOCK_HELD`.
   Its stdout is empty on purpose — everything goes to `data/state/scan.log`.
   The old Hermes `job-scan` no-agent job is retired; `~/.hermes/scripts/job-scan-store.sh`
   stays on disk as the record of what the entrypoint replaced. Manual use:
   `make scan-docker`, or `systemctl --user start job-scan.service` for one immediate run.
2. **Sunday 22:00, weekly.** `job-retry.timer` is the second systemd timer: it starts
   `job-retry.service`, a oneshot that runs `docker compose -f docker/compose.yaml run
   --rm retry`. The retry pass (`docker/entrypoint-retry.sh`: `server retry`, then `server
   prune` after a clean run) re-checks only the Workday boards whose backoff has come up
   and records the outcome in `data/state/board_health.json`, under the same public-DNS bypass
   as the scan — it hits the same Workday hosts, so sharing Pi-hole's bucket would be the
   same failure. After a clean retry it also runs `server/jobs/retry/prune.py`, which
   removes boards that are structurally dead (a Workday 404/422 — a wrong career-site name
   — which no retry can fix) from `data/config/workday_boards.yaml` and `data/state/board_health.json`
   (the retry service mounts `data/config/` read-write for exactly this; the scan keeps it
   read-only). It never removes on a transient failure. Removed keys are appended to
   `data/config/workday_tombstones.yaml`, which `harvest workday` consults before rewriting the
   board file — the dead triples came from feed URLs, so without the tombstones the next
   harvest would re-add them. Delete a tombstone line by hand to let a board back in.
   It shares the scan's lock, so the
   two never hit Workday at once. Output is in `data/state/retry.log`. The old Hermes `job-retry`
   job is retired like `job-scan` above. Manual use: `docker compose -f docker/compose.yaml
   run --rm --entrypoint /usr/local/bin/retry-entrypoint retry --dry-run`.
3. **On the user's go-ahead.** The agent fills the application in a real browser, screenshots it, queues it, and
   submits on one approval.

The single job is paused when the user asks; resume it with `cronjob_manage`. There is no agent in the scan loop,
so a run cannot score postings or draft anything: the portal shows `server/scan/engine.py`'s deterministic keyword score.

**Resuming fires every missed slot at once.** A resumed job runs every slot it slept through, so several full
scans can start within the same second. The scan holds an exclusive `flock` on `data/state/.scan.lock` for the whole
run and the loser exits loudly, and the ledger, `last_scan.json` and `data/state/sg_boards.json` are still written
with pid-tagged temp names and an atomic swap.

**The digest workflow is retired.** Nothing writes a dated digest any more, no message is
delivered to Discord (the job's `deliver` is `local`), and the tools that supported it — `bin/scan_summary.py` and
`bin/append_digest.py` — are gone. `server/jobs/scan/mark_reported.py` stays because the no-agent scan job
calls it. Re-enabling an agent run is a deliberate change, not part of the daily scan.

## What the scan keeps

Seven sources in `data/config/sources.yaml`: `jobbank` (official, local, carries a posted date), `newgrad_list` (the
Simplify new-grad feed, read as HTML, mostly US), `newgrad_json` (the same repo's machine-readable feed, added
2026-09-30), `ats_boards` (public Greenhouse, Ashby and Lever boards: the strongest source, because those postings
are real and applyable), `startups_gallery` (early-stage startups, newest postings first), `workatastartup` (Work at
a Startup, Y Combinator's feed: 285 distinct startup roles per sweep, added 2026-10-04), and `linkedin` (public
guest search, read-only, never signed in). Indeed, Eluta, Careerjet, Talent.com and ZipRecruiter are off: they
answer 403 or need JavaScript from this machine. Re-test before re-enabling any of them.

`workatastartup` carries two properties worth knowing before anyone "improves" it. It answers **406** to any request
whose `Accept` header is not browser-shaped, so it does its own fetching and must never be routed through
`core.http_get`; and **no publish date exists anywhere on that site**, list payload or job page, so its postings are
tagged `date_missing` and the 7-day rule cannot be applied to them. Its apply link needs a YC account, so those
applications are the candidate's to make by hand, like LinkedIn's.

- **Age comes from the publish field**, never `updated_at`: a board touches that field whenever it rescores, so a
  June posting reads as "0 days old". Job Bank's posted date, Greenhouse `first_published`, Lever `createdAt`,
  Ashby `publishedAt`. `max_age_days` is **7**; older postings are dropped.
- **`newgrad_list` reads a derived file, and the derivation hides closes.** The Simplify repo generates its
  `README.md` *from* `.github/scripts/listings.json` and demotes closed postings into collapsed `<details>` blocks
  that still hold a full `<table>`, so the HTML source cannot tell a live row from a dead one and age cannot filter
  them: a posting closed today is still young. On 2026-09-30 it ingested 2,383 rows of which only 280 were live.
  `newgrad_json` reads the JSON, which carries `active`, `is_visible`, the exact `date_posted` and the employer's
  own ATS URL. Both stay on, and the overlap is collapsed once, at the merge point, by the ATS signature of the
  apply URL (`server/scan/posting.py: collapse_requisitions`) -- not inside either source, because a source's own id
  scheme is what makes two copies of one requisition under two ids. Never
  filter that feed on `category` (the strings are inconsistent, so an allow-list drops roles silently) or on
  `sponsorship` beyond an explicit `Does Not Offer Sponsorship`.
- **Workday is the largest source and lives in its own board file.** `data/config/workday_boards.yaml` holds ~1,290
  boards (1,328 before the 2026-10-07 prune; `server/jobs/retry/prune.py` drops structurally dead entries),
  harvested from the feed by `server/jobs/harvest/workday.py` (Workday needs a tenant + wd number + career site,
  not a slug). The scan reads the **first page only** of each board, because the board API is date-sorted newest
  first and carries a real publish field — paging further would mean hundreds of thousands of requests a day for
  postings months old. The user's basic filter for this source runs inside it: engineering titles, Canada/US, and
  posted within 7 days, which drops ~92% of what a board returns before the other filters ever see it.
- **Seven more vendors share one shape and live in `data/config/json_boards.yaml`.** Workable, SmartRecruiters,
  Rippling, BambooHR, Recruitee, Breezy and Teamtailor are 513 boards, harvested by
  `server/jobs/harvest/jsonboards.py` from the same feed. Six of the seven answer with the whole board in one request,
  and SmartRecruiters is date-sorted so it stops at the window edge like Workday. They take the same three
  basic filters. Two of them — Rippling and BambooHR — publish **no date anywhere**, so their postings carry
  `age_days: None` and `date_missing: True`; report each once and never present an undated posting as fresh.
  Every JSON-vendor board also leaves a health row in `data/state/board_health.json` (`vendor|slug`, two segments, so
  it can never collide with a Workday triple), so a board that stops answering shows up on the dashboard instead
  of being a stderr line nobody sees.
- **A vendor's stated country beats the location text.** SmartRecruiters returns a bare ISO code, and those
  codes collide with US state abbreviations ("de" is Delaware, "CT" is Connecticut). `_row_region_ok()` uses
  the structured field when there is one. Never widen it back to text-only: that leak was 581 overseas
  postings in a single 60-board sample.
- **A failed board page is retried, and the count of boards that still failed is printed.** Workday throttles
  by IP: the same board list lost ~50 boards at 8 workers off-peak and ~150 at 16 during US business hours.
  A three-attempt retry brought that back to ~60. A board skipped in silence is how a source rots unnoticed.
- **startups.gallery hands over more than its own feed.** The page renders about 20 of the newest postings and
  deep-links each one into the employer's ATS, so the scan sees the board slug behind every card and harvests it
  into `data/state/sg_boards.json`. `server/jobs/probe_ats.py` reads that file, so a company seen once gets its whole board
  scanned later. Run the probe to fold the harvested slugs into `data/config/ats_boards.yaml`. The probe drops a board
  only after **two consecutive** failed passes (streaks in `data/state/ats_probe_failures.json`): a board already in the
  config survives its first failure, so one bad network afternoon cannot silently shrink the fleet.
- **A level-II title is gated, not excluded.** Report one only with a strong, named fit: the posting's two or three
  main requirements and the past work that meets each. Otherwise it goes to a one-line "Left out" note. Level III
  and L4 sit above level II, so they are not easier to reach. A range including level I counts as level I.
- **Location tiers set the order** in `data/config/filters.yaml: location_tiers`: vancouver, remote_canada, toronto,
  seattle, bay_area, remote_us, remote_unspecified. No match means dropped. A US posting is kept and flagged,
  because the application asks about US work authorization. `--per-tier N` caps each tier; the cron uses 5.

## Manual use

Run these with `/usr/bin/python3`, not a bare `python3`. Inside an agent session `python3` resolves to the Hermes
toolchain build, which has no PyYAML: on 2026-10-02 the scan printed `scan.py needs PyYAML (pip install pyyaml)`
instead of postings, and nothing else looked wrong. `~/.hermes/scripts/job-scan-store.sh` pins the same interpreter
and fails loudly when it cannot import yaml.

    /usr/bin/python3 -m server scan                 # fresh postings, as a table
    /usr/bin/python3 -m server scan --json          # machine readable
    /usr/bin/python3 -m server scan --all           # ignore the dedupe ledger
    /usr/bin/python3 -m server scan --per-tier 5    # at most 5 postings per tier
    /usr/bin/python3 -m server scan --source jobbank
    /usr/bin/python3 -m server probe-ats            # test ATS slugs, rewrite data/config/ats_boards.yaml
    /usr/bin/python3 server/tests/test_boardstate.py   # smoke: board health, backoff, retry queue, sweep cache
    /usr/bin/python3 server/tests/test_dashboard.py    # smoke: client routes, MIME types, path check
    /usr/bin/python3 server/tests/test_net.py          # smoke: DNS cache, pacing, resolver pause, socket cap
    /usr/bin/python3 server/tests/test_retry.py        # smoke: probe-result contract, retry unpack
    /usr/bin/python3 -m server feed --days 7 --where canada   # quick look at the Simplify feed
    /usr/bin/python3 -m server retry --dry-run     # re-check the Workday boards that did not answer
                                                     #   (the Sunday job-retry runs this script itself)
    /usr/bin/python3 -m server sweep --dry-run   # sweep the Workday boards into the cache
    /usr/bin/python3 -m server                       # serve client + API (local dev, foreground)
    /usr/bin/python3 -m server --api-only --port 8788  # API only, loopback (the production role)

## The dashboard

The dashboard is two pieces. `tailscale serve` hosts the static page at
**https://home.taild62434.ts.net:8443/** and reverse-proxies `/api` to the Python server; the Python
server (`server/`, run as `python3 -m server --api-only --port 8788`) binds **loopback only**, so the
API is never on the LAN and never directly on the tailnet. Every request re-reads `data/state/`, so the page
is current the moment a scan finishes. For local development `python3 -m server` (no `--api-only`)
serves `client/` and the API together on the Tailscale address and loopback, exactly as it used to.

The page itself lives in `client/`, served verbatim: plain HTML, one stylesheet, and ES-module
JavaScript with no build step. `server/api/app.py` answers the JSON the client calls -- `GET /api/dashboard`
(everything the page shows), `GET /api/posting/<id>` (one posting's detail), and the two routes that
write: `POST /api/brief/<id>` (starts `server/brief/generate.py`: the brief and the first-level contact
loop) and `POST /api/deep-contacts/<id>` (the same script with `--deep-contacts`: the Hermes agent's
second-level contact search, merged into the existing brief). The client is read-only apart from those
two buttons. `server/tests/test_dashboard.py` pins the routes and the static path check.

The server's modules are split by what they touch: `server/config.py` (paths, tiers, MIME),
`server/api/store.py` (reading `data/state/`), `server/api/briefs.py` (the brief cache and launcher),
`server/api/net.py` (the bind address), `server/api/app.py` (the HTTP handler), `server/api/serve.py` (args and the
serving loop).

    https://home.taild62434.ts.net:8443/    # the page; the API is loopback-only on :8788

The node was renamed to `home` on 2026-10-03 so the name is short enough to type on a phone.

**Port 443 is not available to `tailscale serve` on this machine.** Pi-hole v6 runs here and its web
server binds `0.0.0.0:443`, so it answers instead and presents a `CN=pi.hole` certificate. The serve
config therefore lives on 8443. Do not try to free 443 by editing Pi-hole: the owner asked that
Pi-hole be left alone.

The `tailscale serve` config is one root file mount plus one `/api` proxy mount. The proxy target ends
in `/api` because Serve strips the mount prefix before forwarding, so the target has to put it back:

    sudo tailscale serve reset
    sudo tailscale serve --bg --yes --https=8443 /home/muhan-li/job-hunt/client
    sudo tailscale serve --bg --yes --https=8443 --set-path=/api http://127.0.0.1:8788/api

Managing `tailscale serve` without sudo needs the operator flag once:
`sudo tailscale set --operator=$USER`. Use `https://`, not `http://`: the 8443 listener is TLS-only
and answers a plaintext request with "Client sent an HTTP request to an HTTPS server."

It runs as a user service, enabled, and linger is already on, so it starts at boot without a login:

    systemctl --user status  job-dashboard.service
    systemctl --user restart job-dashboard.service
    systemctl --user stop    job-dashboard.service
    journalctl --user -u job-dashboard.service -n 50

`server/jobs/table.py` renders the same shortlist as a PNG for Discord, which cannot render tables at all:

    /usr/bin/python3 -m server table --per-tier 3 --out /tmp/shortlist.png

**The POSTED column sorts by publish date.** Workday's board API publishes only a relative age
("Posted Today"), never an absolute date, so those dates are derived from `age_days` and the scan
timestamp and are accurate to about a day; they are drawn with a dotted underline. Rippling and
BambooHR publish no date anywhere, so those rows genuinely have none and always sort last.

`python3 -m server scan` runs the scan engine at `server/scan/`, one module per source. Import the module that owns
a function rather than reaching through the shim -- `scanner/filters.py` for `location_matches`, `scanner/pipeline.py`
for `score` and `sort_key`, `scanner/posting.py` for `make_posting` and `collapse_requisitions`.

`--source` takes an id from `data/config/sources.yaml`: `jobbank`, `newgrad_list`, `newgrad_json`, `ats_boards`,
`startups_gallery`, `linkedin`. A wrong id matches no source and exits clean with an empty report, which looks
like a quiet day. (`--source ats` is wrong; `--source ats_boards` is right.)

`server/jobs/recent.py` prints one posting per line, newest first, as `age | company | title | locations | url`,
so `grep`, `sort` and `awk` work on it. It reads the JSON feed directly, caches it in `data/state/` for ten minutes, and
does no scoring, no tiering and no ledger. Use it to check the feed by eye without running a scan.

## Rules the agent follows here

- Never submit an application without explicit approval for that batch.
- Never invent a fact about the candidate. A missing answer becomes a question to the user, marked UNKNOWN.
- Never answer a voluntary self-identification survey unless the candidate instructs it. When he does, record his
  instruction word for word.
- Never defeat a bot check: no CAPTCHA, no verification code. Hand that step to the candidate.
- Never type a password. Account creation and sign-in belong to the candidate.
- Never automate LinkedIn or Indeed as the candidate. The scan may read LinkedIn's public guest listing; applying
  there stays manual.
- US and out-of-town applications use the confirmed `answers_bank` values: US work authorization Yes, TN-eligible,
  no sponsorship needed; willing to relocate Yes. Use `located_in_listed_city_other` outside Metro Vancouver.
- **Outreach is COMPLETE as of 2026-10-04.** The candidate's instruction: "i've completed outreach and any drafts will remain
  unsent". Nothing is drafted, queued or sent, no pending draft is reported, and no
  follow-up is written unless he asks for that specific message. The 28 `sent` records stay as history.
- Outreach is email only. Send cold email only to an address the posting or the employer's own board publishes, or
  to a named individual whose address appears verbatim in a public source and that the agent verified. Never guess
  a pattern. Respect the volume cap.
- Log every outbound message in `data/state/outreach.jsonl`. No silent sends.
- **Always be on the lookout for new sources, and incorporate each one long term.** Meeting one source is not
  adopting one: adoption means an entry in `data/config/sources.yaml`, a row in `data/config/source_candidates.md`, a
  write-up in the skill, and — for any source that reveals employer boards — running `server/jobs/probe_ats.py` so those
  boards land in `data/config/ats_boards.yaml`. A feed that hands over boards is half-used until that probe runs:
  the startups.gallery feed sat on 42 unprobed boards holding 1,597 roles for two days, and another 14 had piled
  up by 2026-10-03, because the harvested set grows every time the feed meets a new company. The scan now says so
  itself — `[scan] N harvested boards are not in ats_boards.yaml -- run server/jobs/probe_ats.py` — and that line is an
  instruction, not a note.
