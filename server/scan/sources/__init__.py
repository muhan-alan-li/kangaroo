"""The source registry. Each source lives in its own module; this one maps
config/sources.yaml's `kind` to the function that reads it."""

from __future__ import annotations

from .ats import src_ats
from .gallery import src_startups_gallery
from .jobbank import src_jobbank
from .linkedin import src_linkedin
from .remotive import src_remotive
from .simplify import src_simplify, src_simplify_json
from .workatastartup import src_workatastartup


SOURCE_FUNCS = {
    "jobbank": src_jobbank,
    "simplify": src_simplify,
    "simplify_json": src_simplify_json,
    "ats": src_ats,
    "linkedin": src_linkedin,
    "remotive": src_remotive,
    "startups_gallery": src_startups_gallery,
    "workatastartup": src_workatastartup,
}
