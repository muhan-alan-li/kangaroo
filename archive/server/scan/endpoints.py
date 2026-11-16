"""Board-list URL templates, in one place.

The probe (`server/jobs/probe_ats.py`), the ATS source (`sources/ats.py`) and
the JSON-vendor source (`sources/jsonboards.py`) all build the same board URLs;
three copies drifted silently before (the probe's table, inline f-strings in the
source, the vendor table). The per-job detail URLs in `scan/portal.py` are a
different shape and stay where they are.
"""
from __future__ import annotations

BOARD_URL = {
    "greenhouse": "https://boards-api.greenhouse.io/v1/boards/{slug}/jobs",
    "ashby": "https://api.ashbyhq.com/posting-api/job-board/{slug}",
    "lever": "https://api.lever.co/v0/postings/{slug}?mode=json",
    "workable": "https://apply.workable.com/api/v1/widget/accounts/{slug}?details=true",
}
