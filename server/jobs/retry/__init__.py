"""Scripts owned by the weekly retry job.

The job-retry container runs `server retry`, then `server prune` after a clean
pass (`docker/entrypoint-retry.sh`); both scripts live here. Shared scan logic
(board state, Workday fetching) stays in `server/scan/` -- this package owns
the job's scripts, not the engine.
"""
