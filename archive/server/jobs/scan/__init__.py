"""Scripts owned by the scheduled scan job.

The job-scan container runs sweep-if-stale, `server scan`, mark-reported
(`docker/entrypoint.sh`); the two Python scripts in that sequence that live
outside the scan engine itself live here. Shared scan logic (board state,
ledger, sources) stays in `server/scan/` -- this package owns the job's
scripts, not the engine.
"""
