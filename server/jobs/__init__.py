"""Runnable jobs: the scheduled scan and retry passes plus the maintenance utilities.

`scan/` owns the scan job's scripts, `retry/` the retry job's, `harvest/` the
board harvesters; the rest are hand-run utilities. Everything here is invoked
through `python3 -m server <command>` (see `server/cli.py`), never as a
standalone script.
"""
