"""The one atomic-write helper every state writer shares.

The dashboard and the scanners read `data/state/` while jobs write it. A reader must never see
a half-written file, so every writer goes through `atomic_write`: a pid-tagged temp file in the
same directory, then `os.replace`. The pid in the temp name matters because two passes can
overlap (two overdue timer slots fire within the same second), and a shared temp path would let
one process rename a file while the other is still filling it.

This module deliberately has no imports from `server.scan`: the brief generator needs the same
guarantee without loading the scanner's network governor or PyYAML.
"""
from __future__ import annotations

import contextlib
import os


@contextlib.contextmanager
def atomic_write(path):
    """Yield a file handle for an atomic write.

    Callers own their `makedirs` and their serialization; this owns only the temp dance.
    """
    tmp = f"{path}.{os.getpid()}.tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        yield fh
    os.replace(tmp, path)
