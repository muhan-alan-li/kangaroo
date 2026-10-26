"""The job-hunt server: the dashboard API plus the scan and research engines.

One entry point, `python3 -m server <command>`. With no command it serves the dashboard;
every other command runs one job. See `cli.py` for the command table. The package is split
by role:

    api/      the dashboard HTTP service
              app.py     routes (GET /api/dashboard, GET /api/posting/<id>,
                         POST /api/brief/<id>, POST /api/deep-contacts/<id>)
              store.py   reading `data/state/` -- feed, row, description, dashboard payload
              briefs.py  the brief cache and the coroutine that launches generation
              net.py     bind addresses (Tailscale address, loopback)
              serve.py   the serving loop
    scan/     the scan engine -- core, parsing, filters, ledger, posting, pipeline,
              harvest, boardstate, portal, keywords, and sources/ (one module per feed).
              engine.py is the `scan` command.
    brief/    the on-demand research engine -- generate (the `brief` command), the
              three-stage tailoring chain, the DeepSeek client, and the Hermes
              agent path behind `--deep-contacts`
    jobs/     the scheduled and maintenance jobs behind the rest of the commands:
              scan/ owns the scan job's scripts (sweep, mark-reported), retry/
              owns the retry job's (retry, prune); the rest are hand-run
              utilities (refresh, probe-ats, harvest, table, feed, bump,
              resume-text)
    config.py shared paths and constants
    tests/    the two smoke tests

Run it from the workspace root.
"""
