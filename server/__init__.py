"""The job-hunt server.

This package is the Python half of the dashboard. It serves the static web client in
`client/` and exposes the JSON API that client calls:

    GET  /api/dashboard        everything the page shows, assembled per request
    GET  /api/posting/<id>     one posting's detail view (facts, cached text, brief state)
    POST /api/brief/<id>       start bin/posting_brief.py for one posting

The modules are split by what they touch, not by layer:

    config.py   paths, constants, and the two config reads (tiers, window)
    store.py    reading `state/` -- the feed, a row, a description, and the dashboard payload
    briefs.py   the brief cache and the one writer this server starts
    net.py      binding address (the Tailscale address, loopback)
    app.py      the HTTP handler and the static file server
    cli.py      argument parsing and the serving loop

Run it with `python3 -m server` from the workspace root.
"""
