"""The job-hunt server: the dashboard API plus the scan and research engines.

One entry point, `python3 -m server <command>`. With no command it serves the dashboard;
every other command runs one job. See `cli.py` for the command table. The package is split
by role:

    api/      the dashboard HTTP service
              app.py     routes: dashboard read, posting detail, brief/contact
                         launches, and the applied list's two writes
              store.py   reading `data/state/` -- feed, row, description, dashboard payload
              triage.py  the applied list (`data/state/applied.json`)
              briefs.py  the brief cache and the launcher for one generation
              net.py     bind addresses (Tailscale address, loopback)
              serve.py   the serving loop
    scan/     the scan engine -- core, parsing, filters, ledger, posting, pipeline,
              harvest, boardstate, portal, keywords, and sources/ (one module per feed).
              engine.py is the `scan` command. Adding a source means a module in
              sources/, a row in SOURCE_FUNCS, a block in data/config/sources.yaml, and
              a row in data/config/source_candidates.md.
    brief/    the on-demand brief engine -- run the `brief` command. Split by stage:
              stages/chain (the default three-stage DeepSeek chain), validate (the
              mechanical edit checks), agent (the older Hermes path and its review),
              contacts (the on-demand deep search), prompts, posting (inputs), state
              (the files under data/state/briefs/), and llm (the chat client).
    jobs/     the scheduled and maintenance jobs behind the rest of the commands:
              scan/ owns the scan job's scripts (sweep, mark-reported), retry/
              owns the retry job's (retry, prune), harvest/ owns the board harvesters;
              the rest are hand-run utilities (refresh, probe-ats, table, feed, bump,
              resume-text).
    config.py shared paths and constants
    fsutil.py the one atomic-write helper every state writer uses
    tests/    the four smoke tests

Run it from the workspace root.
"""
