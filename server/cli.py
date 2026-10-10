"""The one entry point: `python3 -m server <command>`.

With no subcommand (or just flags) it serves the dashboard API, which is what systemd and
local dev use. Every other command runs one job against the workspace. Keeping them here
means the pile of top-level scripts goes away and there is a single, discoverable surface:

    python3 -m server                      # serve the API + client (dev)
    python3 -m server --api-only           # serve the API only (production)
    python3 -m server scan [--per-tier 5]  # one scan
    python3 -m server sweep                # sweep the Workday boards
    python3 -m server retry                # retry the boards that failed
    python3 -m server prune                # drop structurally-dead boards
    python3 -m server refresh              # rebuild the portal feed
    python3 -m server mark-reported …      # mark the run's postings reported
    python3 -m server harvest workday|jsonboards|icims
    python3 -m server probe-ats            # test ATS slugs
    python3 -m server brief <id>           # generate a posting brief
    python3 -m server table [--per-tier 3] # render the shortlist to a PNG
    python3 -m server feed [--days 7]      # print the Simplify feed
    python3 -m server bump --bump          # bump the client cache-buster
    python3 -m server resume-text          # regenerate data/candidate/resume-text.md

Subcommands are registered in `COMMANDS` below and imported lazily, so starting the server
never pays for the scanner's dependencies.
"""
from __future__ import annotations

import sys

# name -> (module, attribute). Imported on first use.
COMMANDS = {
    "scan": ("server.scan.engine", "main"),
    "sweep": ("server.jobs.scan.sweep", "main"),
    "retry": ("server.jobs.retry.retry", "main"),
    "prune": ("server.jobs.retry.prune", "main"),
    "refresh": ("server.jobs.refresh_portal", "main"),
    "mark-reported": ("server.jobs.scan.mark_reported", "main"),
    "harvest": ("server.jobs.harvest", "main"),
    "probe-ats": ("server.jobs.probe_ats", "main"),
    "brief": ("server.brief.generate", "main"),
    "table": ("server.jobs.table", "main"),
    "feed": ("server.jobs.recent", "main"),
    "bump": ("server.jobs.bump", "main"),
    "resume-text": ("server.jobs.resume_text", "main"),
}


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)

    # No subcommand, or a leading flag, means "serve".
    if not argv or argv[0].startswith("-"):
        from .api.serve import main as serve
        return serve(argv)

    command, rest = argv[0], argv[1:]
    if command == "serve":
        from .api.serve import main as serve
        return serve(rest)

    target = COMMANDS.get(command)
    if not target:
        sys.stderr.write(
            f"server: unknown command {command!r}\n"
            f"  commands: serve, {', '.join(sorted(COMMANDS))}\n")
        return 2
    module_name, attr = target
    import importlib
    module = importlib.import_module(module_name)
    return getattr(module, attr)(rest)


if __name__ == "__main__":
    raise SystemExit(main())
