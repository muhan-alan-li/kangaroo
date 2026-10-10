#!/usr/bin/env python3
"""Render a scan's shortlist as a PNG table.

Discord does not render markdown tables -- pipes arrive as literal characters -- and a
monospace code block only stays aligned until the width forces a horizontal scroll, which a
140-character row does on a phone. An image arrives identical on every client.

    /usr/bin/python3 -m server table                    # latest scan, top 3 per tier
    /usr/bin/python3 -m server table --per-tier 5 --out /tmp/shortlist.png

Reads state/last_scan.json, so it renders what the scan actually reported: no network, no
filters re-run, no ledger touched, and no text invented here.
"""
from __future__ import annotations

import argparse
import json
import os
import sys

from PIL import Image, ImageDraw, ImageFont

from ..config import ROOT

# Discord's dark palette, so the image sits on the client without a visible seam.
BG = (30, 31, 34)
BG_ALT = (35, 36, 40)
HEAD = (181, 186, 193)
FG = (219, 222, 225)
MUTED = (148, 155, 164)
RULE = (60, 62, 68)

TIER = {
    "vancouver": (87, 242, 135),
    "remote_canada": (89, 214, 190),
    "toronto": (88, 166, 255),
    "seattle": (155, 145, 255),
    "bay_area": (235, 130, 235),
    "remote_us": (250, 168, 26),
    "remote_unspecified": (160, 166, 175),
}
FLAG = {"LEVEL-II": (250, 168, 26), "BELOW-FLOOR": (242, 63, 66), "US-auth": (148, 155, 164)}

FONT_DIR = "/usr/share/fonts/truetype/dejavu"
PAD = 20
ROW_H = 34
GAP = 22
SCALE = 2          # draw at 2x so the text stays crisp when Discord scales it

# name -> (file, size in points)
FACES = {
    "title": ("DejaVuSans-Bold.ttf", 19),
    "sub": ("DejaVuSans.ttf", 13),
    "head": ("DejaVuSans-Bold.ttf", 12),
    "cell": ("DejaVuSans.ttf", 14),
    "mono": ("DejaVuSansMono.ttf", 14),
}


def load_fonts():
    """Every face once, at its final pixel size. Re-creating a font per cell is slow and
    gets the size wrong."""
    out = {}
    for key, (name, size) in FACES.items():
        path = os.path.join(FONT_DIR, name)
        try:
            out[key] = ImageFont.truetype(path, size * SCALE)
        except OSError:
            out[key] = ImageFont.load_default()
    return out


def clip(draw, text, fnt, width):
    """Trim to a pixel width, with an ellipsis, so no column ever overruns."""
    text = " ".join(str(text or "").split())
    if draw.textlength(text, font=fnt) <= width * SCALE:
        return text
    while text and draw.textlength(text + "…", font=fnt) > width * SCALE:
        text = text[:-1]
    return text + "…"


def load_rows(path, per_tier, limit):
    """The ranked shortlist, ordered the way the scan ranks it.

    `matched` in last_scan.json is in discovery order, not rank order -- the scan sorts only
    when it prints. Taking the first N per tier from it therefore picks arbitrary postings,
    not the best ones, so the rank has to be reapplied here: tier order from filters.yaml,
    then score, then age.
    """
    with open(path, encoding="utf-8") as fh:
        data = json.load(fh)

    filters_path = os.path.join(ROOT, "config", "filters.yaml")
    tiers = []
    try:
        import yaml
        with open(filters_path, encoding="utf-8") as fh:
            tiers = [str(t.get("name")) for t in (yaml.safe_load(fh) or {}).get("location_tiers") or []]
    except Exception:  # noqa: BLE001 -- ranking still works without the tier order
        pass
    rank = {name: i for i, name in enumerate(tiers)}

    def key(p):
        age = p.get("age_days")
        return (rank.get(p.get("tier") or "untiered", len(rank) + 1),
                -int(p.get("score") or 0),
                age if age is not None else 999)

    matched = sorted(data.get("matched") or [], key=key)
    if per_tier:
        counts, kept = {}, []
        for p in matched:
            tier = p.get("tier") or "untiered"
            if counts.get(tier, 0) >= per_tier:
                continue
            counts[tier] = counts.get(tier, 0) + 1
            kept.append(p)
        matched = kept
    if limit:
        matched = matched[:limit]
    return data, matched


