# Archive

The Python workspace, kept for reference. Branch `main` still runs it in production.

Nothing on `rust-app` reads these folders. No `make` target, no crate and no test points here.

| Folder | Was |
|---|---|
| `server/` | The Python server: `api/` (HTTP), `scan/` (engine and one module per source), `brief/` (brief engine), `jobs/` (timers and utilities), `tests/` |
| `client/` | The Preact and TypeScript dashboard. `dist/` is committed |
| `docker/` | The scan and retry image, with `entrypoint.sh` and `entrypoint-retry.sh` |
| `systemd/` | The two user timers, `job-scan` and `job-retry` |

## Run it again

Check out `main`, or move the folders back to the repository root. The Python paths in
`server/` and the settings in `docker/compose.yaml` assume that root.

Then:

```sh
/usr/bin/python3 -m server            # dashboard and API on :8787
/usr/bin/python3 -m server scan --per-tier 5
```

Always write `/usr/bin/python3`. Inside an agent session a bare `python3` can resolve to a
toolchain with no PyYAML, and the scan then finds nothing.
