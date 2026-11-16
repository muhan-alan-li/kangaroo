"""Board-list harvesters: `python3 -m server harvest workday|jsonboards|icims`.

Each harvester reads the Simplify feed and rewrites its vendor's board config
(`data/config/workday_boards.yaml`, `data/config/json_boards.yaml`, `data/config/icims_boards.yaml`).
"""
from __future__ import annotations

import sys

_MODULES = {
    "workday": ".workday",
    "jsonboards": ".jsonboards",
    "icims": ".icims",
}


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if not argv or argv[0] not in _MODULES:
        sys.stderr.write("usage: server harvest workday|jsonboards|icims [options]\n")
        return 2
    kind, rest = argv[0], argv[1:]
    import importlib

    module = importlib.import_module(_MODULES[kind], __package__)
    return module.main(rest)
