"""The scan engine behind `python3 -m server scan`.

Split out of what was one 2,100-line script so that a pass can be extended or
re-run on its own -- the Workday sweep, the retry pass for boards that did not
answer -- without touching the daily scan.

    core       I/O and workspace paths            posting    posting identity
    parsing    dates and salaries                 pipeline   scoring and ranking
    filters    titles, levels, location tiers     harvest    board harvesting
    keywords   terms and posting emails           portal     the dashboard's feed
    ledger     the seen ledger                    boardstate per-board health/backoff
    net        egress guard (DNS, sockets)        sources/   one module per source

`engine.py` is the CLI entry point; the server dispatcher calls `engine.main(argv)`.
`icims_fetch.py` is the one exception: it runs under `.venv-icims` because iCIMS needs
browser TLS impersonation, and `sources/icims.py` drives it as a subprocess.
"""
