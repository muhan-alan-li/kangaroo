"""Remotive's global remote board. Off by default."""

from __future__ import annotations

import json

from ..core import http_get
from ..posting import make_posting


def src_remotive(cfg, filters):
    data = http_get(cfg.get("url", ""))
    if not data:
        return []
    try:
        jobs = json.loads(data).get("jobs", [])
    except json.JSONDecodeError:
        return []
    out = []
    for job in jobs:
        out.append(make_posting(
            source="remotive", company=job.get("company_name", ""),
            title=job.get("title", ""), location=job.get("candidate_required_location", ""),
            url=job.get("url"), age_days=None, raw_id=str(job.get("id")),
        ))
    return out