def render(data, rows, out):
    F = load_fonts()

    cols = [
        ("#", 30, "mono"),
        ("SCORE", 50, "mono"),
        ("AGE", 38, "mono"),
        ("TIER", 152, "cell"),
        ("COMPANY", 185, "cell"),
        ("TITLE", 430, "cell"),
        ("LOCATION", 245, "cell"),
        ("FLAGS", 150, "cell"),
    ]

    probe = ImageDraw.Draw(Image.new("RGB", (10, 10)))
    table_w = sum(c[1] for c in cols) + GAP * (len(cols) - 1)
    width = table_w + PAD * 2
    header_block = 96
    height = header_block + ROW_H * (len(rows) + 1) + 24

    img = Image.new("RGB", (width * SCALE, height * SCALE), BG)
    d = ImageDraw.Draw(img)

    def S(v):
        return int(v * SCALE)

    ts = (data.get("ran_at") or "")[:16].replace("T", " ")
    counts = data.get("tier_counts") or {}
    sub = (f"{ts}Z   ·   {len(data.get('matched') or [])} matched   ·   "
           f"showing {len(rows)}   ·   "
           + "  ".join(f"{k} {v}" for k, v in counts.items() if v))

    d.text((S(PAD), S(20)), "Job scan — shortlist", font=F["title"], fill=FG)
    d.text((S(PAD), S(54)), sub, font=F["sub"], fill=MUTED)

    x, y = PAD, header_block
    for name, w, _ in cols:
        d.text((S(x), S(y)), name, font=F["head"], fill=HEAD)
        x += w + GAP
    d.line([(S(PAD - 6), S(y + 22)), (S(x - GAP + 6), S(y + 22))],
           fill=RULE, width=SCALE)

    for i, p in enumerate(rows):
        ry = y + 32 + i * ROW_H
        if i % 2:
            d.rectangle([S(PAD - 8), S(ry - 7), S(x - GAP + 8), S(ry + ROW_H - 11)],
                        fill=BG_ALT)
        tier = p.get("tier") or "untiered"
        age = p.get("age_days")
        flags = ([("LEVEL-II", "LEVEL-II")] if p.get("level_ii") else []) \
            + ([("US-auth", "US-auth")] if p.get("requires_us_work_auth") else []) \
            + ([("BELOW-FLOOR", "BELOW-FLOOR")] if p.get("below_comp_floor") else [])
        cells = [
            (str(i + 1), "mono", MUTED),
            (str(p.get("score", "")), "mono", FG),
            (f"{age}d" if age is not None else "?", "mono", MUTED),
            (tier, "cell", TIER.get(tier, MUTED)),
            (p.get("company") or "", "cell", FG),
            (p.get("title") or "", "cell", FG),
            (p.get("location") or "", "cell", MUTED),
            (",".join(f[0] for f in flags), "cell",
             FLAG.get(flags[0][1], MUTED) if flags else MUTED),
        ]
        cx = PAD
        for (text, fkey, colour), (_n, w, _f) in zip(cells, cols):
            d.text((S(cx), S(ry)), clip(probe, text, F[fkey], w), font=F[fkey], fill=colour)
            cx += w + GAP

    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    img.save(out, "PNG", optimize=True)
    return img.size, out


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="src", default=os.path.join(ROOT, "state", "last_scan.json"))
    ap.add_argument("--per-tier", type=int, default=3)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--out", default="/tmp/scan-shortlist.png")
    args = ap.parse_args(argv)

    data, rows = load_rows(args.src, args.per_tier, args.limit)
    if not rows:
        sys.stderr.write("nothing to render\n")
        return 1
    size, out = render(data, rows, args.out)
    print(f"{out}  {size[0]}x{size[1]}  {len(rows)} rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
