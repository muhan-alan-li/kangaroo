"""Date and salary parsing. Sources publish dates in several shapes and a
wrong guess either loses a fresh posting or resurrects an old one, so the rules
live in one place."""

from __future__ import annotations

from datetime import datetime, timezone
import re


MONTHS = {m: i + 1 for i, m in enumerate(
    ["january", "february", "march", "april", "may", "june", "july", "august",
     "september", "october", "november", "december"])}


# Sources write dates both ways: "September 18, 2026" and "Sep 28, 2026".
MONTHS_ABBR = {name[:3]: number for name, number in MONTHS.items()}


def parse_age_days(text, posted=None):
    if posted:
        return max(0, (datetime.now(timezone.utc).date() - posted).days)
    t = (text or "").strip().lower()
    m = re.match(r"^(\d+)\s*(h|d|w|mo|m|y)$", t)
    if not m:
        return None
    n, unit = int(m.group(1)), m.group(2)
    return {"h": 0, "d": n, "w": n * 7, "m": n * 30, "mo": n * 30, "y": n * 365}[unit]


def parse_date(text):
    """'September 18, 2026' or 'Sep 28, 2026' -> date, else None."""
    m = re.match(r"([A-Za-z]+)\s+(\d{1,2}),\s*(\d{4})", (text or "").strip())
    if not m:
        return None
    word = m.group(1).lower()
    month = MONTHS.get(word) or MONTHS_ABBR.get(word[:3])
    if not month:
        return None
    try:
        return datetime(int(m.group(3)), month, int(m.group(2)), tzinfo=timezone.utc).date()
    except ValueError:
        return None


# Annualisation factors. 2000 hours is the standard full-time year; using it makes an
# hourly rate comparable to an annual band, at the cost of precision we flag in output.
SALARY_PERIODS = [
    ("hourly", 2000), ("per hour", 2000), ("an hour", 2000), ("/hr", 2000),
    ("annually", 1), ("yearly", 1), ("per year", 1), ("a year", 1), ("/yr", 1),
    ("monthly", 12), ("per month", 12),
    ("biweekly", 26), ("bi-weekly", 26), ("weekly", 52),
]


def parse_salary(text):
    """Salary string -> (annual_min, annual_max, is_estimated) or None.

    Job Bank gives us "$50.00 to $70.00 hourly" and "$130,000.00 to $150,000.00
    annually" in the same field, so the rate has to be annualised before it can be
    compared against a floor. Hourly and weekly figures are annualised and flagged
    estimated; anything with no stated period is treated as annual only if the
    numbers are annual-sized, and dropped otherwise rather than guessed at.
    """
    if not text:
        return None
    amounts = [float(a.replace(",", "")) for a in re.findall(r"\$\s*([\d,]+(?:\.\d+)?)", text)]
    if not amounts:
        return None
    low = text.lower()
    multiplier, stated = None, False
    for key, factor in SALARY_PERIODS:
        if key in low:
            multiplier, stated = factor, True
            break
    if not stated:
        if max(amounts) < 10000:
            return None          # too small to be an annual figure, period unknown
        multiplier = 1
    lo, hi = min(amounts) * multiplier, max(amounts) * multiplier
    return (lo, hi, multiplier != 1)
