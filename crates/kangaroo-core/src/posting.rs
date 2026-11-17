use regex::Regex;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use std::sync::OnceLock;

fn patterns() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            "greenhouse",
            r#"(?i)^https?://(?:job-boards|boards)\.greenhouse\.io/([^/\"'?#]+)/jobs/(\d+)"#,
            "ats|{id}",
        ),
        (
            "ashby",
            r#"(?i)^https?://jobs\.ashbyhq\.com/([^/\"'?#]+)/([0-9a-fA-F-]{16,})"#,
            "ats|{id}",
        ),
        (
            "lever",
            r#"(?i)^https?://jobs\.lever\.co/([^/\"'?#]+)/([0-9a-fA-F-]{16,})"#,
            "ats|{id}",
        ),
        (
            "workday",
            r#"(?i)^https?://[a-z0-9-]+\.wd\d+\.myworkdayjobs\.com/(?:[a-z]{2}-[A-Z]{2}/)?([^/\"'?#]+)/job/(?:.+?/)?([^/\"'?#]+)(?:[?#].*)?$"#,
            "ats|{id}",
        ),
        (
            "workday",
            r#"(?i)^https?://wd\d+\.myworkdaysite\.com/(?:[a-z]{2}-[A-Z]{2}/)?recruiting/[^/\"'?#]+/([^/\"'?#]+)/job/(?:.+?/)?([^/\"'?#]+)(?:[?#].*)?$"#,
            "ats|{id}",
        ),
        (
            "workable",
            r#"(?i)^https?://apply\.workable\.com/([^/\"'?#]+)/j/([0-9A-Fa-f]{6,})"#,
            "ats|workable:{slug}:{id}",
        ),
        (
            "smartrecruiters",
            r#"(?i)^https?://jobs\.smartrecruiters\.com/([^/\"'?#]+)/([0-9]+)"#,
            "ats|smartrecruiters:{slug}:{id}",
        ),
        (
            "rippling",
            r#"(?i)^https?://ats\.rippling\.com/([^/\"'?#]+)/jobs/([0-9a-fA-F-]{16,})"#,
            "ats|rippling:{slug}:{id}",
        ),
        (
            "bamboohr",
            r#"(?i)^https?://([a-z0-9-]+)\.bamboohr\.com/careers/(\d+)"#,
            "ats|bamboohr:{slug}:{id}",
        ),
        (
            "recruitee",
            r#"(?i)^https?://([a-z0-9-]+)\.recruitee\.com/o/([^/\"'?#]+)"#,
            "ats|recruitee:{slug}:{id}",
        ),
        (
            "breezy",
            r#"(?i)^https?://([a-z0-9-]+)\.breezy\.hr/p/([^/\"'?#]+)"#,
            "ats|breezy:{slug}:{id}",
        ),
        (
            "teamtailor",
            r#"(?i)^https?://([a-z0-9-]+)\.teamtailor\.com/jobs/(\d+)"#,
            "ats|teamtailor:{slug}:{id}",
        ),
        (
            "icims",
            r#"(?i)^https?://([a-z0-9-]+)\.icims\.com/jobs/(\d+)"#,
            "ats|icims:{slug}:{id}",
        ),
    ]
}
static REGEXES: OnceLock<Vec<Regex>> = OnceLock::new();
pub fn apply_signature(url: &str) -> Option<(String, String, String, String)> {
    let regs = REGEXES.get_or_init(|| {
        patterns()
            .iter()
            .map(|p| Regex::new(p.1).unwrap())
            .collect()
    });
    for ((vendor, _, template), re) in patterns().iter().zip(regs) {
        if let Some(c) = re.captures(url) {
            let slug = c.get(1)?.as_str().to_owned();
            let id = c.get(2)?.as_str().to_owned();
            let key = template
                .replace("{vendor}", vendor)
                .replace("{slug}", &slug.to_lowercase())
                .replace("{id}", &id);
            return Some(((*vendor).into(), slug, id, key));
        }
    }
    None
}

pub const SOURCE_PREFERENCE: &[(&str, i32)] = &[
    ("ats", 0),
    ("newgrad_json", 1),
    ("startups_gallery", 1),
    ("newgrad_list", 2),
    ("remotive", 2),
    ("jobbank", 2),
    ("linkedin", 3),
];
pub fn collapse_requisitions(postings: Vec<Value>) -> Vec<Value> {
    let mut order: Vec<String> = Vec::new();
    let mut kept: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let mut keys: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for p in postings {
        let id = p.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        let key = p
            .get("url")
            .and_then(Value::as_str)
            .and_then(apply_signature)
            .map(|x| x.3);
        let Some(key) = key else {
            if let std::collections::hash_map::Entry::Vacant(entry) = kept.entry(id.clone()) {
                order.push(id);
                let _ = entry.insert(p);
            }
            continue;
        };
        if let Some(prior_id) = keys.get(&key).cloned() {
            let src = p.get("source").and_then(Value::as_str).unwrap_or("");
            let old = kept
                .get(&prior_id)
                .and_then(|v| v.get("source"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let rank = |s: &str| {
                SOURCE_PREFERENCE
                    .iter()
                    .find(|x| x.0 == s)
                    .map(|x| x.1)
                    .unwrap_or(5)
            };
            if rank(src) < rank(old) {
                let _ = kept.remove(&prior_id);
                order.retain(|existing| existing != &prior_id);
                if !kept.contains_key(&id) {
                    order.push(id.clone());
                }
                let _ = kept.insert(id.clone(), p);
                let _ = keys.insert(key, id);
            }
        } else {
            let _ = keys.insert(key, id.clone());
            if !kept.contains_key(&id) {
                order.push(id.clone());
            }
            let _ = kept.entry(id).or_insert(p);
        }
    }
    order
        .into_iter()
        .filter_map(|id| kept.remove(&id))
        .collect()
}
// Mirrors Python's make_posting fixture contract, whose fields are independent inputs.
#[allow(clippy::too_many_arguments)]
pub fn make_posting(
    source: &str,
    company: &str,
    title: &str,
    location: &str,
    url: Option<&str>,
    posted: Option<&str>,
    age_days: Option<i64>,
    salary: Option<&str>,
    ats: Option<&str>,
    raw_id: Option<&str>,
    extra: Option<&Value>,
    dedupe_key: Option<&str>,
) -> Value {
    let key = dedupe_key
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            raw_id
                .filter(|id| !id.is_empty())
                .map(|id| format!("{source}|{id}"))
        })
        .unwrap_or_else(|| format!("{company}|{title}|{location}").to_lowercase());
    let id = format!("{:x}", Sha1::digest(key.as_bytes()));
    let mut m = Map::new();
    let _ = m.insert("id".into(), Value::String(id[..16].into()));
    let _ = m.insert("source".into(), source.into());
    let _ = m.insert("company".into(), company.trim().into());
    let _ = m.insert("title".into(), title.trim().into());
    let _ = m.insert("location".into(), location.trim().into());
    for (k, v) in [
        ("url", url),
        ("salary", salary),
        ("ats", ats),
        ("posted", posted),
        ("raw_id", raw_id),
    ] {
        let _ = m.insert(k.into(), v.map_or(Value::Null, |x| x.into()));
    }
    let _ = m.insert(
        "age_days".into(),
        age_days.map_or(Value::Null, |x| x.into()),
    );
    let mut result = Value::Object(m);
    if let (Some(extra), Some(out)) = (extra.and_then(Value::as_object), result.as_object_mut()) {
        for (key, value) in extra {
            let _ = out.insert(key.clone(), value.clone());
        }
    }
    result
}
