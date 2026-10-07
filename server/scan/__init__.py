"""The scan engine behind `python3 -m server scan`.

Split out of what was one 2,100-line script so that a pass can be extended or
re-run on its own -- the Workday sweep, the retry pass for boards that did not
answer -- without touching the daily scan.

    core       I/O and workspace paths            posting    posting identity
    parsing    dates and salaries                 pipeline   scoring and ranking
    filters    titles, levels, location tiers     harvest    board harvesting
    ledger     the seen ledger                    sources/   one module per source

`engine.py` is the CLI entry point; the server dispatcher calls `engine.main(argv)`.
"""
