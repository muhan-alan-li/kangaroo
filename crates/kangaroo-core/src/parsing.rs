use chrono::{DateTime, NaiveDate, Utc};
use regex::Regex;
use std::sync::OnceLock;

pub fn parse_date(text: &str) -> Option<NaiveDate> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let trimmed = text.trim().to_owned();
    let c = RE
        .get_or_init(|| Regex::new(r"(?i)^([a-z]+)\s+(\d{1,2}),\s*(\d{4})").unwrap())
        .captures(&trimmed)?;
    let month = match c.get(1)?.as_str().to_lowercase().as_str() {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    };
    NaiveDate::from_ymd_opt(c[3].parse().ok()?, month, c[2].parse().ok()?)
}
pub fn parse_age_days(text: &str, posted: Option<NaiveDate>) -> Option<i64> {
    if let Some(d) = posted {
        return Some((Utc::now().date_naive() - d).num_days().max(0));
    }
    static RE: OnceLock<Regex> = OnceLock::new();
    let lowered = text.trim().to_lowercase();
    let c = RE
        .get_or_init(|| Regex::new(r"^(\d+)\s*(h|d|w|mo|m|y)$").unwrap())
        .captures(&lowered)?;
    let n: i64 = c[1].parse().ok()?;
    Some(match &c[2] {
        "h" => 0,
        "d" => n,
        "w" => n * 7,
        "m" | "mo" => n * 30,
        "y" => n * 365,
        _ => return None,
    })
}
pub fn parse_salary(text: Option<&str>) -> Option<(f64, f64, bool)> {
    let text = text?;
    if text.is_empty() {
        return None;
    }
    static AMT: OnceLock<Regex> = OnceLock::new();
    let re = AMT.get_or_init(|| Regex::new(r"\$\s*([\d,]+(?:\.\d+)?)").unwrap());
    let amounts: Vec<f64> = re
        .captures_iter(text)
        .filter_map(|c| c[1].replace(',', "").parse().ok())
        .collect();
    if amounts.is_empty() {
        return None;
    }
    let low = text.to_lowercase();
    let periods = [
        ("hourly", 2000.0),
        ("per hour", 2000.0),
        ("an hour", 2000.0),
        ("/hr", 2000.0),
        ("annually", 1.0),
        ("yearly", 1.0),
        ("per year", 1.0),
        ("a year", 1.0),
        ("/yr", 1.0),
        ("monthly", 12.0),
        ("per month", 12.0),
        ("biweekly", 26.0),
        ("bi-weekly", 26.0),
        ("weekly", 52.0),
    ];
    let mult = periods
        .iter()
        .find(|(p, _)| low.contains(p))
        .map(|x| x.1)
        .or_else(|| (amounts.iter().copied().fold(0.0, f64::max) >= 10000.0).then_some(1.0))?;
    let lo = amounts.iter().copied().fold(f64::INFINITY, f64::min) * mult;
    let hi = amounts.iter().copied().fold(0.0, f64::max) * mult;
    Some((lo, hi, mult != 1.0))
}
pub fn strip_tags(fragment: &str) -> String {
    let tag = Regex::new(r"(?s)<[^>]*>").unwrap();
    let no = tag.replace_all(fragment, " ");
    let entities = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
    ];
    let mut s = no.into_owned();
    for (a, b) in entities {
        s = s.replace(a, b)
    }
    Regex::new(r"\s+")
        .unwrap()
        .replace_all(s.trim(), " ")
        .into_owned()
}
pub fn parse_iso_datetime(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}
