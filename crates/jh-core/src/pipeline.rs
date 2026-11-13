use crate::{
    filters::{is_level_ii, location_matches, tier_lookup, title_matches},
    parsing::parse_salary,
};
use serde_json::Value;
use serde_yaml::Value as Yaml;
use std::collections::BTreeMap;

fn field<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}
fn truth(p: &Value, k: &str) -> bool {
    p.get(k)
        .is_some_and(|v| !v.is_null() && v != false && v != "" && v != 0)
}
pub fn score(p: &Value, _filters: &Yaml) -> i64 {
    let mut s = 0;
    let title = field(p, "title").unwrap_or("").to_lowercase();
    if [
        "new grad",
        "graduate",
        "entry level",
        "junior",
        "jr.",
        "i ",
        " 1",
    ]
    .iter()
    .any(|x| title.contains(x))
    {
        s += 25
    }
    if truth(p, "ats") {
        s += 15
    }
    if field(p, "source") == Some("jobbank") {
        s += 8
    }
    match field(p, "location_kind") {
        Some("local") => s += 20,
        Some("local_bc") => s += 8,
        Some("remote_ca") => s += 10,
        Some("remote_unspecified") => s += 4,
        _ => (),
    }
    if let Some(age) = p.get("age_days").and_then(Value::as_i64) {
        if age <= 3 {
            s += 20
        } else if age <= 7 {
            s += 12
        } else if age <= 14 {
            s += 6
        } else if age > 30 {
            s -= 10
        }
    }
    if truth(p, "salary") {
        s += 5
    }
    if truth(p, "below_comp_floor") {
        s -= 6
    }
    if truth(p, "url") {
        s += 5
    }
    s
}
pub fn sort_key(p: &Value) -> (i64, i64, i64) {
    (
        p.get("tier_rank").and_then(Value::as_i64).unwrap_or(99),
        -p.get("score").and_then(Value::as_i64).unwrap_or(0),
        p.get("age_days").and_then(Value::as_i64).unwrap_or(999),
    )
}
pub fn cap_per_tier(posts: Vec<Value>, cap: i64) -> (Vec<Value>, usize) {
    if cap <= 0 {
        return (posts, 0);
    }
    let mut counts = BTreeMap::<String, i64>::new();
    let mut kept = Vec::new();
    for p in posts {
        let key = field(&p, "tier").unwrap_or("untiered").to_owned();
        let n = counts.entry(key).or_default();
        *n += 1;
        if *n <= cap {
            kept.push(p)
        }
    }
    let dropped = counts.values().sum::<i64>() as usize - kept.len();
    (kept, dropped)
}
pub fn tier_counts(posts: &[Value], filters: &Yaml) -> BTreeMap<String, usize> {
    let mut c = BTreeMap::new();
    for t in filters
        .get("location_tiers")
        .and_then(Yaml::as_sequence)
        .into_iter()
        .flatten()
    {
        if let Some(n) = t.get("name").and_then(Yaml::as_str) {
            let _ = c.insert(n.to_owned(), 0);
        }
    }
    let _ = c.insert("untiered".into(), 0);
    for p in posts {
        let k = field(p, "tier").unwrap_or("untiered").to_owned();
        *c.entry(k).or_default() += 1
    }
    c
}
pub fn normalize_and_filter(posts: Vec<Value>, f: &Yaml) -> (Vec<Value>, BTreeMap<String, usize>) {
    let tiers = tier_lookup(f);
    let max_age = f.get("max_age_days").and_then(Yaml::as_i64);
    let floor = f.get("comp_floor_cad").and_then(Yaml::as_i64);
    let excludes: Vec<String> = f
        .get("company_exclude")
        .and_then(Yaml::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(Yaml::as_str)
        .map(str::to_lowercase)
        .collect();
    let mut kept = Vec::new();
    let mut dropped = BTreeMap::new();
    for mut p in posts {
        let title = field(&p, "title").unwrap_or("").to_owned();
        if title.is_empty() {
            continue;
        }
        let company = field(&p, "company").unwrap_or("").to_lowercase();
        if excludes.iter().any(|x| company.contains(x)) {
            *dropped.entry("company-excluded".into()).or_default() += 1;
            continue;
        }
        let (ok, why) = title_matches(&title, f);
        if !ok {
            *dropped.entry(why).or_default() += 1;
            continue;
        }
        let (ok, kind, tier) = location_matches(field(&p, "location"), f);
        if !ok {
            *dropped.entry(kind).or_default() += 1;
            continue;
        }
        let mut m = p.as_object().cloned().unwrap_or_default();
        let _ = m.insert("location_kind".into(), kind.into());
        let rank = tiers
            .iter()
            .find(|x| Some(x.0.as_str()) == tier.as_deref())
            .map(|x| x.1 as i64)
            .unwrap_or(tiers.len() as i64);
        let country = tiers
            .iter()
            .find(|x| Some(x.0.as_str()) == tier.as_deref())
            .map(|x| x.2.as_str())
            .unwrap_or("");
        let _ = m.insert(
            "tier".into(),
            tier.clone().map_or(Value::Null, Value::String),
        );
        let _ = m.insert("tier_rank".into(), rank.into());
        if let Some(t) = tiers.iter().find(|x| Some(x.0.as_str()) == tier.as_deref()) {
            let _ = m.insert("tier_label".into(), Value::String(t.3.clone()));
        } else {
            let _ = m.insert("tier_label".into(), Value::Null);
        }
        if country == "usa" {
            let _ = m.insert("requires_us_work_auth".into(), true.into());
        }
        let age = p.get("age_days").and_then(Value::as_i64);
        if max_age.is_some_and(|x| x != 0) && age.is_some_and(|x| x > max_age.unwrap()) {
            *dropped.entry("too-old".into()).or_default() += 1;
            continue;
        }
        if let Some((lo, hi, est)) = parse_salary(field(&p, "salary")) {
            let _ = m.insert(
                "salary_annual_min".into(),
                (lo.round_ties_even() as i64).into(),
            );
            let _ = m.insert(
                "salary_annual_max".into(),
                (hi.round_ties_even() as i64).into(),
            );
            let _ = m.insert("salary_estimated".into(), est.into());
            if floor.is_some_and(|x| x != 0) && hi < floor.unwrap() as f64 {
                let _ = m.insert("below_comp_floor".into(), true.into());
            }
        }
        p = Value::Object(m);
        let sc = score(&p, f);
        let _ = p.as_object_mut().unwrap().insert("score".into(), sc.into());
        if is_level_ii(&title, f) {
            let _ = p
                .as_object_mut()
                .unwrap()
                .insert("level_ii".into(), true.into());
            let min = f
                .get("level_ii")
                .and_then(|x| x.get("min_score"))
                .and_then(Yaml::as_i64)
                .unwrap_or(0);
            if sc < min {
                *dropped.entry("level-ii-weak-fit".into()).or_default() += 1;
                continue;
            }
        }
        kept.push(p)
    }
    (kept, dropped)
}
