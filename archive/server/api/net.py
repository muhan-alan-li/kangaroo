"""Where the server binds.

The Tailscale address by default, never 0.0.0.0: reachable from every device on the tailnet
and from nowhere on the LAN, which is the whole point of having the tailnet.
"""
from __future__ import annotations

import subprocess
import time


def tailnet_ip(wait_seconds=0):
    """The address to bind. Falls back to loopback, which is the safe direction to fail.

    At boot the service can start before tailscaled has an address, and binding loopback then
    would leave the dashboard unreachable from the tailnet for as long as the process lives.
    So it waits for the address rather than settling for the fallback.
    """
    deadline = time.monotonic() + max(0, wait_seconds)
    while True:
        try:
            out = subprocess.run(["tailscale", "ip", "-4"], capture_output=True, text=True,
                                 timeout=10).stdout.strip().splitlines()
            if out and out[0].strip():
                return out[0].strip()
        except Exception:  # noqa: BLE001 -- no tailscale is not a reason to fail
            pass
        if time.monotonic() >= deadline:
            return "127.0.0.1"
        time.sleep(2)
