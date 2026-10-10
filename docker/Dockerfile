# Job-scan image: the interpreter the scheduled scan runs under.
#
# WHY THIS EXISTS. The scan asks the local resolver a few thousand questions in a
# burst (one per ATS board), and Pi-hole answers REFUSED past 1000 queries in 60s
# per client -- which once cost a whole run all ~1,300 Workday boards. This image
# changes nothing about the scan code: it only moves the run somewhere its
# resolver is not Pi-hole (see `dns:` in compose.yaml), so automation traffic and
# the home network stop sharing one rate-limit bucket.
#
# WHAT IS BAKED IN vs MOUNTED. Only the interpreter, the OS packages the
# entrypoint needs (tzdata, so `date -Is` logs in America/Vancouver like the old
# wrapper did), and the scan's Python dependencies. Code (server/), config and
# the candidate profile are bind-mounted read-only at run time, so editing them
# needs no rebuild; state/ is mounted read-write, so the ledger, the portal feed
# and the logs land on the host where the dashboard reads them. Rebuild only when
# THIS file changes:
#
#     docker compose -f docker/compose.yaml build
FROM python:3.13-slim

ENV PYTHONUNBUFFERED=1 TZ=America/Vancouver

# tzdata: the entrypoint stamps state/scan.log with `date -Is`, and the old
# wrapper logged local time; without this the container logs UTC instead.
# pyyaml: the scanner's one hard dependency (config + candidate profile).
# curl_cffi: the iCIMS fetcher's browser impersonation (server/scan/icims_fetch.py).
# pillow: only for `python -m server table` (the Discord PNG), not the scan itself.
RUN apt-get update \
 && apt-get install -y --no-install-recommends tzdata \
 && rm -rf /var/lib/apt/lists/* \
 && pip install --no-cache-dir pyyaml curl_cffi pillow

COPY docker/entrypoint.sh /usr/local/bin/scan-entrypoint
COPY docker/entrypoint-retry.sh /usr/local/bin/retry-entrypoint
RUN chmod +x /usr/local/bin/scan-entrypoint /usr/local/bin/retry-entrypoint

WORKDIR /work

ENTRYPOINT ["/usr/local/bin/scan-entrypoint"]
