#!/bin/sh
# Container entry point for the scheduled job scan. This is the direct
# descendant of the retired ~/.hermes/scripts/job-scan-store.sh: same three
# steps (sweep-if-stale, scan, mark-reported), same lock, same log.
#
# It runs INSIDE the container, where /work is the workspace root assembled
# from the compose mounts (server/, config/, candidate/ read-only; state/
# read-write). The only deliberate difference from the old wrapper is DNS: the
# compose file points the container at public resolvers instead of Pi-hole, so
# this run's few thousand lookups never touch the home network's rate limit.
set -eu

# Serialise with every other scan -- a manual host run, a retry pass, another
# slot. flock on the bind-mounted state dir locks the same file server/scan/engine.py
# takes on a host-side manual run, so container and host runs exclude each other.
exec 9>/work/state/.scan.lock
if ! flock -n 9; then
  echo "scan-entrypoint: another scan holds the lock; skipping this slot" >&2
  exit 0
fi

# The scan itself would try to take the same lock next -- flock is per open file
# description, so its fresh open would fail against this shell's lock even though
# it is the same process tree. This says "the lock is already held for you".
export JOB_SCAN_LOCK_HELD=1

LOG=/work/state/scan.log
# Keep the log bounded. It exists for the morning a run looks wrong; nobody reads a year of it.
if [ -f "$LOG" ] && [ "$(stat -c %s "$LOG" 2>/dev/null || echo 0)" -gt 2000000 ]; then
  mv -f "$LOG" "$LOG.1"
fi

# Workday is ~1,300 boards and the scan's slowest source. Its sweep cache is only
# written by `server sweep`, so without this the scan sweeps the whole fleet inline
# on every run. Sweep only when the cache is stale (26h, inside the scan's own 30h
# window); a failed sweep is not fatal, because the scan falls back to sweeping inline.
SWEEP=/work/state/workday_sweep.json
if [ ! -f "$SWEEP" ] || [ -n "$(find "$SWEEP" -mmin +1560 2>/dev/null)" ]; then
  echo "[scan-entrypoint] workday cache is stale; sweeping before the scan" >&2
  python3 -m server sweep >>"$LOG" 2>&1 \
    || echo "[scan-entrypoint] workday sweep failed; the scan will sweep it inline" >&2
fi

{
  echo "=== container-scan $(date -Is) ==="
  python3 -m server scan --per-tier 5 "$@"
  # The ledger's `reported` flag is what keeps the next run's "new" set small. The portal
  # already holds every posting from this run, so marking them all reported loses nothing.
  # It must run only after the scan has actually written last_scan.json and the feed, which
  # is why it is inside the same `set -e` block -- a failed scan exits before it.
  python3 -m server mark-reported --all-unreported
} >>"$LOG" 2>&1

exit 0
