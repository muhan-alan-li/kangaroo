#!/bin/sh
# Container entry point for the weekly Workday retry pass. The direct descendant
# of the retired ~/.hermes/scripts/job-retry.sh: same lock, same steps, same log.
#
# `server retry` re-checks only the Workday boards that did not answer and whose
# backoff has come up. After a clean retry it also prunes boards that are
# structurally dead (a Workday 404/422 -- a wrong career-site name no retry can
# fix) via `server prune`. Pruning is deliberately NOT "remove anything that
# failed": a throttled run fails every board at once, so only structural failures
# are removed, and only after a run that completed cleanly.
set -eu

# Share the scan's lock: never two Workday passes at once. A skipped run is not a
# failure -- the due boards stay due, and the next weekly slot picks them up.
exec 9>/work/state/.scan.lock
if ! flock -n 9; then
  echo "retry-entrypoint: a scan is running; skipping this slot" >&2
  exit 0
fi

LOG=/work/state/retry.log
if [ -f "$LOG" ] && [ "$(stat -c %s "$LOG" 2>/dev/null || echo 0)" -gt 2000000 ]; then
  mv -f "$LOG" "$LOG.1"
fi

# A manual `--dry-run` re-checks nothing and must not prune.
PRUNE=1
for arg in "$@"; do
  if [ "$arg" = "--dry-run" ]; then PRUNE=0; fi
done

rc=0
{
  echo "=== container-retry $(date -Is) ==="
  python3 -m server retry "$@" || rc=$?
  # Prune only after a retry that completed cleanly (rc 0), and never on a manual
  # --dry-run. `|| true` so a prune problem cannot mask the retry result.
  if [ "${rc}" -eq 0 ] && [ "${PRUNE}" -eq 1 ]; then
    python3 -m server prune || true
  fi
} >>"$LOG" 2>&1

# Exit 2 is the retry's deliberate refusal: the run lost every board it checked,
# which is the throttle and not a pile of dead configs, so it wrote nothing and the
# board state is unchanged. That is a handled outcome, not a broken job.
if [ "${rc:-0}" -eq 2 ]; then
  echo "retry-entrypoint: retry pass refused (Workday throttled); board state unchanged" >&2
  exit 0
fi
exit "${rc:-0}"
